//! Sartorial presentation adapter — HUMAN surface only.
//!
//! ```text
//!           Supertools semantic truth (Builder / Failure)
//!                          |
//!            +-------------+-------------+
//!            |                           |
//!            v                           v
//!      SARTORIAL (this module)      JSON envelope
//!      attached-TTY human output    untouched agent surface
//! ```
//!
//! Rules enforced here:
//! * `--json` never passes through Sartorial (gated by [`set_json_mode`]).
//! * Piped / non-TTY stdout never passes through Sartorial — the existing
//!   deterministic plain lines are printed instead (view constructors are
//!   never reached, and [`render_builder`] returns false so the caller falls
//!   back to the plain path).
//! * Views only DRESS existing facts from `Builder.data` / `Failure`; they
//!   never re-derive semantics, invent status, or add information.
//! * Exit codes, warnings and next_actions remain owned by the core; main
//!   still prints warnings/next_actions to stderr in human mode.

use std::sync::atomic::{AtomicBool, Ordering};

use sartorial::{
    Action, Config, ErrorModel, ErrorView, MotionMode, Notice, Outcome, Plan, PlanChange, Preset,
    Receipt, RenderContext, RenderTarget, SartorialOutput, Status as VStatus, SummaryScreen,
    TableModel, TableView,
};

use crate::output::{Builder, Failure, Status};

static JSON_MODE: AtomicBool = AtomicBool::new(false);

/// Called once from `main` before dispatch. JSON mode disables every
/// Sartorial surface, including stderr progress lines.
pub fn set_json_mode(json: bool) {
    JSON_MODE.store(json, Ordering::Relaxed);
}

fn json_mode() -> bool {
    JSON_MODE.load(Ordering::Relaxed)
}

/// Sartorial renders only for an attached terminal in human mode.
pub fn enabled() -> bool {
    use std::io::IsTerminal;
    !json_mode() && std::io::stdout().is_terminal()
}

/// House preset, automatic motion. Non-TTY forces the plain target as a
/// second safety net (the primary gate is [`enabled`]).
fn context() -> RenderContext {
    use std::io::IsTerminal;
    let config = Config::default()
        .with_preset(Preset::House)
        .with_motion(MotionMode::Auto);
    let ctx = RenderContext::detect().with_config(config);
    if std::io::stdout().is_terminal() {
        ctx
    } else {
        ctx.with_target(RenderTarget::Plain)
    }
}

/// Presentational status mapping — mirrors the envelope status 1:1 without
/// altering it.
fn envelope_status(s: Status) -> VStatus {
    match s {
        Status::Ok => VStatus::Ready,
        Status::NoResults => VStatus::Skipped,
        Status::Failed | Status::Timeout => VStatus::Failed,
        Status::InvalidRequest
        | Status::CapabilityUnavailable
        | Status::Ambiguous
        | Status::Refused => VStatus::Attention,
    }
}

fn jstr(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("-")
        .to_string()
}

fn jnum(v: &serde_json::Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| x.as_u64())
}

fn jbool(v: &serde_json::Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

fn yes_no(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        _ => "no",
    }
}

fn short_version(rec: &serde_json::Value) -> String {
    rec.get("version")
        .and_then(|v| v.as_str())
        .map(|v| crate::process::head(v, 24).0)
        .unwrap_or_else(|| "-".to_string())
}

// ------------------------------------------------------------------ views
//
// Each constructor dresses one operation's existing data. Returning None
// means "no Sartorial view; the plain deterministic lines are better".

/// Render the human surface for a completed operation. Returns true when a
/// Sartorial view was printed to stdout; false means the caller must print
/// the plain deterministic lines instead.
pub fn render_builder(b: &Builder) -> bool {
    if !enabled() {
        return false;
    }
    let ctx = context();
    let result = match b.operation.as_str() {
        "doctor" => doctor_screen(b).map(|v| SartorialOutput::print_result(&v, &ctx)),
        "tools.list" => tools_screen(b, "Supertools Toolkit", false)
            .map(|v| SartorialOutput::print_result(&v, &ctx)),
        "find" => {
            // An interactive install may have appended receipts to the plain
            // lines; keep those authoritative and stay plain in that case.
            if b.data.get("install_results").is_some() {
                None
            } else {
                tools_screen(b, "Supertools Tool Discovery", true)
                    .map(|v| SartorialOutput::print_result(&v, &ctx))
            }
        }
        "tools.show" | "find.tool" => {
            tool_detail_screen(b).map(|v| SartorialOutput::print_result(&v, &ctx))
        }
        "tools.available" | "tools.hidden" | "tools.missing" => {
            tools_filter_table(b).map(|v| SartorialOutput::print_result(&v, &ctx))
        }
        "tools.install-missing" => {
            install_receipt(b).map(|v| SartorialOutput::print_result(&v, &ctx))
        }
        "tools.fix-path" => fix_path_receipt(b)
            .map(|v| SartorialOutput::print_result(&v, &ctx))
            .or_else(|| fix_path_plan(b).map(|v| SartorialOutput::print_result(&v, &ctx))),
        "verify.discover" => {
            verify_discover_screen(b).map(|v| SartorialOutput::print_result(&v, &ctx))
        }
        "verify.quick" | "verify.full" | "verify.task" => {
            verify_outcome(b).map(|v| SartorialOutput::print_result(&v, &ctx))
        }
        _ => None,
    };
    match result {
        Some(Ok(())) => true,
        Some(Err(_)) => true, // partial output already happened; do not double-print
        None => false,
    }
}

fn doctor_screen(b: &Builder) -> Option<SummaryScreen> {
    let d = &b.data;
    let overall = d.get("overall")?.as_str()?;
    let status = match overall {
        "healthy" | "healthy_with_optional_gaps" => VStatus::Ready,
        "degraded" => VStatus::Attention,
        _ => VStatus::Failed,
    };
    let mut s = SummaryScreen::new("Supertools Doctor", status)
        .with_subtitle(format!("overall: {overall}"))
        .fact("Platform", jstr(d, "platform"))
        .fact("Version", jstr(d, "supertools_version"));
    if let Some(caps) = d.get("capability_summary") {
        s = s.fact(
            "Capabilities",
            format!(
                "{} ready · {} degraded · {} unavailable",
                jnum(caps, "ready").unwrap_or(0),
                jnum(caps, "degraded").unwrap_or(0),
                jnum(caps, "unavailable").unwrap_or(0),
            ),
        );
    }
    if let Some(repo) = d.get("repo_context").filter(|r| r.is_object()) {
        if jbool(repo, "in_repository").unwrap_or(false) {
            let branch = repo
                .get("branch")
                .and_then(|x| x.as_str())
                .unwrap_or("(detached)");
            s = s.fact(
                "Repository",
                format!(
                    "{branch} · {}",
                    if jbool(repo, "dirty").unwrap_or(false) {
                        "dirty"
                    } else {
                        "clean"
                    }
                ),
            );
        }
    }
    if let Some(checks) = d.get("checks").and_then(|c| c.as_array()) {
        let mut t = TableModel::new(vec!["Check", "Status", "Detail"]).with_title("Checks");
        for c in checks {
            t.add_row([jstr(c, "id"), jstr(c, "status"), jstr(c, "label")]);
        }
        s = s.with_table(t);
    }
    if let Some(gaps) = d.get("optional_improvements").and_then(|g| g.as_array()) {
        if !gaps.is_empty() {
            let detail: Vec<String> = gaps
                .iter()
                .map(|g| {
                    let action = g
                        .get("action")
                        .and_then(|a| a.as_str())
                        .map(|a| format!(" -> {a}"))
                        .unwrap_or_default();
                    format!("{}: {}{action}", jstr(g, "tool"), jstr(g, "why_it_matters"))
                })
                .collect();
            s = s.notice(
                Notice::info(format!("{} optional improvement(s)", gaps.len()))
                    .with_detail(detail.join("\n")),
            );
        }
    }
    Some(s)
}

fn tools_screen(b: &Builder, title: &str, with_paths: bool) -> Option<SummaryScreen> {
    let d = &b.data;
    let summary = d.get("summary")?;
    let attention = jnum(summary, "hidden").unwrap_or(0)
        + jnum(summary, "ambiguous").unwrap_or(0)
        + jnum(summary, "broken").unwrap_or(0)
        > 0;
    let mut s = SummaryScreen::new(
        title,
        if attention {
            VStatus::Attention
        } else {
            VStatus::Ready
        },
    )
    .with_subtitle(b.summary.clone())
    .fact("Platform", jstr(d, "platform"));
    let tools = d.get("tools")?.as_array()?;
    let mut t = if with_paths {
        TableModel::new(vec!["Tool", "Status", "Path"])
    } else {
        TableModel::new(vec!["Tool", "Status", "Version", "Role"])
    };
    for rec in tools {
        if with_paths {
            t.add_row([
                jstr(rec, "id"),
                jstr(rec, "status"),
                rec.get("path")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string(),
            ]);
        } else {
            t.add_row([
                jstr(rec, "id"),
                jstr(rec, "status"),
                short_version(rec),
                jstr(rec, "role"),
            ]);
        }
    }
    s = s.with_table(t);
    Some(s)
}

fn tool_detail_screen(b: &Builder) -> Option<SummaryScreen> {
    let d = &b.data;
    let status_str = d.get("status")?.as_str()?;
    let status = match status_str {
        "available" => VStatus::Ready,
        "hidden" | "ambiguous" | "missing" => VStatus::Attention,
        _ => VStatus::Failed,
    };
    let mut s = SummaryScreen::new(jstr(d, "name"), status)
        .with_subtitle(b.summary.clone())
        .fact("Status", status_str.to_string())
        .fact("Version", jstr(d, "version"))
        .fact(
            "Executable",
            d.get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("-")
                .to_string(),
        )
        .fact(
            "PATH (process/user/machine)",
            format!(
                "{}/{}/{}",
                yes_no(jbool(d, "on_process_path")),
                yes_no(jbool(d, "on_user_path")),
                yes_no(jbool(d, "on_machine_path")),
            ),
        );
    if let Some(cause) = d.get("cause").and_then(|c| c.as_str()) {
        let mut n = Notice::warning(format!("Likely cause: {cause}"));
        if let Some(action) = d.get("action").and_then(|a| a.as_str()) {
            n = n.with_detail(format!("Action: {action}"));
        }
        s = s.notice(n);
    }
    if let Some(alts) = d.get("alternates").and_then(|a| a.as_array()) {
        if !alts.is_empty() {
            let mut t =
                TableModel::new(vec!["Candidate", "Source"]).with_title("Other installations");
            for a in alts {
                t.add_row([jstr(a, "path"), jstr(a, "source")]);
            }
            s = s.with_table(t);
        }
    }
    if let Some(route) = d.get("install_route").and_then(|r| r.as_str()) {
        s = s.fact("Install route", route.to_string());
    }
    Some(s)
}

fn tools_filter_table(b: &Builder) -> Option<TableView> {
    let d = &b.data;
    let filter = d.get("filter")?.as_str()?;
    let tools = d.get("tools")?.as_array()?;
    if tools.is_empty() {
        return None; // plain "no tools are X" line is clearer
    }
    let mut t = TableModel::new(vec!["Tool", "Status", "Version", "Detail"])
        .with_title(format!("{} tool(s) {filter}", tools.len()));
    for rec in tools {
        let detail = rec
            .get("cause")
            .and_then(|c| c.as_str())
            .or_else(|| rec.get("path").and_then(|p| p.as_str()))
            .unwrap_or("")
            .to_string();
        t.add_row([
            jstr(rec, "id"),
            jstr(rec, "status"),
            short_version(rec),
            detail,
        ]);
    }
    Some(TableView::new(t))
}

fn install_receipt(b: &Builder) -> Option<Receipt> {
    if b.status == Status::Refused {
        return None; // consent explanation stays on the plain path
    }
    let d = &b.data;
    let mut r = Receipt::success("Tool installation").with_status(envelope_status(b.status));
    for res in d.get("results")?.as_array()? {
        let tool = jstr(res, "tool");
        let state = match (
            jbool(res, "attempted").unwrap_or(false),
            jbool(res, "verified").unwrap_or(false),
        ) {
            (false, _) => "skipped",
            (true, true) => "verified",
            (true, false) => "NOT verified",
        };
        r = r.change(tool, state.to_string());
    }
    r = r
        .change("Installed", jnum(d, "installed").unwrap_or(0).to_string())
        .change("Skipped", jnum(d, "skipped").unwrap_or(0).to_string())
        .change("Failed", jnum(d, "failed").unwrap_or(0).to_string());
    Some(r)
}

fn fix_path_receipt(b: &Builder) -> Option<Receipt> {
    let d = &b.data;
    if !jbool(d, "applied").unwrap_or(false) {
        return None;
    }
    let plan = d.get("plan")?;
    let mut r = Receipt::success("USER PATH updated");
    for dir in plan.get("directories_to_add")?.as_array()? {
        r = r.change("Added", dir.as_str().unwrap_or_default().to_string());
    }
    r = r
        .change("Verified by re-read", "yes".to_string())
        .guidance("restart shells/agents to pick up the change".to_string());
    Some(r)
}

fn fix_path_plan(b: &Builder) -> Option<Plan> {
    let d = &b.data;
    let plan = d.get("plan")?;
    let dirs: Vec<String> = plan
        .get("directories_to_add")?
        .as_array()?
        .iter()
        .map(|x| x.as_str().unwrap_or_default().to_string())
        .collect();
    if dirs.is_empty() {
        return None; // nothing to plan; plain output is clearer
    }
    let dry = jbool(d, "dry_run").unwrap_or(false);
    let title = if dry {
        "USER PATH repair — DRY RUN"
    } else {
        "USER PATH repair — proposed"
    };
    let mut p = Plan::new(title).with_description(
        "Additive only; the machine PATH is never modified. Nothing has been written.".to_string(),
    );
    for dir in &dirs {
        p = p.add_change(PlanChange::add(dir.clone()).with_detail("append to USER PATH"));
    }
    p = p.consequence("existing processes keep their inherited PATH; new processes see the change");
    if b.status == Status::Refused {
        p = p.warning("confirmation was not given — nothing was changed");
    }
    Some(p)
}

fn verify_discover_screen(b: &Builder) -> Option<SummaryScreen> {
    let d = &b.data;
    let ambiguous = jbool(d, "ambiguous").unwrap_or(false);
    let authority = d.get("authority").filter(|a| a.is_object());
    let status = if ambiguous {
        VStatus::Attention
    } else if authority.is_some() {
        VStatus::Ready
    } else {
        VStatus::Skipped
    };
    let mut s = SummaryScreen::new("Verification Authority", status).with_subtitle(jstr(d, "root"));
    if let Some(a) = authority {
        s = s
            .fact(
                "Authority",
                format!("{} ({})", jstr(a, "kind"), jstr(a, "source")),
            )
            .fact("Runner", jstr(a, "runner"));
        if let Some(tasks) = a.get("tasks").and_then(|t| t.as_array()) {
            let mut t2 = TableModel::new(vec!["Declared task"]).with_title(format!(
                "Tasks ({})",
                jnum(a, "task_count").unwrap_or(tasks.len() as u64)
            ));
            for task in tasks.iter().take(20) {
                t2.add_row([task.as_str().unwrap_or_default().to_string()]);
            }
            s = s.with_table(t2);
        }
    }
    for which in ["quick", "full"] {
        if let Some(t) = d.get(which).filter(|t| t.is_object()) {
            let cmd = t
                .get("command")
                .and_then(|c| c.as_array())
                .map(|c| {
                    c.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            s = s.fact(
                which.to_string(),
                format!("{} -> {cmd}  [{}]", jstr(t, "task"), jstr(t, "source")),
            );
        }
    }
    if ambiguous {
        let list = d
            .get("ambiguity")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let mut n = Notice::warning(format!(
            "Ambiguous authorities ({list}) — Supertools refuses to guess"
        ));
        if let Some(ex) = d.get("config_example").and_then(|c| c.as_str()) {
            n = n.with_detail(format!(".supertools.toml:\n{ex}"));
        }
        s = s.notice(n);
    }
    if let Some(notes) = d.get("notes").and_then(|n| n.as_array()) {
        for n in notes.iter().filter_map(|x| x.as_str()) {
            s = s.notice(Notice::info(n.to_string()));
        }
    }
    Some(s)
}

fn verify_outcome(b: &Builder) -> Option<Outcome> {
    let d = &b.data;
    let passed = jbool(d, "passed").unwrap_or(false);
    let timed_out = jbool(d, "timed_out").unwrap_or(false);
    let status = if b.status == Status::Ok && passed {
        VStatus::Ready
    } else {
        VStatus::Failed
    };
    let cmd = d
        .get("command")
        .and_then(|c| c.as_array())
        .map(|c| {
            c.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let mut o = Outcome::new(status, b.summary.clone())
        .fact("Authority", jstr(d, "authority"))
        .fact(
            "Task",
            format!("{} [{}]", jstr(d, "task"), jstr(d, "task_source")),
        )
        .fact("Command", cmd)
        .fact(
            "Exit",
            d.get("exit_code")
                .map(|x| x.to_string())
                .unwrap_or_else(|| "-".to_string()),
        )
        .fact(
            "Duration",
            format!("{}ms", jnum(d, "duration_ms").unwrap_or(0)),
        );
    if timed_out {
        o = o.with_warning(Notice::warning("timed out and was killed".to_string()));
    }
    let diag = d.get("diagnostics");
    let tail = diag
        .and_then(|x| x.get("stderr_tail"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            diag.and_then(|x| x.get("stdout_tail"))
                .and_then(|x| x.as_str())
        })
        .unwrap_or("");
    if !tail.trim().is_empty() {
        let lines: Vec<&str> = tail.lines().collect();
        let shown: Vec<&str> = lines.iter().rev().take(12).copied().collect::<Vec<_>>();
        o = o.with_details(shown.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Some(o)
}

/// Fatal-error view on stderr. Returns true when rendered.
pub fn render_failure(f: &Failure) -> bool {
    if !enabled() {
        return false;
    }
    let view = ErrorView::new(failure_model(f));
    // Diagnostics belong on stderr; stdout stays clean.
    SartorialOutput::print_diagnostic(&view, &context()).is_ok()
}

fn failure_model(f: &Failure) -> ErrorModel {
    let mut model = ErrorModel::new(f.message.clone()).with_why(format!(
        "status: {} (exit {})",
        f.status.as_str(),
        f.status.exit_code()
    ));
    for (i, h) in f.hints.iter().enumerate() {
        if h.trim().is_empty() {
            continue;
        }
        let key = char::from_digit((i % 9) as u32 + 1, 10).unwrap_or('?');
        model = model.with_action(Action::new(key, format!("hint-{}", i + 1), h.clone()));
    }
    model
}

/// Restrained, truthful live progress for long-running verification: one
/// stderr line at start (attached TTY, human mode only). No fake spinners,
/// no invented percentages; elapsed time is reported by the result itself.
pub fn verify_progress(command_display: &str, timeout_secs: u64) {
    use std::io::IsTerminal;
    if json_mode() || !std::io::stderr().is_terminal() {
        return;
    }
    eprintln!("running: {command_display}  (timeout {timeout_secs}s)");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain<V: sartorial::render::RenderPlain>(v: &V) -> String {
        sartorial::to_plain(v).expect("plain render")
    }

    fn assert_no_ansi(s: &str) {
        assert!(!s.contains('\u{1b}'), "ANSI leaked into plain render");
    }

    #[test]
    fn disabled_under_captured_test_harness() {
        // cargo test captures output => stdout is not a terminal => the
        // Sartorial path can never engage in piped/CI runs.
        set_json_mode(false);
        assert!(!enabled(), "Sartorial must stay off when stdout is piped");
    }

    #[test]
    fn json_mode_disables_everything() {
        set_json_mode(true);
        assert!(!enabled());
        set_json_mode(false);
    }

    #[test]
    fn status_mapping_mirrors_envelope() {
        assert_eq!(envelope_status(Status::Ok), VStatus::Ready);
        assert_eq!(envelope_status(Status::NoResults), VStatus::Skipped);
        assert_eq!(envelope_status(Status::Failed), VStatus::Failed);
        assert_eq!(envelope_status(Status::Timeout), VStatus::Failed);
        assert_eq!(envelope_status(Status::Ambiguous), VStatus::Attention);
        assert_eq!(envelope_status(Status::Refused), VStatus::Attention);
        assert_eq!(
            envelope_status(Status::CapabilityUnavailable),
            VStatus::Attention
        );
        assert_eq!(envelope_status(Status::InvalidRequest), VStatus::Attention);
    }

    #[test]
    fn doctor_screen_dresses_existing_facts() {
        let b = Builder::ok("doctor", "overall: healthy").data(serde_json::json!({
            "overall": "healthy",
            "supertools_version": "0.1.0",
            "platform": "windows",
            "checks": [{"id": "repo", "status": "pass", "label": "Core repository capability available"}],
            "capability_summary": {"ready": 17, "degraded": 1, "unavailable": 1},
            "optional_improvements": [],
            "repo_context": {"in_repository": true, "branch": "main", "dirty": false},
        }));
        let s = plain(&doctor_screen(&b).expect("view")).to_lowercase();
        assert_no_ansi(&s);
        assert!(s.contains("doctor"), "{s}");
        assert!(s.contains("repo"), "{s}");
        assert!(s.contains("17 ready"), "{s}");
        assert!(s.contains("main"), "{s}");
    }

    #[test]
    fn verify_discover_ambiguity_screen_shows_config_example() {
        let b = Builder::new("verify.discover", Status::Ambiguous, "ambiguous").data(
            serde_json::json!({
                "root": "/proj",
                "ambiguous": true,
                "ambiguity": ["just", "package"],
                "authority": null,
                "notes": [],
                "config_example": crate::verify::config_example("just"),
            }),
        );
        let s = plain(&verify_discover_screen(&b).expect("view"));
        assert_no_ansi(&s);
        assert!(s.contains("Ambiguous"), "{s}");
        assert!(s.contains("[verify]"), "{s}");
        assert!(s.contains("authority"), "{s}");
    }

    #[test]
    fn verify_outcome_shows_command_and_duration() {
        let b = Builder::ok("verify.quick", "verify check passed in 1234ms (exit 0)").data(
            serde_json::json!({
                "authority": "cargo",
                "task": "check",
                "task_source": "cargo built-in semantics",
                "command": ["cargo", "check", "--all-targets"],
                "exit_code": 0,
                "duration_ms": 1234,
                "timed_out": false,
                "passed": true,
                "diagnostics": {"stdout_tail": "Finished dev profile", "stderr_tail": ""},
            }),
        );
        let s = plain(&verify_outcome(&b).expect("view"));
        assert_no_ansi(&s);
        assert!(s.contains("cargo check --all-targets"), "{s}");
        assert!(s.contains("1234ms"), "{s}");
    }

    #[test]
    fn failure_model_contains_message_and_hints() {
        let f = Failure::unavailable("search.structural", "ast-grep is not available")
            .hint("supertools tools install-missing --yes");
        let s = plain(&ErrorView::new(failure_model(&f))).to_lowercase();
        assert_no_ansi(&s);
        assert!(s.contains("ast-grep"), "{s}");
        assert!(s.contains("install-missing"), "{s}");
    }

    #[test]
    fn fix_path_plan_lists_directories() {
        let b = Builder::ok("tools.fix-path", "dry run").data(serde_json::json!({
            "dry_run": true,
            "applied": false,
            "plan": {"directories_to_add": ["C:\\tools\\bin"], "already_present": [], "unchanged_entries": 3, "notes": []},
        }));
        let s = plain(&fix_path_plan(&b).expect("view"));
        assert_no_ansi(&s);
        assert!(s.contains("tools"), "{s}");
        assert!(fix_path_receipt(&b).is_none());
    }

    #[test]
    fn install_receipt_marks_verification_state() {
        let b = Builder::ok("tools.install-missing", "1 installed, 0 skipped, 0 failed").data(
            serde_json::json!({
                "results": [{"tool": "rg", "attempted": true, "verified": true, "notes": []}],
                "installed": 1, "skipped": 0, "failed": 0,
            }),
        );
        let s = plain(&install_receipt(&b).expect("view"));
        assert_no_ansi(&s);
        assert!(s.contains("rg"), "{s}");
        assert!(s.contains("verified"), "{s}");
    }

    #[test]
    fn styling_engages_only_for_the_human_target() {
        // Synthetic proof of the TTY-only styling path: with the Human
        // target and forced color, Sartorial produces ANSI; production code
        // reaches a Human-target context only when stdout is an attached
        // terminal (enabled() gate), and RenderContext::detect() additionally
        // honours NO_COLOR via ColorChoice::Auto.
        use sartorial::render::RenderHuman;
        let b = Builder::ok("doctor", "overall: healthy").data(serde_json::json!({
            "overall": "healthy",
            "supertools_version": "0.1.0",
            "platform": "windows",
            "checks": [{"id": "repo", "status": "pass", "label": "ok"}],
            "capability_summary": {"ready": 17, "degraded": 1, "unavailable": 1},
            "optional_improvements": [],
        }));
        let screen = doctor_screen(&b).expect("view");
        let ctx = RenderContext::detect()
            .with_config(
                Config::default()
                    .with_preset(Preset::House)
                    .with_color(sartorial::ColorChoice::Always),
            )
            .with_target(RenderTarget::Human);
        let styled = screen.to_human_string(&ctx).expect("human render");
        assert!(
            styled.contains('\u{1b}'),
            "Human target must produce ANSI styling"
        );
        // The same view through to_plain never carries ANSI.
        let plain_out = plain(&screen);
        assert_no_ansi(&plain_out);
    }

    #[test]
    fn tools_screen_renders_registry_rows() {
        let b = Builder::ok("tools.list", "9 available, 0 hidden, 2 missing, 0 ambiguous, 0 broken")
            .data(serde_json::json!({
                "platform": "windows",
                "deep": false,
                "tools": [
                    {"id": "git", "status": "available", "version": "git version 2.47.0", "role": "repository truth", "path": "C:/git/bin/git.exe"},
                    {"id": "rg", "status": "missing", "role": "text search", "install_available": true},
                ],
                "summary": {"available": 1, "hidden": 0, "missing": 1, "ambiguous": 0, "broken": 0},
            }));
        let s = plain(&tools_screen(&b, "Supertools Toolkit", false).expect("view"));
        assert_no_ansi(&s);
        assert!(s.contains("git"), "{s}");
        assert!(s.contains("missing"), "{s}");
    }
}
