//! Capability model: "what can Supertools actually do right now?"
//!
//! Derived from the Tool Registry — capabilities reason about operations and
//! backends, never about raw "binary exists" facts.

use serde::Serialize;

use crate::registry::{
    env::{current_platform, EnvFacts},
    install, ToolId, ToolRegistry, ToolStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapStatus {
    Ready,
    /// Works, but through a weaker fallback backend.
    Degraded,
    Unavailable,
}

impl CapStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CapStatus::Ready => "ready",
            CapStatus::Degraded => "degraded",
            CapStatus::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityView {
    pub id: &'static str,
    pub status: CapStatus,
    pub backend: Option<String>,
    pub missing_tools: Vec<ToolId>,
    pub note: Option<String>,
}

fn tool_backend(
    reg: &ToolRegistry,
    id: ToolId,
    label: &str,
) -> (CapStatus, Option<String>, Vec<ToolId>) {
    match reg.status(id) {
        ToolStatus::Available => (CapStatus::Ready, Some(label.to_string()), vec![]),
        ToolStatus::Hidden => (CapStatus::Unavailable, None, vec![id]),
        _ => (CapStatus::Unavailable, None, vec![id]),
    }
}

/// Derive the full capability table from one registry snapshot.
pub fn derive(reg: &ToolRegistry, facts: &EnvFacts) -> Vec<CapabilityView> {
    let mut caps = Vec::new();
    let mut push = |id: &'static str,
                    status: CapStatus,
                    backend: Option<String>,
                    missing: Vec<ToolId>,
                    note: Option<String>| {
        caps.push(CapabilityView {
            id,
            status,
            backend,
            missing_tools: missing,
            note,
        });
    };

    // ---- search ----
    let (st, sb, sm) = tool_backend(reg, ToolId::Rg, "rg");
    if st == CapStatus::Ready {
        push("search.text", st, sb, sm, None);
    } else {
        push(
            "search.text",
            CapStatus::Degraded,
            Some("internal-fallback".into()),
            sm,
            Some("rg unavailable: bounded internal fixed-substring scan only; regex queries unavailable".into()),
        );
    }

    match reg.status(ToolId::Fd) {
        ToolStatus::Available => push("search.files", CapStatus::Ready, Some("fd".into()), vec![], None),
        _ => match reg.status(ToolId::Git) {
            ToolStatus::Available => push(
                "search.files",
                CapStatus::Degraded,
                Some("git-ls-files".into()),
                vec![ToolId::Fd],
                Some("preferred backend fd is unavailable; bounded git ls-files fallback (substring match)".into()),
            ),
            _ => push(
                "search.files",
                CapStatus::Unavailable,
                None,
                vec![ToolId::Fd, ToolId::Git],
                Some("neither fd nor git available for bounded file discovery".into()),
            ),
        },
    }

    let (ct, cb, cm) = tool_backend(reg, ToolId::Rg, "rg");
    if ct == CapStatus::Ready {
        push("search.context", ct, cb, cm, None);
    } else {
        push(
            "search.context",
            CapStatus::Degraded,
            Some("internal-fallback".into()),
            cm,
            Some("rg unavailable: internal fixed-substring scan with bounded context".into()),
        );
    }

    let (sy, syb, sym) = tool_backend(reg, ToolId::Rg, "rg (heuristic)");
    push(
        "search.symbol",
        sy,
        syb,
        sym,
        Some("explicitly heuristic definition-pattern search, not semantic resolution".into()),
    );

    let (ss, ssb, ssm) = tool_backend(reg, ToolId::AstGrep, "ast-grep");
    push(
        "search.structural",
        ss,
        ssb,
        ssm,
        if ss == CapStatus::Unavailable {
            Some("suggest search.text only where text is semantically sufficient".into())
        } else {
            None
        },
    );

    // ---- repo ----
    for cap in ["repo.state", "repo.changed", "repo.history", "repo.remote"] {
        let (t, b, m) = tool_backend(reg, ToolId::Git, "git");
        push(cap, t, b, m, None);
    }
    let gh_ready = reg.status(ToolId::Gh) == ToolStatus::Available;
    let git_ready = reg.status(ToolId::Git) == ToolStatus::Available;
    for cap in ["repo.pr", "repo.ci"] {
        if gh_ready && git_ready {
            push(
                cap,
                CapStatus::Ready,
                Some("gh".into()),
                vec![],
                Some("still requires a GitHub remote and gh authentication at call time".into()),
            );
        } else {
            let mut missing = Vec::new();
            if !git_ready {
                missing.push(ToolId::Git);
            }
            if !gh_ready {
                missing.push(ToolId::Gh);
            }
            push(cap, CapStatus::Unavailable, None, missing, None);
        }
    }

    // ---- verify ----
    let (vt, vb, vm) = tool_backend(reg, ToolId::Mise, "mise");
    push("verify.mise", vt, vb, vm, None);
    let (vt, vb, vm) = tool_backend(reg, ToolId::Just, "just");
    push("verify.just", vt, vb, vm, None);
    let (vt, vb, vm) = tool_backend(reg, ToolId::Cargo, "cargo");
    push("verify.cargo", vt, vb, vm, None);

    // Package-script verification: runners outside the recommended registry.
    // Shares verify's canonical runner preference order (one source of truth).
    let runner = crate::verify::first_runner_on_path();
    match runner {
        Some(r) => push(
            "verify.package",
            CapStatus::Ready,
            Some(r.to_string()),
            vec![],
            None,
        ),
        None => push(
            "verify.package",
            CapStatus::Unavailable,
            None,
            vec![],
            Some("no npm/pnpm/yarn/bun runner found on PATH".into()),
        ),
    }

    // Discovery itself always works; running tasks depends on the authority.
    push(
        "verify.discover",
        CapStatus::Ready,
        Some("project files".into()),
        vec![],
        None,
    );

    // ---- tools / platform ----
    let windows = current_platform() == crate::registry::env::Platform::Windows;
    push(
        "tools.discovery",
        if windows {
            CapStatus::Ready
        } else {
            CapStatus::Degraded
        },
        Some(
            if windows {
                "windows-discovery"
            } else {
                "shallow-path-discovery"
            }
            .into(),
        ),
        vec![],
        if windows {
            None
        } else {
            Some("deep discovery and PATH repair are Windows-first in v0.1".into())
        },
    );

    let managers = install::probe_managers(facts);
    let any_manager = managers.winget.is_some()
        || managers.choco.is_some()
        || managers.scoop.is_some()
        || managers.cargo.is_some()
        || managers.npm.is_some();
    push(
        "tools.install",
        if any_manager {
            CapStatus::Ready
        } else {
            CapStatus::Unavailable
        },
        managers
            .winget
            .as_ref()
            .map(|_| "winget")
            .or(managers.cargo.as_ref().map(|_| "cargo"))
            .or(managers.choco.as_ref().map(|_| "choco"))
            .or(managers.scoop.as_ref().map(|_| "scoop"))
            .or(managers.npm.as_ref().map(|_| "npm"))
            .map(|s| s.to_string()),
        vec![],
        Some("installation always requires explicit consent (prompt or --yes)".into()),
    );

    // ---- guidance capability ----
    match reg.status(ToolId::Threadmoth) {
        ToolStatus::Available => push(
            "guidance.threadmoth",
            CapStatus::Ready,
            Some("threadmoth".into()),
            vec![],
            Some("Threadmoth remains a separate tool; Supertools only teaches when it is appropriate".into()),
        ),
        _ => push(
            "guidance.threadmoth",
            CapStatus::Unavailable,
            None,
            vec![ToolId::Threadmoth],
            Some("normal Supertools functionality continues without it".into()),
        ),
    }

    caps
}

/// `supertools capabilities` — what can Supertools actually do right now?
pub fn capabilities_cmd() -> crate::output::CmdResult {
    let operation = "capabilities";
    let facts = EnvFacts::collect(false);
    let reg = ToolRegistry::from_facts(facts.clone());
    let caps = derive(&reg, &facts);

    let ready = caps.iter().filter(|c| c.status == CapStatus::Ready).count();
    let degraded = caps
        .iter()
        .filter(|c| c.status == CapStatus::Degraded)
        .count();
    let unavailable = caps
        .iter()
        .filter(|c| c.status == CapStatus::Unavailable)
        .count();

    let mut b = crate::output::Builder::ok(
        operation,
        format!("{ready} ready, {degraded} degraded, {unavailable} unavailable"),
    )
    .data(serde_json::json!({
        "capabilities": caps.iter().map(|c| serde_json::json!({
            "id": c.id,
            "status": c.status,
            "backend": c.backend,
            "missing_tools": c.missing_tools.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
            "note": c.note,
        })).collect::<Vec<_>>(),
        "summary": { "ready": ready, "degraded": degraded, "unavailable": unavailable },
    }))
    .line("SUPERTOOLS CAPABILITIES")
    .line("");
    for c in &caps {
        b = b.line(format!("{:<22} {}", c.id, c.status.as_str()));
        if let Some(backend) = &c.backend {
            b = b.line(format!("  backend: {backend}"));
        }
        if !c.missing_tools.is_empty() {
            b = b.line(format!(
                "  missing: {}",
                c.missing_tools
                    .iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(n) = &c.note {
            b = b.line(format!("  note: {n}"));
        }
    }
    b = b.line("");
    b = b.line("Reason from capabilities, not from 'binary X exists'. Details: supertools tools");
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_ids_are_stable() {
        let facts = EnvFacts::collect(false);
        let reg = ToolRegistry::from_facts(facts.clone());
        let caps = derive(&reg, &facts);
        let ids: Vec<&str> = caps.iter().map(|c| c.id).collect();
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
            "verify.mise",
            "verify.just",
            "verify.cargo",
            "verify.package",
            "verify.discover",
            "tools.discovery",
            "tools.install",
            "guidance.threadmoth",
        ] {
            assert!(ids.contains(&expected), "missing capability {expected}");
        }
    }
}
