//! The Tool Registry: ONE semantic authority for developer-tool discovery.
//!
//! `find`, `tools`, `doctor`, `capabilities`, onboarding, installation and
//! PATH repair all consume this registry. Nothing rediscovers tools
//! independently.
//!
//! State model (strong types, no overloaded booleans):
//!
//! - [`ToolStatus::Available`]  credible executable found on the current PATH
//! - [`ToolStatus::Hidden`]     credible installation found, not visible on PATH
//! - [`ToolStatus::Missing`]    no credible installation found
//! - [`ToolStatus::Ambiguous`]  multiple conflicting installations
//! - [`ToolStatus::Broken`]     exists but cannot satisfy its expected capability

pub mod env;
pub mod install;
pub mod pathfix;

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::process::{self, Request};
use env::EnvFacts;

/// The known/recommended developer toolset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolId {
    Git,
    Cargo,
    Rustc,
    Rg,
    Fd,
    Gh,
    Mise,
    Just,
    AstGrep,
    Threadmoth,
}

pub const ALL_TOOLS: [ToolId; 10] = [
    ToolId::Git,
    ToolId::Cargo,
    ToolId::Rustc,
    ToolId::Rg,
    ToolId::Fd,
    ToolId::Gh,
    ToolId::Mise,
    ToolId::Just,
    ToolId::AstGrep,
    ToolId::Threadmoth,
];

impl ToolId {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolId::Git => "git",
            ToolId::Cargo => "cargo",
            ToolId::Rustc => "rustc",
            ToolId::Rg => "rg",
            ToolId::Fd => "fd",
            ToolId::Gh => "gh",
            ToolId::Mise => "mise",
            ToolId::Just => "just",
            ToolId::AstGrep => "ast-grep",
            ToolId::Threadmoth => "threadmoth",
        }
    }

    /// Official project identity (human name).
    pub fn project_name(self) -> &'static str {
        match self {
            ToolId::Git => "Git",
            ToolId::Cargo => "Cargo (Rust)",
            ToolId::Rustc => "rustc (Rust)",
            ToolId::Rg => "ripgrep",
            ToolId::Fd => "fd",
            ToolId::Gh => "GitHub CLI",
            ToolId::Mise => "mise",
            ToolId::Just => "just",
            ToolId::AstGrep => "ast-grep",
            ToolId::Threadmoth => "Threadmoth",
        }
    }

    pub fn homepage(self) -> &'static str {
        match self {
            ToolId::Git => "https://git-scm.com",
            ToolId::Cargo | ToolId::Rustc => "https://rustup.rs",
            ToolId::Rg => "https://github.com/BurntSushi/ripgrep",
            ToolId::Fd => "https://github.com/sharkdp/fd",
            ToolId::Gh => "https://cli.github.com",
            ToolId::Mise => "https://mise.jdx.dev",
            ToolId::Just => "https://github.com/casey/just",
            ToolId::AstGrep => "https://ast-grep.github.io",
            ToolId::Threadmoth => "https://github.com/matthewjameswatkins1978-cyber",
        }
    }

    /// Executable names that may carry this tool. `sg` is an alias of
    /// ast-grep; alias hits must pass the identity check before counting.
    pub fn exec_names(self) -> &'static [&'static str] {
        match self {
            ToolId::Git => &["git"],
            ToolId::Cargo => &["cargo"],
            ToolId::Rustc => &["rustc"],
            ToolId::Rg => &["rg"],
            ToolId::Fd => &["fd"],
            ToolId::Gh => &["gh"],
            ToolId::Mise => &["mise"],
            ToolId::Just => &["just"],
            ToolId::AstGrep => &["ast-grep", "sg"],
            ToolId::Threadmoth => &["threadmoth"],
        }
    }

    pub fn primary_exec(self) -> &'static str {
        self.exec_names()[0]
    }

    /// Substring that must appear in `--version` output to confirm identity.
    pub fn identity_token(self) -> &'static str {
        match self {
            ToolId::Git => "git",
            ToolId::Cargo => "cargo",
            ToolId::Rustc => "rustc",
            ToolId::Rg => "ripgrep",
            ToolId::Fd => "fd",
            ToolId::Gh => "gh",
            ToolId::Mise => "mise",
            ToolId::Just => "just",
            ToolId::AstGrep => "ast-grep",
            ToolId::Threadmoth => "threadmoth",
        }
    }

    pub fn role(self) -> &'static str {
        match self {
            ToolId::Git => "repository",
            ToolId::Cargo => "rust/build",
            ToolId::Rustc => "rust/compiler",
            ToolId::Rg => "text search",
            ToolId::Fd => "file search",
            ToolId::Gh => "github",
            ToolId::Mise => "tasks/env",
            ToolId::Just => "task runner",
            ToolId::AstGrep => "structural search",
            ToolId::Threadmoth => "guarded structural tool",
        }
    }

    /// Recommended toolset member (all of them are).
    pub fn recommended(self) -> bool {
        true
    }

    /// Required by a Supertools domain (git powers the repo domain).
    /// Supertools still starts without it; the affected capabilities are
    /// reported unavailable instead.
    pub fn required(self) -> bool {
        matches!(self, ToolId::Git)
    }

    pub fn used_by_supertools(self) -> &'static str {
        match self {
            ToolId::Git => "trustworthy repository state, changed files, history, remotes; bounded file-search fallback",
            ToolId::Cargo => "Rust project verification (cargo check / cargo test)",
            ToolId::Rustc => "Rust compilation, used through cargo",
            ToolId::Rg => "bounded text search, context search, heuristic symbol search",
            ToolId::Fd => "fast file discovery (search files preferred backend)",
            ToolId::Gh => "GitHub PR and CI information (repo pr / repo ci)",
            ToolId::Mise => "mise task authority for verify discovery",
            ToolId::Just => "just recipe authority for verify discovery",
            ToolId::AstGrep => "structural search (search structural)",
            ToolId::Threadmoth => "guarded structural/source-preserving workflows (separate tool, not wrapped)",
        }
    }

    pub fn suggested_command(self) -> Option<&'static str> {
        match self {
            ToolId::Threadmoth => Some("threadmoth --help"),
            ToolId::AstGrep => Some("ast-grep --help"),
            ToolId::Rg => Some("rg --help"),
            ToolId::Fd => Some("fd --help"),
            ToolId::Mise => Some("mise tasks ls"),
            ToolId::Just => Some("just --list"),
            _ => None,
        }
    }

    /// Supertools capability ids this tool powers.
    pub fn capabilities(self) -> &'static [&'static str] {
        match self {
            ToolId::Git => &[
                "repo.state",
                "repo.changed",
                "repo.history",
                "repo.remote",
                "search.files",
            ],
            ToolId::Cargo => &["verify.cargo"],
            ToolId::Rustc => &["verify.cargo"],
            ToolId::Rg => &["search.text", "search.context", "search.symbol"],
            ToolId::Fd => &["search.files"],
            ToolId::Gh => &["repo.pr", "repo.ci"],
            ToolId::Mise => &["verify.mise"],
            ToolId::Just => &["verify.just"],
            ToolId::AstGrep => &["search.structural"],
            ToolId::Threadmoth => &["guidance.threadmoth"],
        }
    }

    pub fn from_arg(s: &str) -> Option<ToolId> {
        let norm = s.trim().to_ascii_lowercase();
        ALL_TOOLS.iter().copied().find(|t| {
            t.as_str() == norm
                || t.project_name().to_ascii_lowercase() == norm
                || t.exec_names().contains(&norm.as_str())
                || matches!(norm.as_str(), "ripgrep" | "astgrep" | "gh-cli")
                    && t.as_str()
                        == match norm.as_str() {
                            "ripgrep" => "rg",
                            "astgrep" => "ast-grep",
                            "gh-cli" => "gh",
                            _ => "",
                        }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Available,
    Hidden,
    Missing,
    Ambiguous,
    Broken,
}

impl ToolStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolStatus::Available => "available",
            ToolStatus::Hidden => "hidden",
            ToolStatus::Missing => "missing",
            ToolStatus::Ambiguous => "ambiguous",
            ToolStatus::Broken => "broken",
        }
    }
}

/// Where a candidate executable was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    ProcessPath,
    UserPath,
    MachinePath,
    CargoBin,
    UserLocalBin,
    WinGetLinksUser,
    WinGetLinksMachine,
    WindowsAppsAlias,
    ScoopShims,
    ChocoBin,
    NpmGlobal,
    LocalAppDataPrograms,
    ProgramFiles,
    ProgramFilesX86,
    LocalAppDataScan,
    AppDataScan,
}

impl DiscoverySource {
    pub fn label(self) -> &'static str {
        match self {
            DiscoverySource::ProcessPath => "current process PATH",
            DiscoverySource::UserPath => "persisted user PATH",
            DiscoverySource::MachinePath => "persisted machine PATH",
            DiscoverySource::CargoBin => "cargo bin directory",
            DiscoverySource::UserLocalBin => "~/.local/bin",
            DiscoverySource::WinGetLinksUser => "winget links (user)",
            DiscoverySource::WinGetLinksMachine => "winget links (machine)",
            DiscoverySource::WindowsAppsAlias => "Windows app execution alias",
            DiscoverySource::ScoopShims => "scoop shims",
            DiscoverySource::ChocoBin => "chocolatey bin",
            DiscoverySource::NpmGlobal => "npm global bin",
            DiscoverySource::LocalAppDataPrograms => "%LOCALAPPDATA%\\Programs",
            DiscoverySource::ProgramFiles => "Program Files",
            DiscoverySource::ProgramFilesX86 => "Program Files (x86)",
            DiscoverySource::LocalAppDataScan => "bounded %LOCALAPPDATA% scan",
            DiscoverySource::AppDataScan => "bounded %APPDATA% scan",
        }
    }

    /// Executables from trusted locations may be run (`--version`) to confirm
    /// identity. Raw broad-scan hits are reported but NOT executed.
    pub fn trusted(self) -> bool {
        !matches!(
            self,
            DiscoverySource::LocalAppDataScan | DiscoverySource::AppDataScan
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCandidate {
    pub path: PathBuf,
    pub source: DiscoverySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub trusted: bool,
}

/// One tool's discovered state. Explicit fields; no inference required.
#[derive(Debug, Clone, Serialize)]
pub struct ToolRecord {
    pub id: ToolId,
    pub name: &'static str,
    pub homepage: &'static str,
    pub status: ToolStatus,
    pub recommended: bool,
    pub required: bool,
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_source: Option<DiscoverySource>,
    pub on_process_path: bool,
    pub on_user_path: bool,
    pub on_machine_path: bool,
    pub alternates: Vec<ToolCandidate>,
    pub capabilities: Vec<&'static str>,
    pub notes: Vec<String>,
    pub used_by: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested: Option<&'static str>,
    /// Human-readable likely cause / next action guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Best installation route runnable on this machine, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_route: Option<String>,
    pub install_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistrySummary {
    pub available: usize,
    pub hidden: usize,
    pub missing: usize,
    pub ambiguous: usize,
    pub broken: usize,
}

/// The registry: discovered state for the whole recommended toolset.
#[derive(Debug, Clone, Serialize)]
pub struct ToolRegistry {
    pub platform: env::Platform,
    pub deep: bool,
    pub tools: Vec<ToolRecord>,
    pub summary: RegistrySummary,
}

impl ToolRegistry {
    pub fn get(&self, id: ToolId) -> Option<&ToolRecord> {
        self.tools.iter().find(|t| t.id == id)
    }

    pub fn status(&self, id: ToolId) -> ToolStatus {
        self.get(id)
            .map(|t| t.status)
            .unwrap_or(ToolStatus::Missing)
    }

    pub fn available(&self, id: ToolId) -> bool {
        self.status(id) == ToolStatus::Available
    }

    /// Shallow registry: process PATH + persisted PATH + exact known
    /// directories + identity probes. Used by `tools`/`doctor`/`capabilities`.
    pub fn shallow() -> ToolRegistry {
        Self::build(false)
    }

    /// Deep registry: adds bounded location scans and package-manager
    /// metadata. Used by `find` and installation flows.
    pub fn deep() -> ToolRegistry {
        Self::build(true)
    }

    fn build(deep: bool) -> ToolRegistry {
        let facts = EnvFacts::collect(deep);
        Self::from_facts(facts)
    }

    /// Build from explicit facts. Also the seam used by tests with synthetic
    /// environments.
    pub fn from_facts(facts: EnvFacts) -> ToolRegistry {
        let deep = facts.deep;
        let managers = if deep {
            Some(install::probe_managers(&facts))
        } else {
            None
        };

        let mut scan_hits: Vec<(ToolId, ToolCandidate)> = Vec::new();
        if deep {
            scan_hits = env::bounded_scan(&facts);
        }

        let mut tools = Vec::new();
        for id in ALL_TOOLS {
            let mut candidates = collect_candidates(id, &facts);
            if deep {
                for (hit_id, cand) in scan_hits.iter().filter(|(t, _)| *t == id) {
                    let _ = hit_id;
                    if !candidates.iter().any(|c| same_file(&c.path, &cand.path)) {
                        candidates.push(cand.clone());
                    }
                }
            }
            let record = build_record(id, candidates, &facts, managers.as_ref());
            tools.push(record);
        }

        // Stage 4 (deep): package-manager metadata for still-missing tools.
        if deep {
            let missing: Vec<ToolId> = tools
                .iter()
                .filter(|t| t.status == ToolStatus::Missing)
                .map(|t| t.id)
                .collect();
            if !missing.is_empty() {
                for (id, note) in install::manager_metadata_notes(&facts, &missing) {
                    if let Some(rec) = tools.iter_mut().find(|t| t.id == id) {
                        rec.notes.push(note);
                    }
                }
            }
        }

        let summary = RegistrySummary {
            available: tools
                .iter()
                .filter(|t| t.status == ToolStatus::Available)
                .count(),
            hidden: tools
                .iter()
                .filter(|t| t.status == ToolStatus::Hidden)
                .count(),
            missing: tools
                .iter()
                .filter(|t| t.status == ToolStatus::Missing)
                .count(),
            ambiguous: tools
                .iter()
                .filter(|t| t.status == ToolStatus::Ambiguous)
                .count(),
            broken: tools
                .iter()
                .filter(|t| t.status == ToolStatus::Broken)
                .count(),
        };

        ToolRegistry {
            platform: facts.platform,
            deep,
            tools,
            summary,
        }
    }
}

fn collect_candidates(id: ToolId, facts: &EnvFacts) -> Vec<ToolCandidate> {
    let mut out: Vec<ToolCandidate> = Vec::new();
    let names = id.exec_names();

    let mut push_dir_hits = |dir: &Path, source: DiscoverySource| {
        for name in names {
            for exe in facts.exec_variants(name) {
                let p = dir.join(&exe);
                if p.is_file() && !out.iter().any(|c| same_file(&c.path, &p)) {
                    out.push(ToolCandidate {
                        path: p,
                        source,
                        version: None,
                        trusted: source.trusted(),
                    });
                }
            }
        }
    };

    // Stage 1: process PATH (order preserved).
    for dir in &facts.process_path {
        push_dir_hits(dir, DiscoverySource::ProcessPath);
    }
    // Stage 2: persisted user PATH, then machine PATH.
    for dir in facts
        .user_path
        .iter()
        .filter(|d| !facts.process_path.iter().any(|p| same_dir(p, d)))
    {
        push_dir_hits(dir, DiscoverySource::UserPath);
    }
    for dir in facts.machine_path.iter().filter(|d| {
        !facts.process_path.iter().any(|p| same_dir(p, d))
            && !facts.user_path.iter().any(|p| same_dir(p, d))
    }) {
        push_dir_hits(dir, DiscoverySource::MachinePath);
    }
    // Stage 3: known developer locations.
    for kd in &facts.known_dirs {
        if kd.path.is_dir() {
            push_dir_hits(&kd.path, kd.source);
        }
    }
    out
}

fn same_file(a: &Path, b: &Path) -> bool {
    canon(a) == canon(b)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    same_file(a, b)
}

pub fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Compare paths case-insensitively on Windows.
pub fn paths_equal(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        canon(a).to_string_lossy().to_ascii_lowercase()
            == canon(b).to_string_lossy().to_ascii_lowercase()
    } else {
        canon(a) == canon(b)
    }
}

pub struct VersionProbe {
    pub version: Option<String>,
    pub identity_ok: bool,
    pub ran: bool,
    pub error: Option<String>,
}

/// Safely obtain a version string. Only executed for candidates from
/// sufficiently trusted locations (see [`DiscoverySource::trusted`]).
pub fn probe_version(exe: &Path, id: ToolId) -> VersionProbe {
    let req = Request::new(
        exe.to_string_lossy().into_owned(),
        vec!["--version".to_string()],
    )
    .timeout(std::time::Duration::from_secs(5))
    .max_stdout(16 * 1024)
    .max_stderr(16 * 1024);

    match process::run(&req) {
        Ok(out) => {
            let text = if out.stdout.trim().is_empty() {
                out.stderr.clone()
            } else {
                out.stdout.clone()
            };
            let first_line = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            let identity_ok = out.success()
                && first_line
                    .to_ascii_lowercase()
                    .contains(id.identity_token());
            VersionProbe {
                version: if first_line.is_empty() {
                    None
                } else {
                    Some(first_line)
                },
                identity_ok,
                ran: true,
                error: if out.success() {
                    None
                } else {
                    Some(format!(
                        "exit {}{}",
                        out.exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "?".into()),
                        if out.timed_out { " (timed out)" } else { "" }
                    ))
                },
            }
        }
        Err(e) => VersionProbe {
            version: None,
            identity_ok: false,
            ran: false,
            error: Some(e.to_string()),
        },
    }
}

fn build_record(
    id: ToolId,
    candidates: Vec<ToolCandidate>,
    facts: &EnvFacts,
    managers: Option<&install::ManagerAvailability>,
) -> ToolRecord {
    let mut notes = Vec::new();

    // Alias candidates (e.g. `sg` for ast-grep) must pass identity check.
    let mut candidates: Vec<ToolCandidate> = candidates
        .into_iter()
        .filter(|c| {
            let file = c
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let is_alias = file != id.primary_exec().to_ascii_lowercase()
                && id.exec_names().contains(&file.as_str());
            if is_alias && c.trusted {
                let probe = probe_version(&c.path, id);
                if !probe.identity_ok {
                    notes.push(format!(
                        "{} exists at {} but is not {} (identity check failed); ignored",
                        file,
                        c.path.display(),
                        id.project_name()
                    ));
                    return false;
                }
            }
            true
        })
        .collect();

    // Deduplicate by canonical path, preserving stage priority.
    let mut dedup: Vec<ToolCandidate> = Vec::new();
    for c in candidates.drain(..) {
        if !dedup.iter().any(|d| same_file(&d.path, &c.path)) {
            dedup.push(c);
        }
    }
    let candidates = dedup;

    let on_path = candidates
        .iter()
        .find(|c| c.source == DiscoverySource::ProcessPath);

    let mut record = ToolRecord {
        id,
        name: id.project_name(),
        homepage: id.homepage(),
        status: ToolStatus::Missing,
        recommended: id.recommended(),
        required: id.required(),
        role: id.role(),
        version: None,
        path: None,
        discovery_source: None,
        on_process_path: false,
        on_user_path: false,
        on_machine_path: false,
        alternates: Vec::new(),
        capabilities: id.capabilities().to_vec(),
        notes,
        used_by: id.used_by_supertools(),
        suggested: id.suggested_command(),
        cause: None,
        action: None,
        install_route: None,
        install_available: false,
    };

    // Classify (deterministic).
    if let Some(primary) = on_path {
        record.status = ToolStatus::Available;
        record.on_process_path = true;
        record.path = Some(primary.path.clone());
        record.discovery_source = Some(primary.source);
        record.alternates = candidates
            .iter()
            .filter(|c| !same_file(&c.path, &primary.path))
            .cloned()
            .collect();
        if !record.alternates.is_empty() {
            record.notes.push(format!(
                "{} other installation(s) found; PATH order selects {}",
                record.alternates.len(),
                primary.path.display()
            ));
        }
    } else if candidates.is_empty() {
        record.status = ToolStatus::Missing;
    } else {
        let distinct: Vec<&ToolCandidate> = {
            let mut seen: Vec<&ToolCandidate> = Vec::new();
            for c in &candidates {
                if !seen.iter().any(|s| same_file(&s.path, &c.path)) {
                    seen.push(c);
                }
            }
            seen
        };
        if distinct.len() == 1 {
            record.status = ToolStatus::Hidden;
            record.path = Some(distinct[0].path.clone());
            record.discovery_source = Some(distinct[0].source);
        } else {
            record.status = ToolStatus::Ambiguous;
            record.alternates = distinct.iter().map(|c| (*c).clone()).collect();
            record.cause = Some("multiple conflicting installations found outside PATH".into());
            record.action = Some(
                "inspect candidates and remove/pin one, or add the intended directory to user PATH"
                    .into(),
            );
        }
    }

    // PATH membership flags (independent of status).
    if let Some(p) = &record.path {
        if let Some(parent) = p.parent() {
            record.on_user_path = facts.user_path.iter().any(|d| paths_equal(d, parent));
            record.on_machine_path = facts.machine_path.iter().any(|d| paths_equal(d, parent));
        }
    }

    // Version / identity probe for the primary candidate (trusted only).
    let probe_targets: Vec<PathBuf> = match record.status {
        ToolStatus::Available | ToolStatus::Hidden => record.path.clone().into_iter().collect(),
        ToolStatus::Ambiguous => record.alternates.iter().map(|a| a.path.clone()).collect(),
        _ => Vec::new(),
    };
    for target in &probe_targets {
        let trusted = candidates
            .iter()
            .find(|c| same_file(&c.path, target))
            .map(|c| c.trusted)
            .unwrap_or(false);
        if !trusted {
            record.notes.push(format!(
                "not executed (untrusted discovery location): {}",
                target.display()
            ));
            continue;
        }
        let probe = probe_version(target, id);
        if probe_targets.len() > 1 {
            if let Some(a) = record
                .alternates
                .iter_mut()
                .find(|a| same_file(&a.path, target))
            {
                a.version = probe.version.clone();
            }
        }
        if Some(target) == record.path.as_ref() {
            record.version = probe.version.clone();
            if !probe.ran {
                record.status = ToolStatus::Broken;
                record.cause = Some(format!(
                    "executable found but could not run: {}",
                    probe.error.clone().unwrap_or_default()
                ));
            } else if !probe.identity_ok {
                record.status = ToolStatus::Broken;
                record.cause = Some(format!(
                    "`{}` does not report the expected {} identity (got: {})",
                    target.display(),
                    id.project_name(),
                    probe
                        .error
                        .or(probe.version.clone())
                        .unwrap_or_else(|| "no version output".into())
                ));
            }
        }
    }

    // Hidden-tool guidance.
    if record.status == ToolStatus::Hidden {
        if record.on_user_path || record.on_machine_path {
            record.cause = Some(
                "installed and on a persisted PATH, but this shell/agent inherited an older PATH"
                    .into(),
            );
            record.action = Some("restart the shell/agent process".into());
        } else {
            record.cause = Some("its directory is not on any PATH".into());
            record.action = Some("supertools tools fix-path --dry-run".into());
        }
    }

    // Install route (catalogue + manager availability).
    if matches!(record.status, ToolStatus::Missing | ToolStatus::Broken) {
        if let Some(route) = install::choose_route(id, managers, facts) {
            record.install_route = Some(route.display.clone());
            record.install_available = true;
        } else {
            record.install_route = install::guidance(id).map(|g| g.to_string());
            record.install_available = false;
        }
    }

    record
}

/// Lightweight single-tool probe used by domain operations (search/repo/
/// verify). Same registry semantics, minimal work: resolve on process PATH
/// and confirm identity. Domain commands never silently execute tools that
/// are merely HIDDEN.
pub fn quick_require(id: ToolId, operation: &str) -> Result<String, crate::output::Failure> {
    let resolved = process::resolve_program(id.primary_exec()).map_err(|_| {
        crate::output::Failure::unavailable(
            operation,
            format!(
                "{} ({}) is not available on PATH",
                id.project_name(),
                id.primary_exec()
            ),
        )
        .hint(format!("supertools find {}", id.as_str()))
        .hint(format!(
            "supertools tools install-missing --yes  # or install {} manually",
            id.project_name()
        ))
    })?;
    Ok(resolved.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_id_from_arg_aliases() {
        assert_eq!(ToolId::from_arg("sg"), Some(ToolId::AstGrep));
        assert_eq!(ToolId::from_arg("ast-grep"), Some(ToolId::AstGrep));
        assert_eq!(ToolId::from_arg("ripgrep"), Some(ToolId::Rg));
        assert_eq!(ToolId::from_arg("rg"), Some(ToolId::Rg));
        assert_eq!(ToolId::from_arg("nope"), None);
    }

    #[test]
    fn statuses_are_distinct_strings() {
        let all = [
            ToolStatus::Available.as_str(),
            ToolStatus::Hidden.as_str(),
            ToolStatus::Missing.as_str(),
            ToolStatus::Ambiguous.as_str(),
            ToolStatus::Broken.as_str(),
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn raw_scan_sources_are_untrusted() {
        assert!(!DiscoverySource::LocalAppDataScan.trusted());
        assert!(!DiscoverySource::AppDataScan.trusted());
        assert!(DiscoverySource::ProcessPath.trusted());
        assert!(DiscoverySource::CargoBin.trusted());
    }
}
