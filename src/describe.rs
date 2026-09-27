//! `supertools describe` — factual, schema-oriented views generated from the
//! operation catalogue (one semantic authority).

use serde_json::json;

use crate::catalogue::{self, DOMAINS, EXIT_CODES, OPERATIONS};
use crate::output::{Builder, CmdResult, Failure};

pub const STATUSES: &[(&str, &str)] = &[
    ("ok", "operation completed with a meaningful result"),
    (
        "no_results",
        "completed correctly; no matches / nothing applicable",
    ),
    ("failed", "underlying execution failed"),
    (
        "timeout",
        "bounded subprocess was killed after exceeding its timeout",
    ),
    ("invalid_request", "bad arguments or unknown target"),
    (
        "capability_unavailable",
        "required backend is missing in this environment",
    ),
    (
        "ambiguous",
        "conflicting authorities; Supertools refuses to guess",
    ),
    (
        "refused",
        "explicit safety refusal (e.g. arbitrary command strings)",
    ),
];

pub fn describe_cmd(target: Option<String>) -> CmdResult {
    let operation = "describe";
    match target.as_deref() {
        None => {
            let mut b = Builder::ok(operation, format!("{} operations across {} domains", OPERATIONS.len(), DOMAINS.len()))
                .data(json!({
                    "domains": DOMAINS,
                    "operations": OPERATIONS.iter().map(|o| json!({
                        "id": o.id, "domain": o.domain, "cli": o.cli,
                        "purpose": o.purpose, "requires": o.requires,
                        "read_only": o.read_only, "destructive": o.destructive,
                    })).collect::<Vec<_>>(),
                    "exit_codes": exit_codes_json(),
                    "statuses": statuses_json(),
                }));
            b = b.line("SUPERTOOLS OPERATIONS").line("");
            for domain in DOMAINS {
                b = b.line(format!("[{domain}]"));
                for o in catalogue::operations_for_domain(domain) {
                    b = b.line(format!("  {:<22} {}", o.id, o.purpose));
                    b = b.line(format!("  {:<22} {}", "", o.cli));
                }
                b = b.line("");
            }
            b = b.line("EXIT CODES");
            for (code, meaning) in EXIT_CODES {
                b = b.line(format!("  {code}  {meaning}"));
            }
            b = b.line("");
            b = b.line("Details: supertools describe <operation-id>   Guidance: supertools teach <domain>");
            Ok(b)
        }
        Some("exit-codes") | Some("exit_codes") => Ok(Builder::ok(operation, "exit-code contract")
            .data(json!({ "exit_codes": exit_codes_json(), "statuses": statuses_json() }))
            .lines(EXIT_CODES.iter().map(|(c, m)| format!("{c}  {m}")))),
        Some("schema") => Ok(Builder::ok(operation, "canonical JSON envelope schema")
            .data(json!({
                "envelope": {
                    "schema_version": 1,
                    "tool": "supertools",
                    "version": "<crate version>",
                    "operation": "<stable operation id>",
                    "ok": "<bool: operation succeeded (true for status ok|no_results)>",
                    "status": "<ok|no_results|failed|timeout|invalid_request|capability_unavailable|ambiguous|refused>",
                    "summary": "<compact one-line summary>",
                    "data": "<typed per-operation payload>",
                    "evidence": "<[{kind: command|file|fact, ...}]>",
                    "warnings": "<[string]>",
                    "next_actions": "<[{command, reason}]>",
                    "truncated": "<bool: output was bounded>",
                },
                "exit_codes": exit_codes_json(),
                "statuses": statuses_json(),
            }))
            .line("Every --json response is one envelope; see data field above.")
            .line("stdout in JSON mode contains ONLY the envelope — no banners, no ANSI.")),
        Some(t) => {
            if DOMAINS.contains(&t) {
                let ops = catalogue::operations_for_domain(t);
                let mut b = Builder::ok(operation, format!("domain \"{t}\": {} operation(s)", ops.len()))
                    .data(json!({
                        "domain": t,
                        "operations": ops.iter().map(|o| serde_json::to_value(*o).unwrap_or(json!({}))).collect::<Vec<_>>(),
                    }));
                for o in &ops {
                    b = b.line(format!("{} — {}", o.id, o.purpose));
                    b = b.line(format!("  cli: {}", o.cli));
                    b = b.line(format!(
                        "  requires: {}{}{}",
                        if o.requires.is_empty() {
                            "none".to_string()
                        } else {
                            o.requires.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
                        },
                        if o.read_only { "  read-only" } else { "" },
                        if o.destructive { "  DESTRUCTIVE" } else { "" }
                    ));
                }
                b = b.line("").line(format!("Guidance: supertools teach {t}"));
                Ok(b)
            } else if let Some(o) = catalogue::find_operation(t) {
                let mut b = Builder::ok(operation, format!("{} — {}", o.id, o.purpose))
                    .data(serde_json::to_value(o).unwrap_or(json!({})));
                b = b.line(format!("{} — {}", o.id, o.purpose)).line("");
                b = b.line(format!("cli:       {}", o.cli));
                b = b.line(format!("domain:    {}", o.domain));
                b = b.line(format!(
                    "requires:  {}",
                    if o.requires.is_empty() {
                        "none".into()
                    } else {
                        o.requires.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
                    }
                ));
                b = b.line(format!(
                    "safety:    read_only={} destructive={} idempotent={}",
                    o.read_only, o.destructive, o.idempotent
                ));
                if !o.when_to_use.is_empty() {
                    b = b.line("").line("when to use:");
                    for w in o.when_to_use {
                        b = b.line(format!("  - {w}"));
                    }
                }
                if !o.when_not_to_use.is_empty() {
                    b = b.line("when NOT to use:");
                    for w in o.when_not_to_use {
                        b = b.line(format!("  - {w}"));
                    }
                }
                b = b.line("").line(format!("output:    {}", o.output));
                if !o.inputs.is_empty() {
                    b = b.line("inputs:");
                    for i in o.inputs {
                        b = b.line(format!(
                            "  {:<10} {}{} — {}",
                            i.name,
                            serde_json::to_value(i.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                            if i.required { " (required)" } else { "" },
                            i.description
                        ));
                    }
                }
                if let Some(f) = o.fallback {
                    b = b.line("").line(format!("fallback:  {f}"));
                }
                if !o.next.is_empty() {
                    b = b.line(format!("next:      {}", o.next.join(", ")));
                }
                Ok(b)
            } else {
                let mut targets: Vec<&str> = DOMAINS.to_vec();
                targets.push("exit-codes");
                targets.push("schema");
                Err(Failure::invalid(operation, format!("unknown describe target \"{t}\""))
                    .hint(format!("valid targets: {}", targets.join(", ")))
                    .hint(format!(
                        "operation ids: {}",
                        OPERATIONS.iter().map(|o| o.id).collect::<Vec<_>>().join(", ")
                    )))
            }
        }
    }
}

fn exit_codes_json() -> serde_json::Value {
    json!(EXIT_CODES
        .iter()
        .map(|(c, m)| json!({"code": c, "meaning": m}))
        .collect::<Vec<_>>())
}

fn statuses_json() -> serde_json::Value {
    json!(STATUSES
        .iter()
        .map(|(s, m)| json!({"status": s, "meaning": m}))
        .collect::<Vec<_>>())
}
