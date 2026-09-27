//! `tools` (inventory) and `find` (investigation) commands — two views over
//! the SAME Tool Registry. Also hosts the consent-gated installation and
//! USER-PATH repair flows.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use serde_json::json;

use crate::output::{Builder, CmdResult, Evidence, Failure, Status};
use crate::registry::{
    env::EnvFacts, install, pathfix, paths_equal, ToolId, ToolRecord, ToolRegistry, ToolStatus,
    ALL_TOOLS,
};

// ------------------------------------------------------------ rendering

fn short_version(rec: &ToolRecord) -> String {
    match &rec.version {
        None => "-".to_string(),
        Some(v) => {
            // First whitespace-separated token starting with a digit,
            // reduced to its leading numeric dotted prefix.
            let mut out = String::new();
            for tok in v.split_whitespace() {
                if tok.starts_with(|c: char| c.is_ascii_digit()) {
                    let num: String = tok
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    let num = num.trim_end_matches('.');
                    if !num.is_empty() && num.contains('.') {
                        out = num.to_string();
                        break;
                    }
                }
            }
            if out.is_empty() {
                out = crate::process::head(v, 24).0;
            }
            out
        }
    }
}

/// `tools` view: version + role. `find` view: status + path (investigation).
fn table_lines(reg: &ToolRegistry, with_paths: bool) -> Vec<String> {
    let mut lines = if with_paths {
        vec!["TOOL         STATUS       PATH".to_string()]
    } else {
        vec!["TOOL         STATUS       VERSION              ROLE".to_string()]
    };
    for rec in &reg.tools {
        if with_paths {
            lines.push(format!(
                "{:<12} {:<10} {}",
                rec.id.as_str(),
                rec.status.as_str().to_uppercase(),
                rec.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
        } else {
            lines.push(format!(
                "{:<12} {:<12} {:<20} {}",
                rec.id.as_str(),
                rec.status.as_str(),
                short_version(rec),
                rec.role
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "{} available · {} hidden · {} missing{}",
        reg.summary.available,
        reg.summary.hidden,
        reg.summary.missing,
        if reg.summary.ambiguous + reg.summary.broken > 0 {
            format!(
                " · {} ambiguous · {} broken",
                reg.summary.ambiguous, reg.summary.broken
            )
        } else {
            String::new()
        }
    ));
    lines
}

fn registry_data(reg: &ToolRegistry) -> serde_json::Value {
    json!({
        "platform": reg.platform,
        "deep": reg.deep,
        "tools": reg.tools,
        "summary": reg.summary,
    })
}

fn confirm(question: &str, assume_yes: bool) -> bool {
    if assume_yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{question} [Y/n] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let a = answer.trim().to_ascii_lowercase();
    a.is_empty() || a == "y" || a == "yes"
}

fn noninteractive_note() -> String {
    "not interactive and --yes was not supplied; nothing was changed (re-run with --yes for unattended use)".to_string()
}

// ------------------------------------------------------------ tools list

pub fn tools_list() -> CmdResult {
    let operation = "tools.list";
    let reg = ToolRegistry::shallow();
    let mut b = Builder::ok(
        operation,
        format!(
            "{} available, {} hidden, {} missing, {} ambiguous, {} broken",
            reg.summary.available,
            reg.summary.hidden,
            reg.summary.missing,
            reg.summary.ambiguous,
            reg.summary.broken
        ),
    )
    .data(registry_data(&reg));
    b = b.line("SUPERTOOLS TOOLKIT").line("");
    for l in table_lines(&reg, false) {
        b = b.line(l);
    }
    if reg.summary.missing > 0 {
        b = b.line("").line("Supertools remains operational.");
        b = b.line("Run `supertools tools missing` for missing recommended tools.");
    }
    Ok(b)
}

pub fn tools_show(tool: String) -> CmdResult {
    let operation = "tools.show";
    let id = ToolId::from_arg(&tool).ok_or_else(|| {
        Failure::invalid(operation, format!("unknown tool \"{tool}\"")).hint(format!(
            "known tools: {}",
            ALL_TOOLS
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let reg = ToolRegistry::deep();
    let rec = reg
        .get(id)
        .ok_or_else(|| Failure::failed(operation, "registry did not produce a record"))?;
    let mut b = Builder::ok(
        operation,
        format!("{} is {}", rec.id.as_str(), rec.status.as_str()),
    )
    .data(serde_json::to_value(rec).unwrap_or(json!({})));

    let was_alias = tool.trim().to_ascii_lowercase() != id.as_str() && id.exec_names().len() > 1;
    b = b.line(rec.name).line("");
    b = b.line(format!("Status:             {}", rec.status.as_str()));
    if was_alias {
        b = b.line(format!(
            "Note:               \"{}\" is an alias of {} (same program)",
            tool.trim(),
            id.as_str()
        ));
    }
    b = b.line(format!(
        "Version:            {}",
        rec.version.clone().unwrap_or_else(|| "-".into())
    ));
    b = b.line(format!(
        "Executable:         {}",
        rec.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into())
    ));
    b = b.line(format!(
        "Discovery source:   {}",
        rec.discovery_source
            .map(|s| s.label())
            .unwrap_or("not found")
    ));
    b = b.line(format!(
        "Process PATH:       {}",
        yes_no(rec.on_process_path)
    ));
    b = b.line(format!("User PATH:          {}", yes_no(rec.on_user_path)));
    b = b.line(format!(
        "Machine PATH:       {}",
        yes_no(rec.on_machine_path)
    ));
    b = b.line(format!("Recommended:        {}", yes_no(rec.recommended)));
    b = b.line(format!("Required:           {}", yes_no(rec.required)));
    b = b
        .line("")
        .line("Used by Supertools:")
        .line(format!("  {}", rec.used_by));
    if !rec.capabilities.is_empty() {
        b = b.line(format!(
            "Powers capabilities: {}",
            rec.capabilities.join(", ")
        ));
    }
    if !rec.alternates.is_empty() {
        b = b.line("").line("Other installations:");
        for a in &rec.alternates {
            b = b.line(format!(
                "  {} ({}){}",
                a.path.display(),
                a.source.label(),
                a.version
                    .as_ref()
                    .map(|v| format!("  {v}"))
                    .unwrap_or_default()
            ));
        }
    }
    for n in &rec.notes {
        b = b.line(format!("note: {n}"));
    }
    if let Some(cause) = &rec.cause {
        b = b.line("").line(format!("Likely cause:       {cause}"));
    }
    if let Some(action) = &rec.action {
        b = b.line(format!("Action:             {action}"));
    }
    if rec.status == ToolStatus::Missing {
        match &rec.install_route {
            Some(route) if rec.install_available => {
                b = b.line("").line(format!("Install available:  {route}"));
                b = b.line(format!(
                    "Install with:       supertools find {} --install",
                    rec.id.as_str()
                ));
            }
            Some(g) => {
                b = b.line("").line(format!("No automated route: {g}"));
            }
            None => {
                b = b.line("").line("Supertools works without it.");
            }
        }
    }
    if let Some(s) = rec.suggested {
        b = b.line("").line(format!("Suggested:          {s}"));
    }
    Ok(b)
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

pub fn tools_filter(which: &str) -> CmdResult {
    let operation = format!("tools.{which}");
    let want = match which {
        "available" => ToolStatus::Available,
        "hidden" => ToolStatus::Hidden,
        "missing" => ToolStatus::Missing,
        _ => {
            return Err(
                Failure::invalid(&operation, format!("unknown filter \"{which}\""))
                    .hint("filters: available, hidden, missing"),
            )
        }
    };
    let reg = ToolRegistry::deep();
    let selected: Vec<&ToolRecord> = reg.tools.iter().filter(|t| t.status == want).collect();
    let data = json!({
        "filter": which,
        "count": selected.len(),
        "tools": selected.iter().map(|r| serde_json::to_value(*r).unwrap_or(json!({}))).collect::<Vec<_>>(),
    });

    let mut b = if selected.is_empty() {
        Builder::no_results(&operation, format!("no tools are {which}"))
    } else {
        Builder::ok(&operation, format!("{} tool(s) {which}", selected.len()))
    };
    b = b.data(data);

    match which {
        "hidden" => {
            b = b.line("HIDDEN TOOLS").line("");
            for rec in &selected {
                b = b.line(rec.id.as_str());
                if let Some(p) = &rec.path {
                    b = b.line(format!("  {}", p.display()));
                }
                if let Some(cause) = &rec.cause {
                    b = b.line(format!("  cause:  {cause}"));
                }
                if let Some(action) = &rec.action {
                    b = b.line(format!("  action: {action}"));
                }
                b = b.line("");
            }
            if !selected.is_empty() {
                b = b.line(
                    "These programs are installed but are not visible to the current process.",
                );
                b = b
                    .line("")
                    .line("Run:")
                    .line("  supertools tools fix-path --dry-run");
            }
        }
        "missing" => {
            for rec in &selected {
                b = b.line(rec.id.as_str());
                b = b.line(format!(
                    "  {}.",
                    rec.used_by.split(';').next().unwrap_or(rec.used_by)
                ));
                if rec.install_available {
                    b = b.line(format!(
                        "  Install available: {}.",
                        rec.install_route.clone().unwrap_or_default()
                    ));
                } else if let Some(g) = &rec.install_route {
                    b = b.line(format!("  Guidance: {g}"));
                } else {
                    b = b.line("  No automated installation route.");
                }
                b = b.line("");
            }
            b = b.line(format!("{} recommended tool(s) missing.", selected.len()));
            b = b.line("Supertools remains usable.");
        }
        _ => {
            for rec in &selected {
                b = b.line(format!(
                    "{:<12} {:<20} {}",
                    rec.id.as_str(),
                    short_version(rec),
                    rec.path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ));
            }
        }
    }
    Ok(b)
}

// ------------------------------------------------------- install-missing

pub fn tools_install_missing(yes: bool) -> CmdResult {
    let operation = "tools.install-missing";
    let facts = EnvFacts::collect(true);
    let reg = ToolRegistry::from_facts(facts.clone());
    let managers = install::probe_managers(&facts);

    let missing: Vec<ToolId> = reg
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Missing && t.recommended)
        .map(|t| t.id)
        .collect();

    if missing.is_empty() {
        return Ok(Builder::no_results(
            operation,
            "no recommended tools are missing — nothing to install",
        )
        .data(json!({ "results": [], "installed": 0, "skipped": 0, "failed": 0 })));
    }

    let interactive = std::io::stdin().is_terminal();
    if !yes && !interactive {
        let mut b = Builder::new(
            operation,
            Status::Refused,
            "consent required: installation changes this machine",
        )
        .data(json!({
            "missing": missing.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
            "results": [], "installed": 0, "skipped": missing.len(), "failed": 0,
        }))
        .line(noninteractive_note())
        .line(format!(
            "missing: {}",
            missing
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        b = b.next(
            "supertools tools install-missing --yes",
            "unattended installation of the missing recommended tools",
        );
        return Ok(b);
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut human: Vec<String> = Vec::new();
    let mut installed_n = 0usize;
    let mut skipped_n = 0usize;
    let mut failed_n = 0usize;

    for id in &missing {
        let rec = reg.get(*id).cloned();
        let reason = rec.map(|r| r.used_by.to_string()).unwrap_or_default();
        let route = install::choose_route(*id, Some(&managers), &facts);
        let Some(route) = route else {
            skipped_n += 1;
            let guidance = install::install_options(*id)
                .iter()
                .find_map(|m| match m {
                    install::InstallMechanism::Guidance { url, note } => {
                        Some(format!("{url} — {note}"))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "no known route".into());
            human.push(format!(
                "{}: no trustworthy automated route on this machine",
                id.as_str()
            ));
            human.push(format!("  guidance: {guidance}"));
            results.push(json!({
                "tool": id, "attempted": false, "verified": false,
                "notes": ["no trustworthy automated installation route; guidance provided"],
                "guidance": guidance,
            }));
            continue;
        };

        human.push(format!(
            "{} is missing ({}). Route: {}",
            id.as_str(),
            reason.split(';').next().unwrap_or(&reason),
            route.display
        ));
        let question = format!("Install {}?", id.project_name());
        if !confirm(&question, yes) {
            skipped_n += 1;
            human.push("  skipped by user".into());
            results.push(json!({"tool": id, "attempted": false, "verified": false, "notes": ["skipped by user"]}));
            continue;
        }

        match install::execute_route(*id, &route, operation) {
            Ok(mut outcome) => {
                // Refresh discovery state; verify by actually finding the exe.
                let facts2 = EnvFacts::collect(true);
                let reg2 = ToolRegistry::from_facts(facts2.clone());
                install::verify_after_install(&mut outcome, &reg2);
                if outcome.verified {
                    installed_n += 1;
                } else {
                    failed_n += 1;
                }
                human.push(format!(
                    "  manager exit {:?} in {}ms → {}",
                    outcome.manager_exit_code,
                    outcome.duration_ms,
                    if outcome.verified {
                        "VERIFIED"
                    } else {
                        "NOT VERIFIED"
                    }
                ));
                if let Some(p) = &outcome.resolved_path {
                    human.push(format!("  resolved: {}", p.display()));
                }
                for n in &outcome.notes {
                    human.push(format!("  note: {n}"));
                }
                // PATH repair offer for freshly installed but hidden tools.
                if outcome.verified && outcome.needs_new_process && !outcome.on_persisted_user_path
                {
                    if let Some(p) = &outcome.resolved_path {
                        if let Some(dir) = p.parent() {
                            human.push(format!(
                                "  {} is installed at {} but its directory is not on the persisted USER PATH",
                                id.as_str(),
                                p.display()
                            ));
                            if confirm(&format!("  Add {} to your user PATH?", dir.display()), yes)
                            {
                                match apply_path_fix(&facts2, &[dir.to_path_buf()], operation) {
                                    Ok(fixb) => human.extend(fixb.human),
                                    Err(e) => {
                                        human.push(format!("  PATH fix failed: {}", e.message))
                                    }
                                }
                            }
                        }
                    }
                }
                if !outcome.verified && !outcome.visible_on_process_path {
                    human.push(
                        "  note: this process' PATH will not change; a NEW shell/agent process is required to see persisted PATH updates"
                            .into(),
                    );
                }
                results.push(serde_json::to_value(&outcome).unwrap_or(json!({})));
            }
            Err(e) => {
                failed_n += 1;
                human.push(format!("  install failed: {}", e.message));
                results.push(
                    json!({"tool": id, "attempted": true, "verified": false, "notes": [e.message]}),
                );
            }
        }
    }

    let status = if failed_n > 0 {
        Status::Failed
    } else {
        Status::Ok
    };
    let mut b = Builder::new(
        operation,
        status,
        format!("{installed_n} installed, {skipped_n} skipped, {failed_n} failed"),
    )
    .data(json!({
        "results": results,
        "installed": installed_n,
        "skipped": skipped_n,
        "failed": failed_n,
    }))
    .lines(human);
    b = b.next(
        "supertools tools",
        "re-check the inventory after installation",
    );
    Ok(b)
}

// ------------------------------------------------------------ fix-path

fn apply_path_fix(
    facts: &EnvFacts,
    hidden_dirs: &[PathBuf],
    operation: &str,
) -> Result<Builder, Failure> {
    let plan = pathfix::plan_path_fix(&facts.user_path, hidden_dirs);
    let mut b = Builder::ok(operation, "PATH repair plan").data(json!({}));
    if plan.is_noop() {
        b.summary = "no USER PATH change needed".into();
        let notes = plan.notes.clone();
        b.data = json!({ "dry_run": false, "applied": false, "plan": plan, "notes": notes });
        for n in &b.data["notes"].as_array().cloned().unwrap_or_default() {
            if let Some(s) = n.as_str() {
                b = b.line(format!("note: {s}"));
            }
        }
        b = b.line("no changes required");
        return Ok(b);
    }
    let raw = current_raw_user_path(operation)?;
    let composed = pathfix::compose_user_path(&raw, &plan.directories_to_add);
    let (old, new) = pathfix::apply_user_path_fix(&composed)
        .map_err(|e| Failure::failed(operation, format!("USER PATH repair failed: {e}")))?;
    b.summary = format!(
        "added {} director(y/ies) to USER PATH",
        plan.directories_to_add.len()
    );
    b.data = json!({
        "dry_run": false,
        "applied": true,
        "plan": plan,
        "old_length": old.len(),
        "new_length": new.len(),
        "verified_by_reread": true,
    });
    b = b
        .evidence(Evidence::fact("registry", json!("HKCU\\Environment Path")))
        .evidence(Evidence::fact("directories_added", json!(plan.directories_to_add.iter().map(|p| p.display().to_string()).collect::<Vec<_>>())))
        .warning("existing processes (including this shell/agent) keep their inherited PATH; start a NEW process to see the change")
        .line("USER PATH updated and verified by re-reading the registry:");
    for d in &plan.directories_to_add {
        b = b.line(format!("  + {}", d.display()));
    }
    b = b.line("restart shells/agents to pick up the change.");
    Ok(b)
}

fn current_raw_user_path(operation: &str) -> Result<String, Failure> {
    #[cfg(windows)]
    {
        crate::registry::env::windows_user_path_string().ok_or_else(|| {
            Failure::failed(
                operation,
                "cannot read persisted USER PATH from HKCU\\Environment",
            )
        })
    }
    #[cfg(not(windows))]
    {
        let _ = operation;
        Err(Failure::unavailable(
            operation,
            "USER PATH repair is Windows-only in v0.1",
        ))
    }
}

pub fn tools_fix_path(dry_run: bool, yes: bool) -> CmdResult {
    let operation = "tools.fix-path";
    if cfg!(not(windows)) {
        return Err(Failure::unavailable(
            operation,
            "PATH repair is Windows-first in v0.1; other platforms are not implemented",
        ));
    }
    let facts = EnvFacts::collect(true);
    let reg = ToolRegistry::from_facts(facts.clone());

    let hidden: Vec<&ToolRecord> = reg
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Hidden)
        .collect();
    if hidden.is_empty() {
        return Ok(
            Builder::no_results(operation, "no hidden tools — no USER PATH repair needed")
                .data(json!({ "dry_run": dry_run, "applied": false, "hidden_tools": [] })),
        );
    }

    let mut hidden_dirs: Vec<PathBuf> = Vec::new();
    let mut restart_only: Vec<String> = Vec::new();
    for rec in &hidden {
        let Some(p) = &rec.path else { continue };
        let Some(dir) = p.parent() else { continue };
        if rec.on_user_path || rec.on_machine_path {
            restart_only.push(format!(
                "{}: {} is already on a persisted PATH — restart the shell/agent",
                rec.id.as_str(),
                dir.display()
            ));
        } else if !hidden_dirs.iter().any(|d| paths_equal(d, dir)) {
            hidden_dirs.push(dir.to_path_buf());
        }
    }

    if dry_run {
        let plan = pathfix::plan_path_fix(&facts.user_path, &hidden_dirs);
        let mut b = Builder::ok(
            operation,
            format!(
                "dry run: {} director(y/ies) would be added to USER PATH",
                plan.directories_to_add.len()
            ),
        )
        .data(json!({
            "dry_run": true,
            "applied": false,
            "plan": plan,
            "restart_only": restart_only,
            "hidden_tools": hidden.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        }))
        .line("DRY RUN — nothing was written.")
        .line(format!(
            "persisted USER PATH entries: {}",
            facts.user_path.len()
        ))
        .line("would add:");
        for d in &plan.directories_to_add {
            b = b.line(format!("  + {}", d.display()));
        }
        for r in &restart_only {
            b = b.line(format!("note: {r}"));
        }
        if plan.is_noop() {
            b = b.line("no changes would be made.");
        } else {
            b = b.line("").line("apply with: supertools tools fix-path");
        }
        b = b.warning(
            "only the USER PATH is ever modified; machine PATH is read-only for Supertools",
        );
        return Ok(b);
    }

    if hidden_dirs.is_empty() {
        let mut b = Builder::no_results(operation, "no USER PATH change needed")
            .data(json!({ "dry_run": false, "applied": false, "restart_only": restart_only }));
        for r in &restart_only {
            b = b.line(format!("note: {r}"));
        }
        return Ok(b);
    }

    let plan = pathfix::plan_path_fix(&facts.user_path, &hidden_dirs);
    let mut preview = vec!["Proposed USER PATH change (machine PATH untouched):".to_string()];
    for d in &plan.directories_to_add {
        preview.push(format!("  + {}", d.display()));
    }
    let question = format!(
        "Add {} director(y/ies) to your USER PATH?",
        plan.directories_to_add.len()
    );
    if !confirm(&question, yes) {
        let mut b = Builder::new(
            operation,
            Status::Refused,
            "PATH repair not confirmed; nothing was changed",
        )
        .data(json!({ "dry_run": false, "applied": false, "plan": plan }))
        .lines(preview);
        if !yes && !std::io::stdin().is_terminal() {
            b = b.line(noninteractive_note());
            b = b.next(
                "supertools tools fix-path --yes",
                "apply without interactive confirmation",
            );
        }
        return Ok(b);
    }

    let mut b = apply_path_fix(&facts, &hidden_dirs, operation)?;
    for r in &restart_only {
        b = b.line(format!("note: {r}"));
    }
    Ok(b)
}

// ------------------------------------------------------------------ find

pub fn find(
    tool: Option<String>,
    install: bool,
    install_missing: bool,
    fix_path: bool,
    dry_run: bool,
    yes: bool,
) -> CmdResult {
    if install_missing {
        return tools_install_missing(yes);
    }
    if fix_path {
        return tools_fix_path(dry_run, yes);
    }
    if let Some(t) = &tool {
        return find_one(t, install, yes);
    }
    find_all(install, yes)
}

fn find_all(missing_install: bool, yes: bool) -> CmdResult {
    let operation = "find";
    let facts = EnvFacts::collect(true);
    let reg = ToolRegistry::from_facts(facts.clone());
    let mut b = Builder::ok(
        operation,
        format!(
            "{} available, {} hidden, {} missing, {} ambiguous, {} broken",
            reg.summary.available,
            reg.summary.hidden,
            reg.summary.missing,
            reg.summary.ambiguous,
            reg.summary.broken
        ),
    )
    .data(registry_data(&reg))
    .line("SUPERTOOLS — TOOL DISCOVERY")
    .line("");
    for l in table_lines(&reg, true) {
        b = b.line(l);
    }
    let actionable: Vec<&ToolRecord> = reg
        .tools
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                ToolStatus::Hidden
                    | ToolStatus::Missing
                    | ToolStatus::Ambiguous
                    | ToolStatus::Broken
            )
        })
        .collect();
    for rec in &actionable {
        b = b.line("");
        b = b.line(format!(
            "{} — {}",
            rec.id.as_str(),
            rec.status.as_str().to_uppercase()
        ));
        if let Some(p) = &rec.path {
            b = b.line(format!("  {}", p.display()));
        }
        for a in &rec.alternates {
            b = b.line(format!(
                "  candidate: {} ({})",
                a.path.display(),
                a.source.label()
            ));
        }
        if let Some(cause) = &rec.cause {
            b = b.line(format!("  cause:  {cause}"));
        }
        if let Some(action) = &rec.action {
            b = b.line(format!("  action: {action}"));
        }
        for n in &rec.notes {
            b = b.line(format!("  note: {n}"));
        }
    }

    let missing_recommended = reg
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Missing && t.recommended)
        .count();
    if missing_recommended > 0 {
        b = b.line("");
        b = b.line(format!(
            "{missing_recommended} recommended tool(s) missing."
        ));
        b = b.line("Supertools works without these tools, but installing them gives agents the complete recommended toolset.");
        if std::io::stdin().is_terminal() || yes {
            if confirm(
                &format!("Install the {missing_recommended} missing tools?"),
                yes,
            ) {
                let installed = tools_install_missing(yes)?;
                b = b.line("").lines(installed.human.clone());
                if let Some(obj) = b.data.as_object_mut() {
                    obj.insert("install_results".into(), installed.data.clone());
                }
            }
        } else {
            b = b.line("Run `supertools tools install-missing --yes` to install them.");
        }
    } else if missing_install {
        b = b.line("no missing recommended tools to install");
    }
    Ok(b)
}

fn find_one(tool: &str, install: bool, yes: bool) -> CmdResult {
    let operation = "find.tool";
    let id = ToolId::from_arg(tool).ok_or_else(|| {
        Failure::invalid(operation, format!("unknown tool \"{tool}\"")).hint(format!(
            "known tools: {}",
            ALL_TOOLS
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let facts = EnvFacts::collect(true);
    let reg = ToolRegistry::from_facts(facts.clone());
    let rec = reg
        .get(id)
        .cloned()
        .ok_or_else(|| Failure::failed(operation, "registry did not produce a record"))?;

    let mut b = Builder::ok(
        operation,
        format!("{} is {}", rec.id.as_str(), rec.status.as_str()),
    )
    .data(serde_json::to_value(&rec).unwrap_or(json!({})));

    match rec.status {
        ToolStatus::Available => {
            b = b.line(format!("{} is available.", rec.id.as_str()));
            b = b.line("").line("Found:");
            b = b.line(format!(
                "  {}",
                rec.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
            b = b.line("").line(format!(
                "Version: {}",
                rec.version.clone().unwrap_or_else(|| "-".into())
            ));
            b = b
                .line("")
                .line(format!("Current PATH: {}", yes_no(rec.on_process_path)));
            b = b.line(format!("User PATH:      {}", yes_no(rec.on_user_path)));
            b = b.line(format!("Machine PATH:   {}", yes_no(rec.on_machine_path)));
            for a in &rec.alternates {
                b = b.line(format!(
                    "alternate: {} ({})",
                    a.path.display(),
                    a.source.label()
                ));
            }
        }
        ToolStatus::Hidden => {
            b = b.line(format!(
                "{} is installed but not visible to this process.",
                rec.id.as_str()
            ));
            b = b.line("").line("Found:");
            b = b.line(format!(
                "  {}",
                rec.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
            b = b.line("").line(format!(
                "Version: {}",
                rec.version.clone().unwrap_or_else(|| "-".into())
            ));
            b = b.line("").line("Current PATH: no".to_string());
            b = b.line(format!("User PATH:      {}", yes_no(rec.on_user_path)));
            b = b.line(format!("Machine PATH:   {}", yes_no(rec.on_machine_path)));
            if let Some(cause) = &rec.cause {
                b = b.line("").line("Likely cause:").line(format!("  {cause}"));
            }
            if let Some(action) = &rec.action {
                b = b.line("").line("Action:").line(format!("  {action}"));
            }
            if !rec.on_user_path && !rec.on_machine_path {
                if let Some(p) = &rec.path {
                    if let Some(dir) = p.parent() {
                        if confirm(&format!("\nAdd {} to your user PATH?", dir.display()), yes) {
                            match apply_path_fix(&facts, &[dir.to_path_buf()], operation) {
                                Ok(fixb) => b = b.line("").lines(fixb.human),
                                Err(e) => b = b.line(format!("PATH fix failed: {}", e.message)),
                            }
                        } else if !yes && !std::io::stdin().is_terminal() {
                            b = b.line("").line(
                                "run `supertools tools fix-path --yes` to repair the USER PATH",
                            );
                        }
                    }
                }
            }
        }
        ToolStatus::Ambiguous | ToolStatus::Broken => {
            b = b.line(format!(
                "{} needs attention ({}).",
                rec.id.as_str(),
                rec.status.as_str()
            ));
            if let Some(cause) = &rec.cause {
                b = b.line("").line("Cause:").line(format!("  {cause}"));
            }
            b = b.line("").line("Candidates:");
            for a in &rec.alternates {
                b = b.line(format!(
                    "  {} ({}){}",
                    a.path.display(),
                    a.source.label(),
                    a.version
                        .as_ref()
                        .map(|v| format!("  {v}"))
                        .unwrap_or_default()
                ));
            }
            if let Some(p) = &rec.path {
                b = b.line(format!(
                    "  {} ({})",
                    p.display(),
                    rec.discovery_source.map(|s| s.label()).unwrap_or("?")
                ));
            }
        }
        ToolStatus::Missing => {
            b = b.line(format!("{} was not found.", rec.id.as_str()));
            b = b
                .line("")
                .line(format!("Recommended: {}", yes_no(rec.recommended)));
            b = b.line(format!("Required by Supertools: {}", yes_no(rec.required)));
            b = b.line(format!(
                "Installation method available: {}",
                yes_no(rec.install_available)
            ));
            if let Some(route) = &rec.install_route {
                b = b.line(format!("Route/guidance: {route}"));
            }
            if install {
                if rec.install_available {
                    let managers = install::probe_managers(&facts);
                    if let Some(route) = install::choose_route(id, Some(&managers), &facts) {
                        if confirm(
                            &format!("\nInstall {} via {}?", rec.name, route.display),
                            yes,
                        ) {
                            let mut outcome = install::execute_route(id, &route, operation)?;
                            let facts2 = EnvFacts::collect(true);
                            let reg2 = ToolRegistry::from_facts(facts2);
                            install::verify_after_install(&mut outcome, &reg2);
                            b = b.line("");
                            b = b.line(format!(
                                "manager exit {:?} → {}",
                                outcome.manager_exit_code,
                                if outcome.verified {
                                    "VERIFIED"
                                } else {
                                    "NOT VERIFIED"
                                }
                            ));
                            if let Some(p) = &outcome.resolved_path {
                                b = b.line(format!("resolved: {}", p.display()));
                            }
                            for n in &outcome.notes {
                                b = b.line(format!("note: {n}"));
                            }
                            if outcome.verified {
                                b.status = Status::Ok;
                            } else {
                                b.status = Status::Failed;
                                b.summary =
                                    format!("{} installation could not be verified", id.as_str());
                            }
                            if let Some(obj) = b.data.as_object_mut() {
                                obj.insert(
                                    "install".into(),
                                    serde_json::to_value(&outcome).unwrap_or(json!({})),
                                );
                            }
                        } else if !yes && !std::io::stdin().is_terminal() {
                            b = b.line("").line(noninteractive_note());
                            b = b.next(
                                format!("supertools find {} --install --yes", id.as_str()),
                                "unattended installation",
                            );
                        }
                    }
                } else {
                    b = b.line("no trustworthy automated installation route on this machine; see guidance above");
                }
            } else if rec.install_available {
                b = b.line("").line(format!(
                    "Install with: supertools find {} --install",
                    rec.id.as_str()
                ));
            }
        }
    }
    for n in &rec.notes {
        b = b.line(format!("note: {n}"));
    }
    Ok(b)
}
