//! Multi-empire registry (1.0): several named empires, each its own plan file
//! in ONE plans directory, with a switcher in the DATA menu. The registry IS
//! the directory listing — an empire named `OUTPOST` is `OUTPOST.ficsit` on
//! disk — so there is no side-car index to drift, and files dropped into the
//! directory by hand appear in the switcher. The web build mirrors this shape
//! over IndexedDB keys in the worker (no Rust involvement: a `WebSession` is
//! constructed from whichever blob the worker hands it).
//!
//! Names are file stems: sanitization rejects path separators and dot-leading
//! names instead of silently rewriting (the name shown IS the name stored).
//! Rename/delete move the SQLite side-cars (`-wal`/`-shm`/`.bak`) along with
//! the plan so a later open never resurrects stale journal state.

use std::path::{Path, PathBuf};

/// Listing + active marker, serialized to the renderer as-is.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmpireList {
    pub active: String,
    pub names: Vec<String>,
}

/// A usable empire name: non-empty, ≤ 64 chars, no path separators / NUL, not
/// dot-leading (hidden files), no `.ficsit` suffix games. Returns the trimmed
/// name (the ONLY normalization applied).
pub fn sanitize_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("empire name is empty".into());
    }
    if name.len() > 64 {
        return Err("empire name is longer than 64 characters".into());
    }
    if name.starts_with('.') {
        return Err("empire name can't start with a dot".into());
    }
    if name
        .chars()
        .any(|c| c == '/' || c == '\\' || c == '\0' || c.is_control())
    {
        return Err("empire name can't contain path separators".into());
    }
    // Windows-reserved filename characters — the desktop shell ships there, and
    // letting them through would surface as an opaque SQLite create error
    // instead of a reason. (Trailing dots/spaces are covered by trim + the
    // dot-suffix check below being about extensions only; a trailing dot is
    // rejected here explicitly.)
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("empire name can't contain < > : \" | ? *".into());
    }
    if name.ends_with('.') {
        return Err("empire name can't end with a dot".into());
    }
    if name.to_ascii_lowercase().ends_with(".ficsit") {
        return Err("leave off the .ficsit extension".into());
    }
    Ok(name.to_string())
}

/// The plan file for `name` inside `dir`.
pub fn empire_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.ficsit"))
}

/// SQLite side-cars that must travel with (or die with) a plan file.
fn sidecars(path: &Path) -> Vec<PathBuf> {
    let s = path.to_string_lossy();
    vec![
        PathBuf::from(format!("{s}-wal")),
        PathBuf::from(format!("{s}-shm")),
        PathBuf::from(format!("{s}.bak")),
    ]
}

/// Every empire in `dir` (file stems of `*.ficsit`, sorted), with the active
/// one named from `active_path`. The active empire is always listed even if
/// its file hasn't been flushed yet.
pub fn list(dir: &Path, active_path: &Path) -> EmpireList {
    let active = active_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "world".into());
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("ficsit"))
                .then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .flatten()
        })
        .collect();
    if !names.iter().any(|n| n == &active) {
        names.push(active.clone());
    }
    names.sort();
    names.dedup();
    EmpireList { active, names }
}

/// Guard for create: the target must not exist yet.
pub fn ensure_absent(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = empire_path(dir, name);
    if p.exists() {
        return Err(format!("an empire named {name} already exists"));
    }
    Ok(p)
}

/// Guard for switch/rename/delete sources: the target must exist.
pub fn ensure_present(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = empire_path(dir, name);
    if !p.exists() {
        return Err(format!("no empire named {name}"));
    }
    Ok(p)
}

/// Move a plan file (and side-cars, best-effort) to a new name. The caller is
/// responsible for having CLOSED any session holding the source open.
pub fn rename_files(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| format!("rename failed: {e}"))?;
    for (a, b) in sidecars(from).into_iter().zip(sidecars(to)) {
        if a.exists() {
            let _ = std::fs::rename(a, b);
        }
    }
    Ok(())
}

/// Delete a plan file and its side-cars.
pub fn delete_files(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| format!("delete failed: {e}"))?;
    for s in sidecars(path) {
        let _ = std::fs::remove_file(s);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_hostile_names() {
        assert!(sanitize_name("  OUTPOST 7  ").unwrap() == "OUTPOST 7");
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("   ").is_err());
        assert!(sanitize_name("a/b").is_err());
        assert!(sanitize_name("a\\b").is_err());
        assert!(sanitize_name(".hidden").is_err());
        assert!(sanitize_name("x.ficsit").is_err());
        assert!(sanitize_name(&"x".repeat(65)).is_err());
        // Windows-reserved characters + trailing dot are refused with a reason
        assert!(sanitize_name("OUT:POST").is_err());
        assert!(sanitize_name("what?").is_err());
        assert!(sanitize_name("star*").is_err());
        assert!(sanitize_name("dot.").is_err());
    }

    #[test]
    fn list_create_rename_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let a = empire_path(dir.path(), "ALPHA");
        std::fs::write(&a, b"x").unwrap();

        let l = list(dir.path(), &a);
        assert_eq!(l.active, "ALPHA");
        assert_eq!(l.names, vec!["ALPHA"]);

        // the active plan is listed even before its file exists
        let ghost = empire_path(dir.path(), "GHOST");
        let l = list(dir.path(), &ghost);
        assert_eq!(l.active, "GHOST");
        assert!(l.names.contains(&"GHOST".to_string()));

        let b = ensure_absent(dir.path(), "BETA").unwrap();
        std::fs::write(&b, b"y").unwrap();
        assert!(ensure_absent(dir.path(), "BETA").is_err());
        assert!(ensure_present(dir.path(), "BETA").is_ok());
        assert!(ensure_present(dir.path(), "NOPE").is_err());

        let b2 = empire_path(dir.path(), "BETA 2");
        rename_files(&b, &b2).unwrap();
        assert!(!b.exists() && b2.exists());

        delete_files(&b2).unwrap();
        assert!(!b2.exists());
        let l = list(dir.path(), &a);
        assert_eq!(l.names, vec!["ALPHA"]);
    }
}
