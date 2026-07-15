//! gamedata.sqlite — normalized game data keyed by game build (SDD §7).
//! Lives in app-data; re-parsed when the install's build changes.

use rusqlite::Connection;

use crate::docs::{Belt, Buildable, GameData, Item, Machine, MachineKind, Recipe};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Cache schema version, stored in meta('schema_version') and required by
/// `build_matches` alongside the game build. Bump whenever the persisted shape
/// changes meaning. Two hazards make this key load-bearing even while the
/// cache is write-only:
/// 1. serde-default fields (e.g. `Item.is_resource`) silently deserialize as
///    `false` from pre-versioned blobs — a wired read path would resurrect the
///    packaging-cycle hazard the raw-resource gate exists to prevent;
/// 2. schematics and milestones are NOT persisted at all (no table) — a wired
///    read would silently drop unlocked-alternate resolution and the
///    milestone_gap family's cost/tier data.
///
/// Absence of the key = stale, which covers every pre-existing cache.
/// v3: `Machine.footprint_m` (serde-default None) joined the persisted shape —
/// pre-v3 blobs would silently read as "no clearance data".
/// v4: footprint parsing now applies each box's `RelativeTransform` and
/// unions CT_Hard boxes only — v3 blobs hold wrong values for every
/// transform-bearing class (Manufacturer 18×13 vs true 18×20, Particle
/// Accelerator 52×22 garbage vs 37×27) and must re-parse.
const SCHEMA_VERSION: &str = "4";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS items (class TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS recipes (class TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS machines (class TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS belts (class TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS buildables (class TEXT PRIMARY KEY, json TEXT NOT NULL);
";

pub fn write(conn: &Connection, gd: &GameData) -> Result<(), DbError> {
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch("BEGIN")?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('game_build', ?1)",
        [&gd.build_version],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
        [SCHEMA_VERSION],
    )?;
    let clear = |table: &str| conn.execute_batch(&format!("DELETE FROM {table}"));
    clear("items")?;
    clear("recipes")?;
    clear("machines")?;
    clear("belts")?;
    clear("buildables")?;
    for (class, item) in &gd.items {
        conn.execute(
            "INSERT INTO items (class, json) VALUES (?1, ?2)",
            (class, serde_json::to_string(item)?),
        )?;
    }
    for (class, r) in &gd.recipes {
        conn.execute(
            "INSERT INTO recipes (class, json) VALUES (?1, ?2)",
            (class, serde_json::to_string(r)?),
        )?;
    }
    for (class, m) in &gd.machines {
        conn.execute(
            "INSERT INTO machines (class, json) VALUES (?1, ?2)",
            (class, serde_json::to_string(m)?),
        )?;
    }
    for (class, b) in &gd.belts {
        conn.execute(
            "INSERT INTO belts (class, json) VALUES (?1, ?2)",
            (class, serde_json::to_string(b)?),
        )?;
    }
    for (class, b) in &gd.buildables {
        conn.execute(
            "INSERT INTO buildables (class, json) VALUES (?1, ?2)",
            (class, serde_json::to_string(b)?),
        )?;
    }
    conn.execute_batch("COMMIT")?;
    Ok(())
}

pub fn read(conn: &Connection) -> Result<GameData, DbError> {
    let build_version: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'game_build'", [], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    let mut gd = GameData {
        build_version,
        ..Default::default()
    };
    let load = |table: &str| -> Result<Vec<(String, String)>, DbError> {
        let mut stmt = conn.prepare(&format!("SELECT class, json FROM {table}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    };
    for (class, json) in load("items")? {
        gd.items.insert(class, serde_json::from_str::<Item>(&json)?);
    }
    for (class, json) in load("recipes")? {
        gd.recipes
            .insert(class, serde_json::from_str::<Recipe>(&json)?);
    }
    for (class, json) in load("machines")? {
        gd.machines
            .insert(class, serde_json::from_str::<Machine>(&json)?);
    }
    for (class, json) in load("belts")? {
        gd.belts.insert(class, serde_json::from_str::<Belt>(&json)?);
    }
    for (class, json) in load("buildables")? {
        gd.buildables
            .insert(class, serde_json::from_str::<Buildable>(&json)?);
    }
    Ok(gd)
}

/// True only when the stored game build matches the install's AND the stored
/// cache schema version matches `SCHEMA_VERSION` (re-parse trigger otherwise).
/// A missing `schema_version` key counts as stale — see `SCHEMA_VERSION` for
/// why a build match alone is not enough.
pub fn build_matches(conn: &Connection, build: &str) -> bool {
    let meta = |key: &str| {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
    };
    let build_ok = meta("game_build").map(|s| s == build).unwrap_or(false);
    let schema_ok = meta("schema_version")
        .map(|s| s == SCHEMA_VERSION)
        .unwrap_or(false);
    build_ok && schema_ok
}

/// Machine power lookup with a sensible default for unknown classes.
pub fn machine_power(gd: &GameData, class: &str) -> f64 {
    gd.machines.get(class).map(|m| m.power_mw).unwrap_or(0.0)
}

/// Planning draw for one machine running `recipe`: the recipe's variable-power
/// average when present (Particle Accelerator etc. — draw varies by recipe),
/// otherwise the machine's fixed draw.
pub fn recipe_power(gd: &GameData, recipe: &Recipe, machine: &str) -> f64 {
    recipe
        .variable_power_mw
        .unwrap_or_else(|| machine_power(gd, machine))
}

/// Manufacturer classes a recipe can run in (first automated machine wins for Phase 1).
pub fn manufacturer_for(gd: &GameData, recipe: &Recipe) -> Option<String> {
    recipe
        .produced_in
        .iter()
        .find(|c| {
            gd.machines
                .get(*c)
                .map(|m| matches!(m.kind, MachineKind::Manufacturer))
                .unwrap_or(false)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::parse_docs;

    #[test]
    fn roundtrip_through_sqlite() {
        let gd = parse_docs(include_str!("../assets/docs-fixture.json"), "463028").unwrap();
        let conn = Connection::open_in_memory().unwrap();
        write(&conn, &gd).unwrap();
        let back = read(&conn).unwrap();
        assert_eq!(gd, back);
        assert!(build_matches(&conn, "463028"));
        assert!(!build_matches(&conn, "999999"));
        let mf = &back.recipes["Recipe_ModularFrame_C"];
        assert_eq!(
            manufacturer_for(&back, mf).as_deref(),
            Some("Build_AssemblerMk1_C")
        );
        // variable_power_mw survives the roundtrip and recipe_power prefers it
        assert_eq!(
            recipe_power(&back, mf, "Build_AssemblerMk1_C"),
            15.0,
            "fixed-power recipes fall back to the machine draw"
        );
        let diamond = &back.recipes["Recipe_Diamond_C"];
        assert_eq!(diamond.variable_power_mw, Some(500.0));
        assert_eq!(
            recipe_power(&back, diamond, "Build_HadronCollider_C"),
            500.0
        );
        let dark = &back.recipes["Recipe_DarkMatter_C"];
        assert_eq!(
            recipe_power(&back, dark, "Build_HadronCollider_C"),
            1000.0,
            "recipe average beats the machine estimate"
        );
        // Schema-version key: a matching build with a stale (or missing)
        // schema_version must read as a cache miss. The literal is '3' — the
        // immediate predecessor — so a "4"→"3" SCHEMA_VERSION revert (which
        // would resurrect caches holding pre-transform footprints) cannot
        // satisfy this assert.
        conn.execute(
            "UPDATE meta SET value = '3' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
        assert!(
            !build_matches(&conn, "463028"),
            "old schema version must invalidate the cache even on a build match"
        );
        conn.execute("DELETE FROM meta WHERE key = 'schema_version'", [])
            .unwrap();
        assert!(
            !build_matches(&conn, "463028"),
            "absent schema version (pre-existing cache) must read as stale"
        );
    }
}
