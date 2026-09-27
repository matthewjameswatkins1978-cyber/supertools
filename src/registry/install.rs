//! Structured installation catalogue for the recommended toolset.
//!
//! ONE catalogue; no download URLs scattered through code. Each entry
//! declares official upstream distribution mechanisms in preference order
//! plus the verification method (executable resolvable and `--version`
//! identity confirmed by the registry).
//!
//! Supertools only ever invokes an existing trusted package manager with an
//! argument vector. It never downloads arbitrary binaries itself. If no
//! trustworthy route exists on this machine, it explains and declines.

use std::path::PathBuf;

use serde::Serialize;

use super::env::EnvFacts;
use super::ToolId;
use crate::process::{self, Request};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "mechanism", rename_all = "snake_case")]
pub enum InstallMechanism {
    Winget {
        package: String,
    },
    Choco {
        package: String,
    },
    Scoop {
        package: String,
    },
    Cargo {
        package: String,
    },
    Npm {
        package: String,
    },
    /// No automatable route; guidance only (never installs).
    Guidance {
        url: String,
        note: String,
    },
}

impl InstallMechanism {
    pub fn manager_program(&self) -> Option<&'static str> {
        match self {
            InstallMechanism::Winget { .. } => Some("winget"),
            InstallMechanism::Choco { .. } => Some("choco"),
            InstallMechanism::Scoop { .. } => Some("scoop"),
            InstallMechanism::Cargo { .. } => Some("cargo"),
            InstallMechanism::Npm { .. } => Some("npm"),
            InstallMechanism::Guidance { .. } => None,
        }
    }
}

/// Catalogue entry: mechanisms in preference order for Windows.
/// Ordering rationale: prefer user-scope mechanisms (winget portable,
/// cargo, scoop) over machine-wide ones (choco needs admin); Rust-native
/// tools prefer cargo, matching upstream documentation.
pub fn install_options(id: ToolId) -> Vec<InstallMechanism> {
    let g = |url: &str, note: &str| InstallMechanism::Guidance {
        url: url.to_string(),
        note: note.to_string(),
    };
    match id {
        ToolId::Git => vec![
            InstallMechanism::Winget { package: "Git.Git".into() },
            InstallMechanism::Choco { package: "git".into() },
            InstallMechanism::Scoop { package: "git".into() },
            g("https://git-scm.com/download/win", "official installer"),
        ],
        ToolId::Cargo | ToolId::Rustc => vec![
            InstallMechanism::Winget { package: "Rustlang.Rustup".into() },
            InstallMechanism::Choco { package: "rustup.install".into() },
            g("https://rustup.rs", "official rustup installer"),
        ],
        ToolId::Rg => vec![
            InstallMechanism::Winget { package: "BurntSushi.ripgrep.MSVC".into() },
            InstallMechanism::Choco { package: "ripgrep".into() },
            InstallMechanism::Scoop { package: "ripgrep".into() },
            g("https://github.com/BurntSushi/ripgrep/releases", "official releases"),
        ],
        ToolId::Fd => vec![
            InstallMechanism::Cargo { package: "fd-find".into() },
            InstallMechanism::Winget { package: "sharkdp.fd".into() },
            InstallMechanism::Choco { package: "fd".into() },
            InstallMechanism::Scoop { package: "fd".into() },
            g("https://github.com/sharkdp/fd", "official releases"),
        ],
        ToolId::Gh => vec![
            InstallMechanism::Winget { package: "GitHub.cli".into() },
            InstallMechanism::Choco { package: "github-cli".into() },
            InstallMechanism::Scoop { package: "gh".into() },
            g("https://cli.github.com", "official installer"),
        ],
        ToolId::Mise => vec![
            InstallMechanism::Winget { package: "jdx.mise".into() },
            InstallMechanism::Choco { package: "mise".into() },
            InstallMechanism::Scoop { package: "mise".into() },
            g("https://mise.jdx.dev/installing-mise.html", "official install docs"),
        ],
        ToolId::Just => vec![
            InstallMechanism::Cargo { package: "just".into() },
            InstallMechanism::Winget { package: "Casey.Just".into() },
            InstallMechanism::Choco { package: "just".into() },
            InstallMechanism::Scoop { package: "just".into() },
            g("https://github.com/casey/just", "official releases"),
        ],
        ToolId::AstGrep => vec![
            InstallMechanism::Npm { package: "@ast-grep/cli".into() },
            g("https://ast-grep.github.io/guide/quick-start.html", "official install docs"),
        ],
        ToolId::Threadmoth => vec![g(
            "https://github.com/matthewjameswatkins1978-cyber",
            "Threadmoth distribution is managed by the Threadmoth project; Supertools will not reinstall or disturb an existing Threadmoth",
        )],
    }
}

/// Guidance-only fallback text for tools without an automatable route.
pub fn guidance(id: ToolId) -> Option<&'static str> {
    match id {
        ToolId::AstGrep => Some("see https://ast-grep.github.io/guide/quick-start.html (npm @ast-grep/cli or official releases)"),
        ToolId::Threadmoth => Some("managed by the Threadmoth project; existing installation must not be disturbed"),
        _ => None,
    }
}

/// Which package managers can actually be invoked on this machine.
#[derive(Debug, Clone, Default)]
pub struct ManagerAvailability {
    pub winget: Option<String>,
    pub choco: Option<String>,
    pub scoop: Option<String>,
    pub cargo: Option<String>,
    pub npm: Option<String>,
}

pub fn probe_managers(facts: &EnvFacts) -> ManagerAvailability {
    let resolve = |p: &str| {
        crate::process::resolve_program(p)
            .ok()
            .map(|x| x.to_string_lossy().into_owned())
    };
    ManagerAvailability {
        cargo: resolve("cargo"),
        npm: resolve("npm"),
        choco: resolve("choco"),
        scoop: resolve("scoop"),
        // winget frequently lives behind the WindowsApps execution alias,
        // which may not be on this process' PATH. Check PATH first, then the
        // known alias location, then the machine Links dir.
        winget: resolve("winget")
            .or_else(|| known_dir_file(facts, super::DiscoverySource::WindowsAppsAlias, "winget"))
            .or_else(|| {
                known_dir_file(facts, super::DiscoverySource::WinGetLinksMachine, "winget")
            }),
    }
}

fn known_dir_file(facts: &EnvFacts, source: super::DiscoverySource, name: &str) -> Option<String> {
    facts
        .known_dirs
        .iter()
        .find(|k| k.source == source)
        .and_then(|k| {
            facts
                .exec_variants(name)
                .into_iter()
                .map(|v| k.path.join(v))
                .find(|p| p.is_file())
        })
        .map(|p| p.to_string_lossy().into_owned())
}

/// Discovery stage 4: inspect package-manager KNOWLEDGE (metadata only,
/// bounded, never installs anything) for tools that are still missing.
pub fn manager_metadata_notes(facts: &EnvFacts, missing: &[ToolId]) -> Vec<(ToolId, String)> {
    let mut notes = Vec::new();

    // cargo install --list: crate -> executable knowledge.
    let cargo_crates: [(ToolId, &str, &str); 2] = [
        (ToolId::Fd, "fd-find", "fd"),
        (ToolId::Just, "just", "just"),
    ];
    let cargo_relevant = missing
        .iter()
        .any(|id| *id == ToolId::Fd || *id == ToolId::Just);
    if cargo_relevant {
        if let Ok(cargo) = crate::process::resolve_program("cargo") {
            let out = crate::process::run(
                &Request::new(
                    cargo.to_string_lossy().into_owned(),
                    vec!["install".into(), "--list".into()],
                )
                .timeout(std::time::Duration::from_secs(15)),
            );
            if let Ok(o) = out {
                if o.success() {
                    for (id, krate, bin) in cargo_crates {
                        if !missing.contains(&id) {
                            continue;
                        }
                        if o.stdout
                            .lines()
                            .any(|l| l.starts_with(&format!("{krate} v")))
                        {
                            notes.push((
                                id,
                                format!(
                                    "cargo install --list reports crate `{krate}` installed, but no `{bin}` executable was found in any scanned location"
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    // winget list: installed package knowledge (only when winget is callable).
    let managers = probe_managers(facts);
    if let Some(winget) = &managers.winget {
        let out = crate::process::run(
            &Request::new(
                winget.clone(),
                vec!["list".into(), "--accept-source-agreements".into()],
            )
            .timeout(std::time::Duration::from_secs(20)),
        );
        if let Ok(o) = out {
            if o.success() {
                let lower = o.stdout.to_ascii_lowercase();
                for id in missing {
                    for mech in install_options(*id) {
                        if let InstallMechanism::Winget { package } = &mech {
                            if lower.contains(&format!(" {}", package.to_ascii_lowercase()))
                                || lower.contains(&package.to_ascii_lowercase())
                            {
                                notes.push((
                                    *id,
                                    format!(
                                        "winget list reports package `{package}` installed, but no executable was found in any scanned location"
                                    ),
                                ));
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    notes
}

#[derive(Debug, Clone)]
pub struct InstallRoute {
    pub mechanism: InstallMechanism,
    pub manager_program: String,
    pub display: String,
}

/// Choose the first catalogue mechanism whose manager is actually runnable.
pub fn choose_route(
    id: ToolId,
    managers: Option<&ManagerAvailability>,
    _facts: &EnvFacts,
) -> Option<InstallRoute> {
    let managers = managers?;
    for mech in install_options(id) {
        let Some(program) = mech.manager_program() else {
            continue;
        };
        let resolved = match program {
            "winget" => managers.winget.clone(),
            "choco" => managers.choco.clone(),
            "scoop" => managers.scoop.clone(),
            "cargo" => managers.cargo.clone(),
            "npm" => managers.npm.clone(),
            _ => None,
        };
        if let Some(resolved) = resolved {
            return Some(InstallRoute {
                display: format!("{} via {}", mech_display(&mech), program),
                manager_program: resolved,
                mechanism: mech,
            });
        }
    }
    None
}

pub fn mech_display(m: &InstallMechanism) -> String {
    match m {
        InstallMechanism::Winget { package } => format!("winget install {package}"),
        InstallMechanism::Choco { package } => format!("choco install {package} -y"),
        InstallMechanism::Scoop { package } => format!("scoop install {package}"),
        InstallMechanism::Cargo { package } => format!("cargo install {package} --locked"),
        InstallMechanism::Npm { package } => format!("npm install -g {package}"),
        InstallMechanism::Guidance { url, .. } => url.clone(),
    }
}

pub fn route_args(m: &InstallMechanism) -> Vec<String> {
    match m {
        InstallMechanism::Winget { package } => vec![
            "install".into(),
            "--id".into(),
            package.clone(),
            "-e".into(),
            "--silent".into(),
            "--disable-interactivity".into(),
            "--accept-package-agreements".into(),
            "--accept-source-agreements".into(),
        ],
        InstallMechanism::Choco { package } => {
            vec![
                "install".into(),
                package.clone(),
                "-y".into(),
                "--no-progress".into(),
            ]
        }
        InstallMechanism::Scoop { package } => vec!["install".into(), package.clone()],
        InstallMechanism::Cargo { package } => {
            vec!["install".into(), package.clone(), "--locked".into()]
        }
        InstallMechanism::Npm { package } => {
            vec![
                "install".into(),
                "-g".into(),
                package.clone(),
                "--no-fund".into(),
                "--no-audit".into(),
            ]
        }
        InstallMechanism::Guidance { .. } => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub tool: ToolId,
    pub route: String,
    pub attempted: bool,
    pub manager_exit_code: Option<i32>,
    pub manager_timed_out: bool,
    pub duration_ms: u64,
    /// True only when the executable was actually found and identity-verified
    /// after installation. Never inferred from manager exit code alone.
    pub verified: bool,
    pub resolved_path: Option<PathBuf>,
    pub version: Option<String>,
    pub visible_on_process_path: bool,
    pub on_persisted_user_path: bool,
    pub needs_new_process: bool,
    pub notes: Vec<String>,
    pub manager_output_tail: Option<String>,
}

/// Execute an install route, then verify by re-discovery.
pub fn execute_route(
    id: ToolId,
    route: &InstallRoute,
    operation: &str,
) -> Result<InstallOutcome, crate::output::Failure> {
    let args = route_args(&route.mechanism);
    if args.is_empty() {
        return Err(crate::output::Failure::new(
            operation,
            crate::output::Status::Refused,
            format!(
                "no automatable installation route for {}",
                id.project_name()
            ),
        ));
    }

    let req = Request::new(route.manager_program.clone(), args)
        .timeout(std::time::Duration::from_secs(20 * 60))
        .max_stdout(512 * 1024)
        .max_stderr(512 * 1024);
    let outcome = process::run(&req).map_err(|e| {
        crate::output::Failure::failed(
            operation,
            format!("install manager could not run ({}): {e}", route.display),
        )
    })?;

    Ok(InstallOutcome {
        tool: id,
        route: route.display.clone(),
        attempted: true,
        manager_exit_code: outcome.exit_code,
        manager_timed_out: outcome.timed_out,
        duration_ms: outcome.duration_ms,
        verified: false,
        resolved_path: None,
        version: None,
        visible_on_process_path: false,
        on_persisted_user_path: false,
        needs_new_process: false,
        notes: Vec::new(),
        manager_output_tail: {
            let combined = if outcome.stderr.trim().is_empty() {
                outcome.stdout.clone()
            } else {
                format!("{}\n{}", outcome.stdout, outcome.stderr)
            };
            let (tail, _) = process::tail(combined.trim(), 2048);
            if tail.is_empty() {
                None
            } else {
                Some(tail)
            }
        },
    })
}

/// Post-install verification via fresh deep discovery. Success is never
/// claimed from the manager's exit code alone.
pub fn verify_after_install(outcome: &mut InstallOutcome, deep_registry: &super::ToolRegistry) {
    if let Some(rec) = deep_registry.get(outcome.tool) {
        outcome.verified = matches!(
            rec.status,
            super::ToolStatus::Available | super::ToolStatus::Hidden
        );
        outcome.resolved_path = rec.path.clone();
        outcome.version = rec.version.clone();
        outcome.visible_on_process_path = rec.on_process_path;
        outcome.on_persisted_user_path = rec.on_user_path;
        outcome.needs_new_process = rec.status == super::ToolStatus::Hidden;
        match rec.status {
            super::ToolStatus::Available => {
                outcome.notes.push("executable found and identity-verified".into());
            }
            super::ToolStatus::Hidden => outcome.notes.push(
                "installed, but its directory is not visible to this process; PATH repair or a new shell is required".into(),
            ),
            super::ToolStatus::Broken => outcome.notes.push(
                "an executable was found but failed identity verification".into(),
            ),
            super::ToolStatus::Ambiguous => outcome
                .notes
                .push("multiple installations found after install; manual attention required".into()),
            super::ToolStatus::Missing => outcome.notes.push(
                "package manager completed but no executable was found; installation may have failed or targeted another location".into(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ALL_TOOLS;

    #[test]
    fn every_tool_has_catalogue_entry() {
        for id in ALL_TOOLS {
            assert!(
                !install_options(id).is_empty(),
                "{id:?} has no install options"
            );
        }
    }

    #[test]
    fn no_route_without_managers() {
        let facts = EnvFacts {
            platform: super::super::env::Platform::Windows,
            deep: true,
            process_path: vec![],
            user_path: vec![],
            machine_path: vec![],
            known_dirs: vec![],
            scan_roots: vec![],
            pathext: vec![".exe".into()],
        };
        assert!(choose_route(ToolId::Fd, Some(&ManagerAvailability::default()), &facts).is_none());
        assert!(choose_route(ToolId::Fd, None, &facts).is_none());
    }

    #[test]
    fn cargo_route_selected_when_cargo_available() {
        let facts = EnvFacts {
            platform: super::super::env::Platform::Windows,
            deep: true,
            process_path: vec![],
            user_path: vec![],
            machine_path: vec![],
            known_dirs: vec![],
            scan_roots: vec![],
            pathext: vec![".exe".into()],
        };
        let managers = ManagerAvailability {
            cargo: Some("C:\\x\\cargo.exe".into()),
            ..Default::default()
        };
        let route = choose_route(ToolId::Just, Some(&managers), &facts).unwrap();
        assert!(route.display.contains("cargo install just"));
    }

    #[test]
    fn threadmoth_has_no_automatable_route() {
        let options = install_options(ToolId::Threadmoth);
        assert!(options
            .iter()
            .all(|m| matches!(m, InstallMechanism::Guidance { .. })));
    }
}
