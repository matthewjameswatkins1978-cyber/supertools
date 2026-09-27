//! CLI surface. Thin dispatch; all semantics live in the domain modules and
//! the catalogue.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "supertools",
    version,
    about = "Agent-native harnesses for existing developer tools",
    long_about = "Supertools makes common developer tools easier, safer and more predictable for AI coding agents.\n\
                  It wraps existing tools (git, ripgrep, fd, gh, cargo, mise, just, ast-grep) with bounded\n\
                  operations, structured evidence, safe defaults and runtime self-teaching.\n\n\
                  Start here: supertools capabilities | supertools describe | supertools teach <domain> | supertools doctor",
    after_help = "EXIT CODES:\n  \
                  0 success · 1 execution/verification failure (incl. timeout) · 2 invalid/refused/ambiguous ·\n  \
                  3 capability unavailable · 4 completed, no matches/no applicable result\n\n\
                  All commands accept --json for the canonical machine-readable envelope."
)]
pub struct Cli {
    /// Canonical machine-readable JSON envelope on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Is anything actually wrong? Health classification + repair guidance.
    Doctor,
    /// What can Supertools actually do right now?
    Capabilities,
    /// Factual operation schemas: describe [domain|operation-id|exit-codes|schema]
    Describe {
        /// Domain (search/repo/verify/tools/common), operation id, "exit-codes" or "schema".
        target: Option<String>,
    },
    /// Compact operational guidance for an AI agent.
    Teach {
        /// search | repo | verify | tools | install
        topic: String,
        /// For `teach install`: codex | opencode | stdout
        #[arg(long)]
        target: Option<String>,
        /// For `teach install`: explicit AGENTS.md-style file path
        #[arg(long)]
        path: Option<PathBuf>,
        /// For `teach install`: write the managed block (idempotent, preserves other content)
        #[arg(long)]
        apply: bool,
    },
    /// Tool inventory: what developer tools does this machine have?
    Tools {
        #[command(subcommand)]
        cmd: Option<ToolsCommand>,
    },
    /// Deep tool investigation: where is it installed, why invisible, what to do.
    Find {
        /// Single tool to investigate (git, cargo, rustc, rg, fd, gh, mise, just, ast-grep, sg, threadmoth).
        tool: Option<String>,
        /// Offer installation of one missing tool (consent required unless --yes).
        #[arg(long)]
        install: bool,
        /// Walk through all missing recommended tools interactively.
        #[arg(long = "install-missing")]
        install_missing: bool,
        /// USER PATH repair workflow (additive, preview first).
        #[arg(long = "fix-path")]
        fix_path: bool,
        /// Non-mutating inspection (with --fix-path).
        #[arg(long)]
        dry_run: bool,
        /// Unattended: skip confirmation prompts.
        #[arg(long)]
        yes: bool,
    },
    /// Bounded repository discovery (read-only).
    Search {
        #[command(subcommand)]
        cmd: SearchCommand,
    },
    /// Trustworthy repository state (read-only).
    Repo {
        #[command(subcommand)]
        cmd: RepoCommand,
    },
    /// Project-authoritative verification.
    Verify {
        #[command(subcommand)]
        cmd: VerifyCommand,
    },
}

#[derive(Subcommand)]
pub enum ToolsCommand {
    /// Detailed registry record for one tool.
    Show { tool: String },
    /// Tools with status=missing.
    Missing,
    /// Tools installed but invisible to this process.
    Hidden,
    /// Tools currently usable.
    Available,
    /// Install missing recommended tools (consent-gated, verified by re-discovery).
    #[command(name = "install-missing")]
    InstallMissing {
        /// Unattended: skip per-tool confirmation prompts.
        #[arg(long)]
        yes: bool,
    },
    /// Additive USER PATH repair so hidden tools become visible to new processes.
    #[command(name = "fix-path")]
    FixPath {
        /// Plan without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Apply without interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum SearchCommand {
    /// Bounded textual matches (backend: rg; internal fixed-substring fallback).
    Text {
        /// Search string (regex by default; never shell-evaluated).
        query: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Restrict to a directory.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Treat the query as a fixed substring.
        #[arg(long, conflicts_with = "regex")]
        fixed: bool,
        /// Explicit regex mode (default).
        #[arg(long)]
        regex: bool,
        /// Include/exclude globs (rg --glob), repeatable.
        #[arg(long = "glob")]
        globs: Vec<String>,
    },
    /// Find files/paths by name (backend: fd; bounded git fallback).
    Files {
        query: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        path: Option<PathBuf>,
        /// Treat the query as a fixed substring (fd --fixed-strings).
        #[arg(long)]
        fixed: bool,
    },
    /// Matches with bounded surrounding context.
    Context {
        query: String,
        /// Maximum number of context blocks.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "regex")]
        fixed: bool,
        #[arg(long)]
        regex: bool,
        /// Context lines around each match (1-10).
        #[arg(long, default_value_t = 2)]
        context: usize,
    },
    /// Heuristic symbol-definition search (explicitly labeled heuristic).
    Symbol {
        /// Symbol name (letters, digits, _ — escaped for regex use).
        query: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Syntax-aware structural search (backend: ast-grep).
    Structural {
        /// ast-grep pattern, e.g. `fn $NAME($$$)`.
        pattern: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum RepoCommand {
    /// One-call trustworthy repository state.
    State,
    /// Bounded structured changed-file list.
    Changed {
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Bounded recent history, optionally scoped to a path.
    History {
        /// Path to scope history to.
        target: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Remote identities (names, URLs, GitHub owner/repo).
    Remote,
    /// Current-branch PR state (needs gh + GitHub remote + auth).
    Pr,
    /// Recent CI runs (needs gh + GitHub remote + auth).
    Ci {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum VerifyCommand {
    /// Discover this project's verification authority (does not run tasks).
    Discover,
    /// Cheapest authoritative verification.
    Quick {
        /// Kill verification after N seconds.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Strongest locally appropriate authoritative verification.
    Full {
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Run one DECLARED task by exact name (arbitrary strings are refused).
    Task {
        name: String,
        #[arg(long)]
        timeout: Option<u64>,
    },
}
