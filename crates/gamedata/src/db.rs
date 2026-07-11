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

/// True when the stored build differs from the install's (re-parse trigger).
pub fn build_matches(conn: &Connection, build: &str) -> bool {
    conn.query_row("SELECT value FROM meta WHERE key = 'game_build'", [], |r| {
        r.get::<_, String>(0)
    })
    .map(|stored| stored == build)
    .unwrap_or(false)
}

/// Machine power lookup with a sensible default for unknown classes.
pub fn machine_power(gd: &GameData, class: &str) -> f64 {
    gd.machines.get(class).map(|m| m.power_mw).unwrap_or(0.0)
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
    }
}
