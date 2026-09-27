//! The operation catalogue: ONE semantic authority for what Supertools can
//! do. `describe`, `teach`, `capabilities` and CLI help are all generated
//! from this table. Operation IDs are stable.

use serde::Serialize;

use crate::registry::ToolId;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Query,
    Path,
    Name,
    Number,
    Flag,
    Pattern,
    Topic,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct InputSpec {
    pub name: &'static str,
    pub kind: InputKind,
    pub required: bool,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Operation {
    pub id: &'static str,
    pub domain: &'static str,
    /// Canonical CLI invocation shape.
    pub cli: &'static str,
    pub purpose: &'static str,
    pub when_to_use: &'static [&'static str],
    pub when_not_to_use: &'static [&'static str],
    /// Capabilities that must be present (tool ids).
    pub requires: &'static [ToolId],
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub inputs: &'static [InputSpec],
    /// What the evidence/data contains.
    pub output: &'static str,
    /// Typical follow-up operation ids.
    pub next: &'static [&'static str],
    /// Fallback guidance when this operation is unavailable.
    pub fallback: Option<&'static str>,
}

pub const DOMAINS: [&str; 5] = ["search", "repo", "verify", "tools", "common"];

const Q: InputSpec = InputSpec {
    name: "query",
    kind: InputKind::Query,
    required: true,
    description: "untrusted search string; never interpreted by a shell",
};
const LIMIT: InputSpec = InputSpec {
    name: "--limit",
    kind: InputKind::Number,
    required: false,
    description: "maximum results returned (bounded output; truncation is reported)",
};
const PATHOPT: InputSpec = InputSpec {
    name: "--path",
    kind: InputKind::Path,
    required: false,
    description: "restrict operation to a directory inside the repository",
};

pub static OPERATIONS: &[Operation] = &[
    // ---------------- search ----------------
    Operation {
        id: "search.text",
        domain: "search",
        cli: "supertools search text <query> [--fixed|--regex] [--limit N] [--glob G] [--path P]",
        purpose: "Find bounded textual matches in repository content",
        when_to_use: &[
            "finding a literal or regex occurrence in code or docs",
            "investigating an unfamiliar codebase before modifying it",
        ],
        when_not_to_use: &[
            "when syntactic structure rather than text is consequential (use search.structural)",
            "when you need to locate files by name (use search.files)",
        ],
        requires: &[ToolId::Rg],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[Q, LIMIT, PATHOPT],
        output: "matches[] with path, line, column, matched text; explicit truncation state",
        next: &["search.context", "search.files"],
        fallback: Some("without rg, a bounded internal fixed-substring scan runs instead (regex requires rg)"),
    },
    Operation {
        id: "search.files",
        domain: "search",
        cli: "supertools search files <query> [--fixed] [--limit N] [--path P]",
        purpose: "Find repository files/paths whose names match a query",
        when_to_use: &[
            "locating files by name or path fragment",
            "building a mental map of an unfamiliar repository",
        ],
        when_not_to_use: &["searching inside file contents (use search.text)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[Q, LIMIT, PATHOPT],
        output: "paths[] respecting ignore rules; backend used; truncation state",
        next: &["search.text"],
        fallback: Some("preferred backend fd; falls back to a bounded git ls-files scan"),
    },
    Operation {
        id: "search.context",
        domain: "search",
        cli: "supertools search context <query> [--context N] [--limit N] [--path P]",
        purpose: "Textual matches with bounded surrounding lines so a match can be understood in one call",
        when_to_use: &[
            "understanding a match without issuing several follow-up reads",
        ],
        when_not_to_use: &["reading entire files (it returns bounded windows, not whole files)"],
        requires: &[ToolId::Rg],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[Q, LIMIT, PATHOPT],
        output: "blocks[] of {path, anchor line, window lines with match markers}",
        next: &["search.text", "search.structural"],
        fallback: Some("without rg, bounded internal fixed-substring scan with context"),
    },
    Operation {
        id: "search.symbol",
        domain: "search",
        cli: "supertools search symbol <name> [--limit N] [--path P]",
        purpose: "Heuristic search for definitions/uses of a symbol name (explicitly heuristic, not semantic)",
        when_to_use: &["quickly locating where a function/type/constant is likely defined"],
        when_not_to_use: &[
            "when you need true semantic symbol resolution (it is a labeled heuristic)",
        ],
        requires: &[ToolId::Rg],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[Q, LIMIT, PATHOPT],
        output: "matches[] like search.text plus heuristic=true marker and pattern used",
        next: &["search.context", "search.structural"],
        fallback: None,
    },
    Operation {
        id: "search.structural",
        domain: "search",
        cli: "supertools search structural <pattern> [--limit N] [--path P]",
        purpose: "Syntax-aware structural search via ast-grep patterns",
        when_to_use: &[
            "when syntactic structure is consequential (e.g. `fn $NAME($$$)`)",
            "finding code shapes rather than text",
        ],
        when_not_to_use: &["plain text lookup (use search.text; it is cheaper)"],
        requires: &[ToolId::AstGrep],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[
            InputSpec { name: "pattern", kind: InputKind::Pattern, required: true, description: "ast-grep pattern; language inferred from file extension" },
            LIMIT,
            PATHOPT,
        ],
        output: "structured matches[] with file, range, matched text",
        next: &["search.context"],
        fallback: Some("if ast-grep is unavailable: capability_unavailable; search.text is suggested only when text is semantically sufficient"),
    },
    // ---------------- repo ----------------
    Operation {
        id: "repo.state",
        domain: "repo",
        cli: "supertools repo state",
        purpose: "One-call trustworthy repository state (branch, HEAD, dirty counts, upstream, in-progress operation)",
        when_to_use: &[
            "before modifying a repository",
            "answering 'what state is this repo in?' without five git calls",
        ],
        when_not_to_use: &["inspecting per-file change detail (use repo.changed)"],
        requires: &[ToolId::Git],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "root, branch, head, detached, upstream, ahead/behind, dirty, staged/unstaged/untracked/conflict counts, operation state",
        next: &["repo.changed", "repo.history"],
        fallback: None,
    },
    Operation {
        id: "repo.changed",
        domain: "repo",
        cli: "supertools repo changed [--limit N]",
        purpose: "Bounded structured list of changed files (staged/unstaged/untracked/conflicts/renames)",
        when_to_use: &["reviewing what changed before verifying or reporting"],
        when_not_to_use: &["reading full diffs (Supertools never dumps entire diffs)"],
        requires: &[ToolId::Git],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[LIMIT],
        output: "files[] with NUL-safe paths, staged/unstaged/untracked/conflict flags, rename sources",
        next: &["repo.state", "verify.quick"],
        fallback: None,
    },
    Operation {
        id: "repo.history",
        domain: "repo",
        cli: "supertools repo history [target] [--limit N]",
        purpose: "Bounded recent commit history, optionally scoped to a path",
        when_to_use: &["understanding recent evolution of a repo or a specific file"],
        when_not_to_use: &["deep archaeology over thousands of commits (use git log directly)"],
        requires: &[ToolId::Git],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[
            InputSpec { name: "target", kind: InputKind::Path, required: false, description: "path to scope history to" },
            LIMIT,
        ],
        output: "commits[] with sha, subject, author, iso timestamp, refs",
        next: &["repo.state"],
        fallback: None,
    },
    Operation {
        id: "repo.remote",
        domain: "repo",
        cli: "supertools repo remote",
        purpose: "Remote identity: names, URLs, GitHub owner/repo extraction",
        when_to_use: &["checking where a repository points before trusting GitHub features"],
        when_not_to_use: &["changing remotes (read-only in v0.1)"],
        requires: &[ToolId::Git],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "remotes[] with name, url, github flag, owner/repo when GitHub",
        next: &["repo.pr", "repo.ci"],
        fallback: None,
    },
    Operation {
        id: "repo.pr",
        domain: "repo",
        cli: "supertools repo pr",
        purpose: "Compact GitHub PR information for the current branch (via gh JSON)",
        when_to_use: &["checking PR state/review/CI rollup for the checked-out branch"],
        when_not_to_use: &["creating, merging or closing PRs (never in v0.1)"],
        requires: &[ToolId::Git, ToolId::Gh],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "PR number, title, state, branch names, review decision, bounded status rollup",
        next: &["repo.ci"],
        fallback: Some("requires gh + GitHub remote + authentication; otherwise reported cleanly"),
    },
    Operation {
        id: "repo.ci",
        domain: "repo",
        cli: "supertools repo ci [--limit N]",
        purpose: "Compact recent GitHub Actions run states (via gh JSON)",
        when_to_use: &["checking whether CI is green for recent branches"],
        when_not_to_use: &["re-running or cancelling workflows (not in v0.1)"],
        requires: &[ToolId::Git, ToolId::Gh],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[LIMIT],
        output: "runs[] with workflow name, status, conclusion, branch, url",
        next: &["repo.pr"],
        fallback: Some("requires gh + GitHub remote + authentication; otherwise reported cleanly"),
    },
    // ---------------- verify ----------------
    Operation {
        id: "verify.discover",
        domain: "verify",
        cli: "supertools verify discover",
        purpose: "Discover how THIS repository expects itself to be checked (authority, quick task, full task, declared tasks)",
        when_to_use: &["before verifying an unfamiliar project", "deciding whether verification is even possible here"],
        when_not_to_use: &["running verification (use verify quick/full/task)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "authority (mise/just/package/cargo) with precedence evidence, quick/full mapping, declared task names",
        next: &["verify.quick", "verify.full"],
        fallback: Some("ambiguous authorities fail closed and explain how to disambiguate via .supertools.toml"),
    },
    Operation {
        id: "verify.quick",
        domain: "verify",
        cli: "supertools verify quick [--timeout SECS]",
        purpose: "Run the cheapest repository-authoritative verification with meaningful feedback",
        when_to_use: &["fast feedback loops while iterating"],
        when_not_to_use: &["final acceptance (use verify.full)"],
        requires: &[],
        read_only: false,
        destructive: false,
        idempotent: true,
        inputs: &[InputSpec { name: "--timeout", kind: InputKind::Number, required: false, description: "kill the verification after N seconds" }],
        output: "authority, task, command vector, duration, exit state, bounded diagnostics tail",
        next: &["verify.full"],
        fallback: None,
    },
    Operation {
        id: "verify.full",
        domain: "verify",
        cli: "supertools verify full [--timeout SECS]",
        purpose: "Run the strongest locally appropriate repository-authoritative verification suite",
        when_to_use: &["before declaring work complete", "final local gate before push/review"],
        when_not_to_use: &["tight iteration loops (use verify.quick)"],
        requires: &[],
        read_only: false,
        destructive: false,
        idempotent: true,
        inputs: &[InputSpec { name: "--timeout", kind: InputKind::Number, required: false, description: "kill the verification after N seconds" }],
        output: "authority, task, command vector, duration, exit state, bounded diagnostics tail",
        next: &["repo.changed"],
        fallback: None,
    },
    Operation {
        id: "verify.task",
        domain: "verify",
        cli: "supertools verify task <name> [--timeout SECS]",
        purpose: "Run one DECLARED project task by exact name (never an arbitrary command string)",
        when_to_use: &["running a specific discovered task, e.g. one listed by verify.discover"],
        when_not_to_use: &["executing arbitrary commands (refused by design)"],
        requires: &[],
        read_only: false,
        destructive: false,
        idempotent: false,
        inputs: &[InputSpec { name: "name", kind: InputKind::Name, required: true, description: "must resolve to a discovered declared task name" }],
        output: "same evidence as verify.quick",
        next: &["verify.discover"],
        fallback: None,
    },
    // ---------------- tools ----------------
    Operation {
        id: "tools.list",
        domain: "tools",
        cli: "supertools tools",
        purpose: "Compact inventory of the recommended developer toolset with discovered state",
        when_to_use: &["answering 'what developer tools does this machine have?'"],
        when_not_to_use: &["deep per-tool investigation (use find <tool>)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "tools[] with explicit status/version/recommended/required/path-visibility fields",
        next: &["tools.show", "capabilities", "doctor"],
        fallback: None,
    },
    Operation {
        id: "tools.show",
        domain: "tools",
        cli: "supertools tools show <tool>",
        purpose: "Detailed registry record for one tool (status, path, version, discovery source, PATH membership, install route)",
        when_to_use: &["inspecting one tool's real state and provenance"],
        when_not_to_use: &["inventory overviews (use tools)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[InputSpec { name: "tool", kind: InputKind::Name, required: true, description: "tool id, e.g. rg, ast-grep, sg (alias)" }],
        output: "full ToolRecord incl. alternates, notes, cause/action guidance",
        next: &["find.tool", "tools.fix-path"],
        fallback: None,
    },
    Operation {
        id: "tools.filters",
        domain: "tools",
        cli: "supertools tools available|hidden|missing",
        purpose: "Filter the registry by state",
        when_to_use: &["listing what is usable / installed-but-invisible / absent"],
        when_not_to_use: &[],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "filtered tools[] with the same stable schema",
        next: &["tools.fix-path", "tools.install-missing"],
        fallback: None,
    },
    Operation {
        id: "tools.install-missing",
        domain: "tools",
        cli: "supertools tools install-missing [--yes]",
        purpose: "Interactively install missing recommended tools via trusted existing package managers, then verify by re-discovery",
        when_to_use: &["bringing a machine up to the recommended toolset, with user consent"],
        when_not_to_use: &["silent automation without --yes (installation always requires explicit consent)"],
        requires: &[],
        read_only: false,
        destructive: false,
        idempotent: true,
        inputs: &[InputSpec { name: "--yes", kind: InputKind::Flag, required: false, description: "unattended: skip per-tool confirmation prompts" }],
        output: "per-tool install outcome: route, manager exit, verified executable path, PATH visibility",
        next: &["tools.fix-path", "tools.list"],
        fallback: Some("tools without a trustworthy route get guidance, never arbitrary downloads"),
    },
    Operation {
        id: "tools.fix-path",
        domain: "tools",
        cli: "supertools tools fix-path [--dry-run] [--yes]",
        purpose: "Repair the persisted USER PATH so hidden tools become visible to new processes (additive only)",
        when_to_use: &["tools report status=hidden and their directory is not on any persisted PATH"],
        when_not_to_use: &["machine-wide PATH changes (never performed)", "removing entries (never performed)"],
        requires: &[],
        read_only: false,
        destructive: false,
        idempotent: true,
        inputs: &[
            InputSpec { name: "--dry-run", kind: InputKind::Flag, required: false, description: "plan without writing" },
            InputSpec { name: "--yes", kind: InputKind::Flag, required: false, description: "apply without interactive confirmation" },
        ],
        output: "plan (dirs to add), applied state, verification re-read, restart-required notice",
        next: &["tools.hidden", "doctor"],
        fallback: Some("Windows-only in v0.1"),
    },
    Operation {
        id: "find.tool",
        domain: "tools",
        cli: "supertools find [tool] [--install] [--fix-path] [--dry-run] [--yes]",
        purpose: "Deep Windows-first investigation: where is a tool actually installed, why is it invisible, what can be done",
        when_to_use: &[
            "a tool seems missing but might be installed off-PATH",
            "diagnosing stale inherited PATH state",
        ],
        when_not_to_use: &["quick inventory (use tools)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[
            InputSpec { name: "tool", kind: InputKind::Name, required: false, description: "single tool to investigate; omit for the whole toolset" },
            InputSpec { name: "--install", kind: InputKind::Flag, required: false, description: "offer/perform installation of one missing tool (needs consent or --yes)" },
            InputSpec { name: "--fix-path", kind: InputKind::Flag, required: false, description: "USER PATH repair workflow" },
        ],
        output: "per-tool discovery detail: candidates, sources, PATH membership, likely cause, action",
        next: &["tools.fix-path", "tools.install-missing"],
        fallback: None,
    },
    // ---------------- common ----------------
    Operation {
        id: "common.doctor",
        domain: "common",
        cli: "supertools doctor",
        purpose: "Answer 'is anything actually wrong?' — health classification plus repair guidance, never an inventory dump",
        when_to_use: &["first contact with a machine", "after environment changes"],
        when_not_to_use: &["listing tools (use tools)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "overall health (healthy/healthy_with_optional_gaps/degraded/broken), checks, optional improvements",
        next: &["tools.list", "capabilities", "find.tool"],
        fallback: None,
    },
    Operation {
        id: "common.capabilities",
        domain: "common",
        cli: "supertools capabilities",
        purpose: "Answer 'what can Supertools actually do right now?' — per-capability readiness with backend identity",
        when_to_use: &["deciding which operations are usable before planning work"],
        when_not_to_use: &["checking whether a binary exists (use tools)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[],
        output: "capabilities[] with id, status (ready/degraded/unavailable), backend, missing tools, note",
        next: &["common.describe", "doctor"],
        fallback: None,
    },
    Operation {
        id: "common.describe",
        domain: "common",
        cli: "supertools describe [domain|operation-id]",
        purpose: "Factual, schema-oriented description of operations, generated from the internal catalogue",
        when_to_use: &["learning exact invocation shape, inputs, outputs, safety flags of an operation"],
        when_not_to_use: &["learning workflow progression (use teach)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[InputSpec { name: "target", kind: InputKind::Topic, required: false, description: "domain name or operation id; omit for everything" }],
        output: "operation records: id, cli, requires, read_only, destructive, idempotent, inputs, output, next, fallback",
        next: &["common.teach"],
        fallback: None,
    },
    Operation {
        id: "common.teach",
        domain: "common",
        cli: "supertools teach <search|repo|verify|tools|install> [--target codex|opencode] [--apply]",
        purpose: "Compact operational guidance for an AI agent, generated from the catalogue; `teach install` emits the tiny persistent AGENTS.md instruction block",
        when_to_use: &["learning the preferred progression through a domain in a few lines"],
        when_not_to_use: &["full reference (use describe)"],
        requires: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        inputs: &[InputSpec { name: "topic", kind: InputKind::Topic, required: true, description: "domain name, or 'install' for the AGENTS.md snippet/managed-block flow" }],
        output: "compact guidance text; for install: snippet + target paths (+ optional managed-block write with --apply)",
        next: &["common.describe", "capabilities"],
        fallback: None,
    },
];

/// Stable exit-code contract (documented in README and `describe`).
pub static EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success — operation completed with a meaningful result"),
    ("1", "execution/verification failure (includes subprocess timeout)"),
    ("2", "invalid or refused request (bad arguments, unknown target, ambiguous authority, arbitrary-command refusal)"),
    ("3", "capability unavailable (required backend missing, or environment too degraded for doctor)"),
    ("4", "operation completed but no matches / no applicable result"),
];

pub fn find_operation(id: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|o| o.id == id)
}

pub fn operations_for_domain(domain: &str) -> Vec<&'static Operation> {
    OPERATIONS.iter().filter(|o| o.domain == domain).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_are_stable_and_unique() {
        let mut ids: Vec<&str> = OPERATIONS.iter().map(|o| o.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate operation ids");
        // The v0.1 contract — changing this list is a breaking change.
        for expected in [
            "search.text",
            "search.files",
            "search.context",
            "search.symbol",
            "search.structural",
            "repo.state",
            "repo.changed",
            "repo.history",
            "repo.remote",
            "repo.pr",
            "repo.ci",
            "verify.discover",
            "verify.quick",
            "verify.full",
            "verify.task",
            "tools.list",
            "tools.show",
            "tools.filters",
            "tools.install-missing",
            "tools.fix-path",
            "find.tool",
            "common.doctor",
            "common.capabilities",
            "common.describe",
            "common.teach",
        ] {
            assert!(ids.contains(&expected), "missing operation id {expected}");
        }
    }

    #[test]
    fn domains_are_covered() {
        for o in OPERATIONS {
            assert!(DOMAINS.contains(&o.domain), "{} has unknown domain", o.id);
        }
    }

    #[test]
    fn search_and_repo_are_read_only() {
        for o in OPERATIONS
            .iter()
            .filter(|o| o.domain == "search" || o.domain == "repo")
        {
            assert!(o.read_only, "{} must be read-only in v0.1", o.id);
            assert!(!o.destructive, "{} must not be destructive", o.id);
        }
    }

    #[test]
    fn mutating_operations_are_declared() {
        for o in OPERATIONS.iter().filter(|o| !o.read_only) {
            assert!(
                matches!(
                    o.id,
                    "verify.quick"
                        | "verify.full"
                        | "verify.task"
                        | "tools.install-missing"
                        | "tools.fix-path"
                ),
                "unexpected non-read-only operation {}",
                o.id
            );
        }
    }
}
