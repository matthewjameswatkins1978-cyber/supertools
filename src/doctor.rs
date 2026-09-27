//! `supertools doctor` — "Is anything actually wrong?"
//! Health classification over the Tool Registry + capability model.
//! Never an inventory dump; never mutates anything.

use serde_json::json;

use crate::capabilities::{self, CapStatus};
use crate::output::{Builder, CmdResult, Status};
use crate::registry::{env::EnvFacts, ToolId, ToolRegistry, ToolStatus};

#[derive(serde::Serialize)]
struct Check {
    id: &'static str,
    status: &'static str, // pass | warn | fail
    label: String,
}

pub fn overall_label(git: bool, rg: bool, gaps: bool) -> &'static str {
    if !git && !rg {
        "broken"
    } else if !git || !rg {
        "degraded"
    } else if gaps {
        "healthy_with_optional_gaps"
    } else {
        "healthy"
    }
}

pub fn doctor_cmd() -> CmdResult {
    let operation = "doctor";
    let facts = EnvFacts::collect(false);
    let reg = ToolRegistry::from_facts(facts.clone());
    let caps = capabilities::derive(&reg, &facts);

    let git_ok = reg.available(ToolId::Git);
    let rg_ok = reg.available(ToolId::Rg);

    let mut checks: Vec<Check> = Vec::new();
    let mut check = |id: &'static str, ok: bool, warn: bool, label: String| {
        checks.push(Check {
            id,
            status: if ok {
                "pass"
            } else if warn {
                "warn"
            } else {
                "fail"
            },
            label,
        });
    };

    check(
        "repo",
        git_ok,
        false,
        if git_ok {
            "Core repository capability available".into()
        } else {
            "Git is not available — repo.* operations are unavailable".into()
        },
    );
    let cap_text = caps.iter().find(|c| c.id == "search.text");
    let text_full = cap_text
        .map(|c| c.status == CapStatus::Ready)
        .unwrap_or(false);
    check(
        "search.text",
        rg_ok || git_ok,
        !text_full,
        if text_full {
            "Text search available (rg)".into()
        } else if rg_ok || git_ok {
            "Text search degraded: internal fixed-substring fallback only (rg missing)".into()
        } else {
            "Text search unavailable (rg and fallback backends missing)".into()
        },
    );
    check(
        "search.structural",
        reg.available(ToolId::AstGrep),
        true,
        if reg.available(ToolId::AstGrep) {
            "Structural search available (ast-grep)".into()
        } else {
            "Structural search unavailable (ast-grep missing)".into()
        },
    );
    check(
        "search.files",
        reg.available(ToolId::Fd) || git_ok,
        true,
        if reg.available(ToolId::Fd) {
            "File discovery available (fd)".into()
        } else if git_ok {
            "File discovery via bounded git fallback (fd missing)".into()
        } else {
            "File discovery unavailable".into()
        },
    );
    check(
        "verify.rust",
        reg.available(ToolId::Cargo),
        true,
        if reg.available(ToolId::Cargo) {
            "Rust verification available (cargo)".into()
        } else {
            "Rust verification unavailable (cargo missing)".into()
        },
    );
    check(
        "github",
        reg.available(ToolId::Gh) && git_ok,
        true,
        if reg.available(ToolId::Gh) && git_ok {
            "GitHub integration available (gh)".into()
        } else {
            "GitHub integration unavailable (gh missing or git missing)".into()
        },
    );
    let runners_ok = reg.available(ToolId::Mise) || reg.available(ToolId::Just);
    check(
        "task-runners",
        runners_ok,
        true,
        if runners_ok {
            "Task-runner authority available (mise/just)".into()
        } else {
            "No task runner (mise/just); verify falls back to other project authorities".into()
        },
    );
    check(
        "threadmoth",
        reg.available(ToolId::Threadmoth),
        true,
        if reg.available(ToolId::Threadmoth) {
            "Threadmoth available (separate guarded tool; guidance enabled)".into()
        } else {
            "Threadmoth not installed (optional)".into()
        },
    );

    let gaps: Vec<serde_json::Value> = reg
        .tools
        .iter()
        .filter(|t| t.status != ToolStatus::Available && !t.required)
        .map(|t| {
            json!({
                "tool": t.id,
                "status": t.status,
                "why_it_matters": t.used_by,
                "action": t.action.clone().or_else(|| {
                    if t.status == ToolStatus::Missing && t.install_available {
                        Some(format!("supertools find {} --install", t.id.as_str()))
                    } else {
                        None
                    }
                }),
            })
        })
        .collect();

    let overall = overall_label(git_ok, rg_ok, !gaps.is_empty());

    // Repository context (read-only), when git works.
    let repo_context = if git_ok {
        match crate::repo::state() {
            Ok(b) => Some(b.data),
            Err(_) => Some(json!({"in_repository": false})),
        }
    } else {
        None
    };

    let status = match overall {
        "healthy" | "healthy_with_optional_gaps" => Status::Ok,
        _ => Status::CapabilityUnavailable,
    };

    let mut b = Builder::new(operation, status, format!("overall: {overall}"))
        .data(json!({
            "overall": overall,
            "supertools_version": env!("CARGO_PKG_VERSION"),
            "platform": reg.platform,
            "checks": checks.iter().map(|c| json!({"id": c.id, "status": c.status, "label": c.label})).collect::<Vec<_>>(),
            "optional_improvements": gaps,
            "repo_context": repo_context,
        }));

    b = b
        .line("SUPERTOOLS DOCTOR")
        .line("")
        .line(format!("Overall: {}", overall.to_uppercase()))
        .line("");
    for c in &checks {
        let mark = match c.status {
            "pass" => "+",
            "warn" => "!",
            _ => "x",
        };
        b = b.line(format!("{mark} {}", c.label));
    }

    let improvements = b.data["optional_improvements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !improvements.is_empty() {
        b = b.line("").line("Optional improvements:").line("");
        for g in &improvements {
            b = b.line(format!(
                "  {}",
                g.get("tool").and_then(|t| t.as_str()).unwrap_or("?")
            ));
            b = b.line(format!(
                "    {}",
                g.get("why_it_matters")
                    .and_then(|w| w.as_str())
                    .unwrap_or("")
            ));
            if let Some(a) = g.get("action").and_then(|a| a.as_str()) {
                b = b.line(format!("    -> {a}"));
            }
        }
    } else {
        b = b.line("").line("No repair is required.");
    }

    if matches!(overall, "degraded" | "broken") {
        b = b
            .next(
                "supertools tools install-missing --yes",
                "install missing core backends",
            )
            .next("supertools find", "investigate tool locations");
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overall_classification() {
        assert_eq!(overall_label(true, true, false), "healthy");
        assert_eq!(
            overall_label(true, true, true),
            "healthy_with_optional_gaps"
        );
        assert_eq!(overall_label(false, true, true), "degraded");
        assert_eq!(overall_label(true, false, false), "degraded");
        assert_eq!(overall_label(false, false, false), "broken");
    }
}
