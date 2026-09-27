//! USER-PATH repair: pure planning + Windows-only application.
//!
//! Guarantees implemented in code (not by convention):
//! - only the persisted USER PATH is ever written; machine PATH is read-only;
//! - directories are added, never removed or reordered;
//! - canonicalised, case-insensitive (on Windows) duplicate suppression;
//! - the write is verified by re-reading the registry;
//! - planning is a pure function, tested without touching the real registry.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PathFixPlan {
    pub directories_to_add: Vec<PathBuf>,
    pub already_present: Vec<PathBuf>,
    pub unchanged_entries: usize,
    pub notes: Vec<String>,
}

impl PathFixPlan {
    pub fn is_noop(&self) -> bool {
        self.directories_to_add.is_empty()
    }
}

fn norm(p: &Path) -> String {
    let c = crate::registry::canon(p);
    let s = c.to_string_lossy().into_owned();
    if cfg!(windows) {
        s.to_ascii_lowercase()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_string()
    } else {
        s.trim_end_matches('/').to_string()
    }
}

/// Plan a USER PATH repair. Pure: takes the persisted user PATH entries and
/// the directories containing hidden tools; returns what would change.
pub fn plan_path_fix(user_path: &[PathBuf], hidden_dirs: &[PathBuf]) -> PathFixPlan {
    let existing: Vec<String> = user_path.iter().map(|p| norm(p)).collect();
    let mut to_add: Vec<PathBuf> = Vec::new();
    let mut already_present: Vec<PathBuf> = Vec::new();
    let mut notes = Vec::new();

    for dir in hidden_dirs {
        if !dir.is_absolute() {
            notes.push(format!("skipped non-absolute directory: {}", dir.display()));
            continue;
        }
        let n = norm(dir);
        if existing.contains(&n) || to_add.iter().any(|d| norm(d) == n) {
            already_present.push(dir.clone());
        } else {
            to_add.push(dir.clone());
        }
    }

    if !already_present.is_empty() && to_add.is_empty() {
        notes.push(
            "all required directories are already on the persisted user PATH; \
             this process inherited an older PATH — restart the shell/agent"
                .into(),
        );
    }

    PathFixPlan {
        directories_to_add: to_add,
        already_present,
        unchanged_entries: user_path.len(),
        notes,
    }
}

/// Compose the new USER PATH string: existing entries preserved verbatim and
/// in order; new directories appended.
pub fn compose_user_path(current_raw: &str, to_add: &[PathBuf]) -> String {
    let base = current_raw.trim_end_matches(';');
    let mut parts: Vec<String> = Vec::new();
    if !base.is_empty() {
        parts.push(base.to_string());
    }
    for d in to_add {
        parts.push(d.to_string_lossy().into_owned());
    }
    parts.join(";")
}

/// Apply a plan to the persisted USER PATH (Windows registry).
/// Returns (old_value, new_value) on success.
#[cfg(windows)]
pub fn apply_user_path_fix(new_value: &str) -> Result<(String, String), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ};
    use winreg::types::ToRegValue;
    use winreg::{RegKey, RegValue};

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|e| format!("cannot open HKCU\\Environment: {e}"))?;

    // Preserve the existing registry value type (REG_EXPAND_SZ vs REG_SZ).
    let (raw, ty) = key
        .get_raw_value("Path")
        .map(|rv| (rv.bytes, rv.vtype))
        .unwrap_or_else(|_| (Vec::new(), REG_SZ));
    let old = String::from_utf16_lossy(
        &raw.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    )
    .trim_end_matches('\0')
    .to_string();

    let written = RegValue {
        bytes: new_value.to_reg_value().bytes,
        vtype: ty,
    };
    key.set_raw_value("Path", &written)
        .map_err(|e| format!("cannot write HKCU\\Environment Path: {e}"))?;

    // Verify by re-reading the RAW value (expansion would alter %entries%).
    let check = key
        .get_raw_value("Path")
        .map_err(|e| format!("cannot re-read HKCU\\Environment Path: {e}"))?;
    if check.bytes != written.bytes {
        return Err("verification failed: persisted value does not match planned value".into());
    }
    Ok((old, new_value.to_string()))
}

#[cfg(not(windows))]
pub fn apply_user_path_fix(_new_value: &str) -> Result<(String, String), String> {
    Err("PATH repair is Windows-only in v0.1".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn plan_adds_missing_dirs_only() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        let plan = plan_path_fix(std::slice::from_ref(&a), &[a.clone(), b.clone()]);
        assert_eq!(plan.directories_to_add, vec![b.clone()]);
        assert_eq!(plan.already_present, vec![a.clone()]);
    }

    #[test]
    fn plan_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        std::fs::create_dir_all(&a).unwrap();

        let plan1 = plan_path_fix(&[], std::slice::from_ref(&a));
        assert_eq!(plan1.directories_to_add.len(), 1);

        let user_path_after = vec![a.clone()];
        let plan2 = plan_path_fix(&user_path_after, std::slice::from_ref(&a));
        assert!(plan2.is_noop());
        assert!(!plan2.notes.is_empty());
    }

    #[test]
    fn plan_dedupes_repeated_hidden_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("dup");
        std::fs::create_dir_all(&a).unwrap();
        let plan = plan_path_fix(&[], &[a.clone(), a.clone(), a.clone()]);
        assert_eq!(plan.directories_to_add.len(), 1);
    }

    #[test]
    fn plan_rejects_relative_dirs() {
        let plan = plan_path_fix(&[], &[pb("relative/dir")]);
        assert!(plan.is_noop());
        assert!(plan.notes.iter().any(|n| n.contains("non-absolute")));
    }

    #[test]
    fn compose_preserves_existing_entries() {
        let out = compose_user_path(r"C:\keep;C:\also", &[pb(r"C:\new")]);
        assert_eq!(out, r"C:\keep;C:\also;C:\new");
        let out = compose_user_path(r"C:\keep;", &[pb(r"C:\new")]);
        assert_eq!(out, r"C:\keep;C:\new");
        let out = compose_user_path("", &[pb(r"C:\new")]);
        assert_eq!(out, r"C:\new");
    }
}
