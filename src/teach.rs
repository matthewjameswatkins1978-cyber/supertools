//! `supertools teach` — compact operational guidance for AI agents,
//! generated from the operation catalogue, plus the tiny persistent
//! AGENTS.md instruction block (`teach install`).

use std::path::PathBuf;

use serde_json::json;

use crate::catalogue;
use crate::output::{Builder, CmdResult, Failure};

pub const BLOCK_BEGIN: &str =
    "<!-- supertools:begin (managed block — content outside these markers is never touched) -->";
pub const BLOCK_END: &str = "<!-- supertools:end -->";

pub const AGENT_INSTRUCTION: &str = "\
Supertools is installed locally.

Before manually constructing repository search, Git-state,
or verification command sequences, prefer:

    supertools search
    supertools repo
    supertools verify

Tool discovery and environment health:

    supertools tools
    supertools doctor

If unfamiliar with an operation, run:

    supertools teach <domain>
    supertools describe <operation>

Fall back to the native underlying tool when Supertools does
not support the required operation.";

pub fn managed_block() -> String {
    format!("{BLOCK_BEGIN}\n{AGENT_INSTRUCTION}\n{BLOCK_END}")
}

/// Domain progressions: the preferred route through each domain, compact.
fn progression(domain: &str) -> Vec<(&'static str, &'static str)> {
    match domain {
        "search" => vec![
            ("search.files", "locate candidate files by name"),
            ("search.text", "find occurrences (bounded, ignore-aware)"),
            ("search.context", "understand a match in one call"),
            ("search.symbol", "likely definitions (explicit heuristic)"),
            (
                "search.structural",
                "when syntax matters (ast-grep patterns)",
            ),
        ],
        "repo" => vec![
            ("repo.state", "establish repository truth FIRST"),
            ("repo.changed", "what changed (bounded, NUL-safe paths)"),
            ("repo.history", "how the repo got here"),
            ("repo.remote", "where it points; GitHub identity"),
            ("repo.pr", "PR state for this branch (needs gh + auth)"),
            ("repo.ci", "recent CI runs (needs gh + auth)"),
        ],
        "verify" => vec![
            (
                "verify.discover",
                "learn THIS project's verification authority",
            ),
            ("verify.quick", "cheapest authoritative feedback loop"),
            ("verify.full", "strongest local gate before declaring done"),
            ("verify.task", "run one declared task by exact name"),
        ],
        "tools" => vec![
            ("tools.list", "inventory of the recommended toolset"),
            ("tools.show", "one tool's real state and provenance"),
            ("tools.filters", "available / hidden / missing views"),
            ("find.tool", "deep investigation: where/why/action"),
            (
                "tools.install-missing",
                "consent-gated installation + verification",
            ),
            ("tools.fix-path", "additive USER PATH repair (Windows)"),
        ],
        _ => vec![],
    }
}

fn domain_extra_guidance(domain: &str) -> Vec<&'static str> {
    match domain {
        "search" => vec![
            "All search operations are read-only and bounded; truncation is always explicit.",
            "Plain text vs structure: use search.text for literals/regex; search.structural when syntax is consequential.",
            "If Threadmoth is installed, use it for guarded structural/source-preserving mutation workflows — Supertools search is for fast discovery only.",
            "Native rg/fd/ast-grep remain directly usable; Supertools adds bounds, schemas and evidence, not lock-in.",
        ],
        "repo" => vec![
            "All repo operations are read-only in v0.1: they never stage, commit, merge, rebase, reset, push or mutate git config.",
            "repo.state replaces five git calls: branch, HEAD, upstream, ahead/behind, dirty counts, in-progress operation.",
            "Native git remains the authority for anything Supertools does not expose.",
        ],
        "verify" => vec![
            "Supertools never invents verification truth: it discovers the project's own authority (mise > just > package scripts > cargo semantics, or explicit .supertools.toml).",
            "Ambiguous authorities fail closed (exit 2) rather than guessing; declare the canonical one in .supertools.toml.",
            "verify task <name> only runs DECLARED task names — arbitrary command strings are refused.",
            "Diagnostics are bounded; failure tails are preserved and truncation is explicit.",
        ],
        "tools" => vec![
            "One Tool Registry feeds tools/find/doctor/capabilities — statuses are explicit: available, hidden, missing, ambiguous, broken.",
            "hidden ≠ missing: installed but invisible to this process. Cause is usually a stale inherited PATH or a directory missing from USER PATH.",
            "Installation always requires consent (interactive prompt or --yes) and is verified by re-discovery, never by manager exit code alone.",
            "PATH repair only ever ADDS directories to the persisted USER PATH; machine PATH is read-only; existing processes need a restart.",
        ],
        _ => vec![],
    }
}

pub fn teach_cmd(
    topic: &str,
    target: Option<String>,
    path: Option<PathBuf>,
    apply: bool,
) -> CmdResult {
    match topic {
        "install" => teach_install(target, path, apply),
        "search" | "repo" | "verify" | "tools" => teach_domain(topic),
        other => Err(
            Failure::invalid("teach", format!("unknown teach topic \"{other}\""))
                .hint("topics: search, repo, verify, tools, install"),
        ),
    }
}

fn teach_domain(domain: &str) -> CmdResult {
    let operation = format!("teach.{domain}");
    let ops: Vec<&crate::catalogue::Operation> = progression(domain)
        .iter()
        .filter_map(|(id, _)| catalogue::find_operation(id))
        .collect();

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("SUPERTOOLS TEACH — {}", domain.to_uppercase()));
    lines.push(String::new());
    lines.push("Preferred progression:".to_string());
    for (i, (id, why)) in progression(domain).iter().enumerate() {
        lines.push(format!("  {}. {:<20} {}", i + 1, id, why));
    }
    lines.push(String::new());
    lines.push("Key facts:".to_string());
    for g in domain_extra_guidance(domain) {
        lines.push(format!("  - {g}"));
    }
    lines.push(String::new());
    lines.push("Invocation shapes:".to_string());
    for o in &ops {
        lines.push(format!("  {}", o.cli));
        let req: Vec<&str> = o.requires.iter().map(|r| r.as_str()).collect();
        let reqs = if req.is_empty() {
            "nothing".to_string()
        } else {
            req.join(", ")
        };
        lines.push(format!(
            "      needs: {reqs} · read_only={} · {}",
            o.read_only, o.output
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "Full schemas: supertools describe {domain} · machine view: add --json"
    ));

    let line_count = lines.len();
    let mut b = Builder::ok(
        &operation,
        format!("compact {domain} guidance ({line_count} lines)"),
    )
    .data(json!({
        "domain": domain,
        "progression": progression(domain).iter().map(|(id, why)| json!({"id": id, "why": why})).collect::<Vec<_>>(),
        "operations": ops.iter().map(|o| json!({"id": o.id, "cli": o.cli, "requires": o.requires, "read_only": o.read_only})).collect::<Vec<_>>(),
        "guidance": domain_extra_guidance(domain),
        "fallback": "native underlying tools remain directly usable",
    }))
    .lines(lines);
    b = b.next(
        format!("supertools describe {domain}"),
        "factual schema-oriented detail for every operation",
    );
    Ok(b)
}

#[derive(Debug, serde::Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockAction {
    Create,
    Append,
    Replace,
    Unchanged,
    WouldCreate,
    WouldAppend,
    WouldReplace,
}

/// Idempotent managed-block upsert. Content outside markers is preserved
/// byte-for-byte. Pure function — unit tested.
pub fn upsert_block(
    existing: Option<&str>,
    block: &str,
    apply: bool,
) -> (Option<String>, BlockAction) {
    match existing {
        None => (
            apply.then(|| format!("{block}\n")),
            if apply {
                BlockAction::Create
            } else {
                BlockAction::WouldCreate
            },
        ),
        Some(text) => {
            if let (Some(begin), Some(end)) = (text.find(BLOCK_BEGIN), text.find(BLOCK_END)) {
                if end >= begin {
                    let replaced = format!(
                        "{}{}{}",
                        &text[..begin],
                        block,
                        &text[end + BLOCK_END.len()..]
                    );
                    if replaced == text {
                        return (None, BlockAction::Unchanged);
                    }
                    return (
                        apply.then_some(replaced),
                        if apply {
                            BlockAction::Replace
                        } else {
                            BlockAction::WouldReplace
                        },
                    );
                }
            }
            let sep = if text.ends_with('\n') { "\n" } else { "\n\n" };
            let appended = format!("{text}{sep}{block}\n");
            (
                apply.then_some(appended),
                if apply {
                    BlockAction::Append
                } else {
                    BlockAction::WouldAppend
                },
            )
        }
    }
}

fn known_targets() -> Vec<(&'static str, PathBuf)> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    vec![
        ("codex", home.join(".codex").join("AGENTS.md")),
        (
            "opencode",
            home.join(".config").join("opencode").join("AGENTS.md"),
        ),
    ]
}

fn teach_install(target: Option<String>, path: Option<PathBuf>, apply: bool) -> CmdResult {
    let operation = "teach.install";
    let block = managed_block();

    let targets: Vec<(String, PathBuf)> = if let Some(p) = path {
        vec![("custom".to_string(), p)]
    } else if let Some(t) = &target {
        match t.as_str() {
            "stdout" => vec![],
            other => {
                let found = known_targets().into_iter().find(|(n, _)| *n == other);
                match found {
                    Some((n, p)) => vec![(n.to_string(), p)],
                    None => {
                        return Err(Failure::invalid(
                            operation,
                            format!("unknown teach install target \"{other}\""),
                        )
                        .hint("targets: codex, opencode, stdout (or --path FILE)"))
                    }
                }
            }
        }
    } else {
        known_targets()
            .into_iter()
            .map(|(n, p)| (n.to_string(), p))
            .collect()
    };

    // stdout-only mode: just emit the instruction block.
    if targets.is_empty() {
        return Ok(
            Builder::ok(operation, "persistent agent instruction block (stdout)")
                .data(json!({ "block": block, "targets": [], "applied": false }))
                .lines(block.lines().map(str::to_string)),
        );
    }

    let mut results = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut applied_any = false;

    for (name, p) in &targets {
        let existing = std::fs::read_to_string(p).ok();
        let file_exists = p.exists();
        let (new_content, action) = upsert_block(existing.as_deref(), &block, apply);
        if let Some(content) = new_content {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Failure::failed(
                        operation,
                        format!("cannot create {}: {e}", parent.display()),
                    )
                })?;
            }
            std::fs::write(p, content).map_err(|e| {
                Failure::failed(operation, format!("cannot write {}: {e}", p.display()))
            })?;
            applied_any = true;
        }
        results.push(json!({
            "target": name,
            "path": p.to_string_lossy(),
            "existed": file_exists,
            "action": action,
        }));
        lines.push(format!("{name}: {} → {:?}", p.display(), action));
    }

    let mut b = Builder::ok(
        operation,
        if apply {
            format!("managed block written ({} target(s))", targets.len())
        } else {
            "preview only — nothing written (use --apply to write the managed block)".to_string()
        },
    )
    .data(json!({
        "block": block,
        "targets": results,
        "applied": applied_any,
        "idempotent": true,
        "preserves_unrelated_content": true,
    }))
    .lines(lines);

    b = b.line("");
    b = b.lines(AGENT_INSTRUCTION.lines().map(str::to_string));
    if !apply {
        b = b.line("");
        b = b.line("This is the ENTIRE persistent instruction — it stays small by design.");
        b = b.line("Apply with: supertools teach install --apply   (managed block, idempotent, preserves other content)");
    }
    Ok(b)
}

/// Teach output must stay compact; this guard keeps the promise testable.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_creates_when_absent() {
        let block = managed_block();
        let (out, action) = upsert_block(None, &block, true);
        assert_eq!(action, BlockAction::Create);
        assert!(out.unwrap().contains(BLOCK_BEGIN));
    }

    #[test]
    fn upsert_appends_and_preserves() {
        let block = managed_block();
        let existing = "# My Agent Notes\n\nDo important things.\n";
        let (out, action) = upsert_block(Some(existing), &block, true);
        assert_eq!(action, BlockAction::Append);
        let out = out.unwrap();
        assert!(out.starts_with("# My Agent Notes"));
        assert!(out.contains("Do important things."));
        assert!(out.contains(BLOCK_BEGIN));
    }

    #[test]
    fn upsert_is_idempotent() {
        let block = managed_block();
        let (first, _) = upsert_block(None, &block, true);
        let first = first.unwrap();
        let (second, action) = upsert_block(Some(&first), &block, true);
        assert_eq!(action, BlockAction::Unchanged);
        assert!(second.is_none());
    }

    #[test]
    fn upsert_replaces_only_block() {
        let old_block = format!("{BLOCK_BEGIN}\nOLD CONTENT\n{BLOCK_END}");
        let existing = format!("before\n{old_block}\nafter\n");
        let (out, action) = upsert_block(Some(&existing), &managed_block(), true);
        assert_eq!(action, BlockAction::Replace);
        let out = out.unwrap();
        assert!(out.starts_with("before\n"));
        assert!(out.ends_with("\nafter\n"));
        assert!(!out.contains("OLD CONTENT"));
        assert!(out.contains(AGENT_INSTRUCTION));
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (out, action) = upsert_block(Some("keep me\n"), &managed_block(), false);
        assert_eq!(action, BlockAction::WouldAppend);
        assert!(out.is_none());
    }

    #[test]
    fn teach_domains_are_compact_and_canonical() {
        for domain in ["search", "repo", "verify", "tools"] {
            let b = teach_domain(domain).unwrap();
            assert!(
                b.human.len() < 60,
                "{domain} teaching too long: {} lines",
                b.human.len()
            );
            let text = b.human.join("\n");
            assert!(
                text.contains("supertools"),
                "{domain} missing canonical commands"
            );
            assert!(
                text.contains("Preferred progression"),
                "{domain} missing progression"
            );
        }
    }
}
