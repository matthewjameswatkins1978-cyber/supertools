//! Environment fact collection for the Tool Registry.
//!
//! Staged discovery (no full-disk crawling):
//!   1. process PATH
//!   2. persisted user PATH / machine PATH (Windows registry)
//!   3. exact known developer locations (cargo bin, winget links, scoop
//!      shims, chocolatey bin, npm global, ~/.local/bin)
//!   4. narrowly bounded directory scans (depth-limited, entry-capped)
//!   5. package-manager metadata (deep mode only; handled in install.rs)

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{DiscoverySource, ToolCandidate, ToolId, ALL_TOOLS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
    Other,
}

pub fn current_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Other
    }
}

#[derive(Debug, Clone)]
pub struct KnownDir {
    pub path: PathBuf,
    pub source: DiscoverySource,
}

/// All environment facts discovery needs, collected once. Tests construct
/// synthetic `EnvFacts` instead of touching the real machine.
#[derive(Debug, Clone)]
pub struct EnvFacts {
    pub platform: Platform,
    pub deep: bool,
    pub process_path: Vec<PathBuf>,
    pub user_path: Vec<PathBuf>,
    pub machine_path: Vec<PathBuf>,
    pub known_dirs: Vec<KnownDir>,
    /// Roots for bounded depth-limited scans (deep mode, Windows-first).
    pub scan_roots: Vec<(PathBuf, DiscoverySource, usize)>,
    /// Executable extensions, e.g. [".exe", ".bat"] on Windows; [""] elsewhere.
    pub pathext: Vec<String>,
}

impl EnvFacts {
    /// File names that would resolve `name` as an executable here.
    pub fn exec_variants(&self, name: &str) -> Vec<String> {
        self.pathext
            .iter()
            .map(|ext| format!("{name}{ext}"))
            .collect()
    }

    pub fn collect(deep: bool) -> EnvFacts {
        let platform = current_platform();
        let process_path = split_path_env(&std::env::var_os("PATH").unwrap_or_default());
        let pathext = collect_pathext();
        let home = home_dir();

        #[cfg(windows)]
        let (user_path, machine_path) = windows_persisted_paths();
        #[cfg(not(windows))]
        let (user_path, machine_path): (Vec<PathBuf>, Vec<PathBuf>) = (Vec::new(), Vec::new());

        let known_dirs = known_dirs(&home, platform);
        let scan_roots = if deep {
            scan_roots(&home, platform)
        } else {
            Vec::new()
        };

        EnvFacts {
            platform,
            deep,
            process_path,
            user_path,
            machine_path,
            known_dirs,
            scan_roots,
            pathext,
        }
    }
}

pub fn split_path_env(raw: &std::ffi::OsStr) -> Vec<PathBuf> {
    std::env::split_paths(raw)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

fn collect_pathext() -> Vec<String> {
    if cfg!(windows) {
        let mut exts = vec![".exe".to_string()];
        if let Some(raw) = std::env::var_os("PATHEXT") {
            for e in std::env::split_paths(&raw) {
                let s = e.to_string_lossy().to_ascii_lowercase();
                if !s.is_empty() && !exts.contains(&s) {
                    exts.push(s);
                }
            }
        }
        // Only extensions that can carry a real executable identity matter
        // for discovery; keep the full list for PATH fidelity.
        exts
    } else {
        vec![String::new()]
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn known_dirs(home: &Option<PathBuf>, _platform: Platform) -> Vec<KnownDir> {
    let mut dirs: Vec<KnownDir> = Vec::new();
    let mut push = |p: Option<PathBuf>, source: DiscoverySource| {
        if let Some(p) = p {
            dirs.push(KnownDir { path: p, source });
        }
    };

    // Cargo bin.
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from);
    push(
        cargo_home
            .clone()
            .map(|c| c.join("bin"))
            .or_else(|| home.as_ref().map(|h| h.join(".cargo").join("bin"))),
        DiscoverySource::CargoBin,
    );
    // ~/.local/bin
    push(
        home.as_ref().map(|h| h.join(".local").join("bin")),
        DiscoverySource::UserLocalBin,
    );

    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        let pf = std::env::var_os("ProgramFiles").map(PathBuf::from);
        push(
            local
                .as_ref()
                .map(|l| l.join("Microsoft").join("WinGet").join("Links")),
            DiscoverySource::WinGetLinksUser,
        );
        push(
            pf.as_ref().map(|p| p.join("WinGet").join("Links")),
            DiscoverySource::WinGetLinksMachine,
        );
        push(
            local
                .as_ref()
                .map(|l| l.join("Microsoft").join("WindowsApps")),
            DiscoverySource::WindowsAppsAlias,
        );
        let scoop = std::env::var_os("SCOOP")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join("scoop")));
        push(scoop.map(|s| s.join("shims")), DiscoverySource::ScoopShims);
        push(
            std::env::var_os("ProgramData")
                .map(|p| PathBuf::from(p).join("chocolatey").join("bin")),
            DiscoverySource::ChocoBin,
        );
        push(
            appdata.as_ref().map(|a| a.join("npm")),
            DiscoverySource::NpmGlobal,
        );
        push(
            local.as_ref().map(|l| l.join("Programs")),
            DiscoverySource::LocalAppDataPrograms,
        );
    }

    dirs
}

fn scan_roots(
    home: &Option<PathBuf>,
    _platform: Platform,
) -> Vec<(PathBuf, DiscoverySource, usize)> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            roots.push((PathBuf::from(pf), DiscoverySource::ProgramFiles, 3));
        }
        if let Some(pf86) = std::env::var_os("ProgramFiles(x86)") {
            roots.push((PathBuf::from(pf86), DiscoverySource::ProgramFilesX86, 2));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let l = PathBuf::from(local);
            roots.push((
                l.join("Microsoft").join("WinGet").join("Packages"),
                DiscoverySource::LocalAppDataScan,
                2,
            ));
            roots.push((l.clone(), DiscoverySource::LocalAppDataScan, 1));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            roots.push((PathBuf::from(appdata), DiscoverySource::AppDataScan, 1));
        }
    } else if let Some(h) = home {
        roots.push((
            h.join(".local").join("share"),
            DiscoverySource::AppDataScan,
            2,
        ));
    }
    roots
}

const SCAN_ENTRY_BUDGET: u32 = 40_000;

/// Bounded depth-limited scan of known roots. Matches directory entries by
/// name against the recommended toolset's executable names; never recurses
/// beyond the stated depth and stops after a global entry budget.
pub fn bounded_scan(facts: &EnvFacts) -> Vec<(ToolId, ToolCandidate)> {
    let mut wanted: HashSet<String> = HashSet::new();
    let mut which_tool: Vec<(String, ToolId, bool)> = Vec::new(); // (filename, tool, is_alias)
    for id in ALL_TOOLS {
        for name in id.exec_names() {
            for variant in facts.exec_variants(name) {
                let lower = variant.to_ascii_lowercase();
                if wanted.insert(lower.clone()) {
                    which_tool.push((lower, id, *name != id.primary_exec()));
                }
            }
        }
    }

    let mut hits: Vec<(ToolId, ToolCandidate)> = Vec::new();
    let mut budget = SCAN_ENTRY_BUDGET;

    for (root, source, depth) in &facts.scan_roots {
        if budget == 0 {
            break;
        }
        if !root.is_dir() {
            continue;
        }
        scan_dir(
            root,
            *source,
            *depth,
            &wanted,
            &which_tool,
            &mut hits,
            &mut budget,
        );
    }
    hits
}

#[allow(clippy::too_many_arguments)]
fn scan_dir(
    dir: &Path,
    source: DiscoverySource,
    depth: usize,
    wanted: &HashSet<String>,
    which_tool: &[(String, ToolId, bool)],
    hits: &mut Vec<(ToolId, ToolCandidate)>,
    budget: &mut u32,
) {
    if depth == 0 || *budget == 0 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if ft.is_file() && wanted.contains(&name) {
            let path = entry.path();
            let Some((_, id, is_alias)) = which_tool.iter().find(|(n, _, _)| *n == name) else {
                continue;
            };
            // Alias executables (e.g. `sg` for ast-grep) are only accepted
            // from trusted sources where identity can be confirmed by running
            // `--version`. Raw-scan alias hits are skipped: identity cannot
            // be established without executing an untrusted binary.
            if !*is_alias {
                hits.push((
                    *id,
                    ToolCandidate {
                        path,
                        source,
                        version: None,
                        trusted: source.trusted(),
                    },
                ));
            }
        } else if ft.is_dir() {
            // Skip obvious non-application junk to stay bounded.
            if matches!(
                name.as_str(),
                "temp" | "tmp" | "cache" | "caches" | "crashdumps" | "packages"
            ) && depth <= 2
            {
                continue;
            }
            scan_dir(
                &entry.path(),
                source,
                depth - 1,
                wanted,
                which_tool,
                hits,
                budget,
            );
        }
    }
}

#[cfg(windows)]
fn windows_persisted_paths() -> (Vec<PathBuf>, Vec<PathBuf>) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::{RegKey, HKEY};

    let read = |root: HKEY, sub: &str| -> Vec<PathBuf> {
        let Ok(key) = RegKey::predef(root).open_subkey_with_flags(sub, winreg::enums::KEY_READ)
        else {
            return Vec::new();
        };
        let Ok(raw) = key.get_value::<String, _>("Path") else {
            return Vec::new();
        };
        split_path_env(std::ffi::OsString::from(raw).as_os_str())
    };

    let user = read(HKEY_CURRENT_USER, "Environment");
    let machine = read(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    (user, machine)
}

#[cfg(windows)]
pub fn windows_user_path_string() -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ)
        .ok()?;
    key.get_value::<String, _>("Path").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_env_drops_empty_entries() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let raw = format!("/a{sep}{sep}/b{sep}");
        let dirs = split_path_env(std::ffi::OsString::from(raw).as_os_str());
        assert_eq!(dirs.len(), 2);
    }

    #[test]
    fn exec_variants_use_pathext() {
        let mut facts = EnvFacts {
            platform: Platform::Windows,
            deep: false,
            process_path: vec![],
            user_path: vec![],
            machine_path: vec![],
            known_dirs: vec![],
            scan_roots: vec![],
            pathext: vec![".exe".into(), ".cmd".into()],
        };
        assert_eq!(facts.exec_variants("rg"), vec!["rg.exe", "rg.cmd"]);
        facts.pathext = vec![String::new()];
        assert_eq!(facts.exec_variants("rg"), vec!["rg"]);
    }

    #[test]
    fn bounded_scan_finds_exe_in_temp_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("SomeApp").join("bin");
        std::fs::create_dir_all(&deep).unwrap();
        let marker = deep.join("threadmoth.exe");
        std::fs::write(&marker, b"not a real exe").unwrap();

        let facts = EnvFacts {
            platform: Platform::Windows,
            deep: true,
            process_path: vec![],
            user_path: vec![],
            machine_path: vec![],
            known_dirs: vec![],
            scan_roots: vec![(
                tmp.path().to_path_buf(),
                DiscoverySource::LocalAppDataScan,
                3,
            )],
            pathext: vec![".exe".into()],
        };
        let hits = bounded_scan(&facts);
        assert!(hits
            .iter()
            .any(|(id, c)| *id == ToolId::Threadmoth && c.path == marker));
        assert!(hits.iter().all(|(_, c)| !c.trusted));
    }

    #[test]
    fn bounded_scan_respects_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("just.exe"), b"x").unwrap();
        let facts = EnvFacts {
            platform: Platform::Windows,
            deep: true,
            process_path: vec![],
            user_path: vec![],
            machine_path: vec![],
            known_dirs: vec![],
            scan_roots: vec![(tmp.path().to_path_buf(), DiscoverySource::ProgramFiles, 2)],
            pathext: vec![".exe".into()],
        };
        assert!(bounded_scan(&facts).is_empty());
    }
}
