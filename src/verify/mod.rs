//! Verify domain: discover how THIS repository expects itself to be checked
//! and invoke that existing authority predictably.
//!
//! Supertools never invents verification truth and never becomes a build
//! system. Precedence (documented in README):
//!
//!   1. `.supertools.toml` [verify] explicit configuration (always wins)
//!   2. a single task-runner authority present at the repo root:
//!      mise config / justfile / package.json scripts
//!      (more than one => AMBIGUOUS, fail closed, exit 2)
//!   3. Cargo.toml => cargo authority with cargo's own semantics
//!      (quick = cargo check, full = cargo test)
//!   4. nothing => no authority; reported explicitly
//!
//! `verify task <name>` only executes names that resolve to discovered
//! declared tasks — never arbitrary command strings.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::output::{Builder, CmdResult, Evidence, Failure, Status};
use crate::process::{self, tail, Request};
use crate::registry::{quick_require, ToolId};

pub const DEFAULT_QUICK_TIMEOUT: u64 = 900;
pub const DEFAULT_FULL_TIMEOUT: u64 = 1800;
pub const DEFAULT_TASK_TIMEOUT: u64 = 900;
const DIAG_TAIL_OK: usize = 1024;
const DIAG_TAIL_FAIL: usize = 8 * 1024;
const MAX_STDOUT_CAP: usize = 2 * 1024 * 1024;

// ------------------------------------------------------------------ config

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct VerifySection {
    pub authority: Option<String>,
    pub quick: Option<String>,
    pub full: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct LimitsSection {
    pub output_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SupertoolsConfig {
    #[serde(default)]
    pub verify: VerifySection,
    #[serde(default)]
    pub limits: LimitsSection,
}

pub fn load_config(
    root: &Path,
    operation: &str,
) -> Result<(Option<SupertoolsConfig>, Option<PathBuf>), Failure> {
    let path = root.join(".supertools.toml");
    if !path.is_file() {
        return Ok((None, None));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Failure::invalid(operation, format!("cannot read {}: {e}", path.display())))?;
    let cfg: SupertoolsConfig = toml::from_str(&text).map_err(|e| {
        Failure::invalid(
            operation,
            format!("{} is malformed (fail closed): {e}", path.display()),
        )
    })?;
    Ok((Some(cfg), Some(path)))
}

// --------------------------------------------------------------- discovery

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityKind {
    Mise,
    Just,
    Package,
    Cargo,
}

impl AuthorityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthorityKind::Mise => "mise",
            AuthorityKind::Just => "just",
            AuthorityKind::Package => "package",
            AuthorityKind::Cargo => "cargo",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskRef {
    pub task: String,
    pub command: Vec<String>,
    pub source: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Authority {
    pub kind: AuthorityKind,
    pub source: String,
    pub tasks: Vec<String>,
    pub runner: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Discovery {
    pub root: PathBuf,
    pub authority: Option<Authority>,
    pub quick: Option<TaskRef>,
    pub full: Option<TaskRef>,
    pub candidates: Vec<serde_json::Value>,
    pub ambiguity: Vec<String>,
    pub config_path: Option<PathBuf>,
    pub notes: Vec<String>,
    /// Loaded project config (not serialized; drives timeouts/limits).
    #[serde(skip)]
    pub config: Option<SupertoolsConfig>,
}

pub fn project_root(operation: &str) -> Result<PathBuf, Failure> {
    if let Ok(git) = quick_require(ToolId::Git, operation) {
        let out = process::run(
            &Request::new(git, vec!["rev-parse".into(), "--show-toplevel".into()])
                .timeout(Duration::from_secs(15)),
        );
        if let Ok(o) = out {
            if o.success() {
                return Ok(PathBuf::from(o.stdout.trim()));
            }
        }
    }
    std::env::current_dir().map_err(|e| {
        Failure::failed(
            operation,
            format!("cannot determine current directory: {e}"),
        )
    })
}

fn first_existing(root: &Path, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|n| root.join(n).is_file())
        .map(|n| n.to_string())
}

pub fn discover(operation: &str) -> Result<Discovery, Failure> {
    let root = project_root(operation)?;
    discover_in(&root, operation)
}

/// Discovery against an explicit root — keeps tests free of global cwd
/// mutation and lets callers pin the project directory.
pub fn discover_in(root: &Path, operation: &str) -> Result<Discovery, Failure> {
    let root = root.to_path_buf();
    let (config, config_path) = load_config(&root, operation)?;
    let mut notes: Vec<String> = Vec::new();
    let mut candidates: Vec<serde_json::Value> = Vec::new();

    // Runner presence detection (repo-root local evidence only).
    let mise_file = first_existing(&root, &["mise.toml", ".mise.toml"]);
    let just_file = first_existing(&root, &["justfile", "Justfile", ".justfile"]);
    let package_file = if root.join("package.json").is_file() {
        let has_scripts = std::fs::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .map(|v| {
                v.get("scripts")
                    .and_then(|s| s.as_object())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if has_scripts {
            Some("package.json".to_string())
        } else {
            None
        }
    } else {
        None
    };
    let cargo_file = first_existing(&root, &["Cargo.toml"]);

    for (kind, file) in [
        ("mise", mise_file.clone()),
        ("just", just_file.clone()),
        ("package", package_file.clone()),
        ("cargo", cargo_file.clone()),
    ] {
        candidates.push(json!({ "kind": kind, "present": file.is_some(), "source": file }));
    }

    let runners_present: Vec<&str> = [
        mise_file.is_some().then_some("mise"),
        just_file.is_some().then_some("just"),
        package_file.is_some().then_some("package"),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Authority selection.
    let configured = config.as_ref().and_then(|c| c.verify.authority.clone());
    let kind: Option<AuthorityKind> = if let Some(a) = &configured {
        match a.as_str() {
            "mise" => Some(AuthorityKind::Mise),
            "just" => Some(AuthorityKind::Just),
            "package" => Some(AuthorityKind::Package),
            "cargo" => Some(AuthorityKind::Cargo),
            other => {
                return Err(Failure::invalid(
                    operation,
                    format!(
                        ".supertools.toml declares unknown authority \"{other}\" (fail closed)"
                    ),
                )
                .hint("valid authorities: mise, just, package, cargo"))
            }
        }
    } else if runners_present.len() > 1 {
        // Ambiguous: fail closed rather than pick the easiest.
        let mut d = empty_discovery(root, candidates, config_path, config);
        d.ambiguity = runners_present.iter().map(|s| s.to_string()).collect();
        d.notes.push(
            "multiple plausible verification authorities exist and no canonical one is declared"
                .into(),
        );
        return Ok(d);
    } else if let Some(r) = runners_present.first() {
        Some(match *r {
            "mise" => AuthorityKind::Mise,
            "just" => AuthorityKind::Just,
            _ => AuthorityKind::Package,
        })
    } else if cargo_file.is_some() {
        Some(AuthorityKind::Cargo)
    } else {
        None
    };

    let Some(kind) = kind else {
        let mut d = empty_discovery(root, candidates, config_path, config);
        d.notes.push(
            "no verification authority discovered (no mise/just/package/cargo project files)"
                .into(),
        );
        return Ok(d);
    };

    // Build the authority: declared tasks + runner resolution.
    let (tasks, runner, source): (Vec<String>, Option<String>, String) = match kind {
        AuthorityKind::Mise => {
            let tasks = if process::resolve_program("mise").is_ok() {
                mise_tasks(&root, &mut notes)
            } else {
                notes.push("mise.toml present but the mise executable is not available".into());
                Vec::new()
            };
            (
                tasks,
                process::resolve_program("mise")
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned()),
                mise_file.clone().unwrap_or_else(|| "mise.toml".into()),
            )
        }
        AuthorityKind::Just => {
            let tasks = if process::resolve_program("just").is_ok() {
                just_recipes(&root, &mut notes)
            } else {
                notes.push("justfile present but the just executable is not available".into());
                Vec::new()
            };
            (
                tasks,
                process::resolve_program("just")
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned()),
                just_file.clone().unwrap_or_else(|| "justfile".into()),
            )
        }
        AuthorityKind::Package => {
            let tasks = package_scripts(&root);
            let runner = pick_package_runner(&root, &mut notes);
            (
                tasks,
                runner,
                package_file
                    .clone()
                    .unwrap_or_else(|| "package.json".into()),
            )
        }
        AuthorityKind::Cargo => {
            let tasks = vec!["check".into(), "test".into(), "clippy".into(), "fmt".into()];
            let runner = process::resolve_program("cargo")
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
            if runner.is_none() {
                notes.push("Cargo.toml present but the cargo executable is not available".into());
            }
            (
                tasks,
                runner,
                cargo_file.clone().unwrap_or_else(|| "Cargo.toml".into()),
            )
        }
    };

    let authority = Authority {
        kind,
        source,
        tasks: tasks.clone(),
        runner,
    };

    // quick/full mapping: explicit config wins; otherwise documented heuristics.
    let quick_name = config
        .as_ref()
        .and_then(|c| c.verify.quick.clone())
        .or_else(|| match kind {
            AuthorityKind::Cargo => Some("check".into()),
            _ => ["quick", "check"]
                .iter()
                .find(|n| tasks.iter().any(|t| t == **n))
                .map(|s| s.to_string()),
        });
    let full_name = config
        .as_ref()
        .and_then(|c| c.verify.full.clone())
        .or_else(|| match kind {
            AuthorityKind::Cargo => Some("test".into()),
            _ => ["full", "verify", "ci", "test", "check"]
                .iter()
                .find(|n| tasks.iter().any(|t| t == **n))
                .map(|s| s.to_string()),
        });

    let quick_from_config = config
        .as_ref()
        .and_then(|c| c.verify.quick.clone())
        .is_some();
    let full_from_config = config
        .as_ref()
        .and_then(|c| c.verify.full.clone())
        .is_some();
    let mut d = empty_discovery(root, candidates, config_path, config);
    d.authority = Some(authority);
    d.notes = notes;
    d.quick = quick_name.map(|t| task_ref(kind, &t, quick_from_config));
    d.full = full_name.map(|t| task_ref(kind, &t, full_from_config));
    Ok(d)
}

fn empty_discovery(
    root: PathBuf,
    candidates: Vec<serde_json::Value>,
    config_path: Option<PathBuf>,
    config: Option<SupertoolsConfig>,
) -> Discovery {
    Discovery {
        root,
        authority: None,
        quick: None,
        full: None,
        candidates,
        ambiguity: Vec::new(),
        config_path,
        notes: Vec::new(),
        config,
    }
}

fn task_ref(kind: AuthorityKind, task: &str, from_config: bool) -> TaskRef {
    let source = if from_config {
        "explicit (.supertools.toml)"
    } else if kind == AuthorityKind::Cargo {
        "cargo built-in semantics"
    } else {
        "heuristic mapping over declared tasks"
    };
    TaskRef {
        task: task.to_string(),
        command: task_command(kind, task),
        source: source.to_string(),
    }
}

fn task_command(kind: AuthorityKind, task: &str) -> Vec<String> {
    match kind {
        AuthorityKind::Mise => vec!["mise".into(), "run".into(), task.into()],
        AuthorityKind::Just => vec!["just".into(), task.into()],
        AuthorityKind::Package => vec!["<runner>".into(), "run".into(), task.into()],
        AuthorityKind::Cargo => match task {
            "check" => vec!["cargo".into(), "check".into(), "--all-targets".into()],
            "test" => vec!["cargo".into(), "test".into()],
            "clippy" => vec!["cargo".into(), "clippy".into(), "--all-targets".into()],
            "fmt" => vec!["cargo".into(), "fmt".into(), "--".into(), "--check".into()],
            other => vec!["cargo".into(), other.into()],
        },
    }
}

fn mise_tasks(root: &Path, notes: &mut Vec<String>) -> Vec<String> {
    let out = process::run(
        &Request::new("mise", vec!["tasks".into(), "ls".into(), "--json".into()])
            .cwd(root)
            .timeout(Duration::from_secs(30)),
    );
    match out {
        Ok(o) if o.success() => {
            // `mise tasks ls --json` emits an array of task objects.
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(o.stdout.trim()) {
                if let Some(arr) = v.as_array() {
                    return arr
                        .iter()
                        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
                        .collect();
                }
            }
            notes.push("could not parse `mise tasks ls --json` output".into());
            Vec::new()
        }
        Ok(o) => {
            notes.push(format!(
                "`mise tasks ls --json` failed (exit {:?})",
                o.exit_code
            ));
            Vec::new()
        }
        Err(e) => {
            notes.push(format!("mise could not run: {e}"));
            Vec::new()
        }
    }
}

fn just_recipes(root: &Path, notes: &mut Vec<String>) -> Vec<String> {
    let out = process::run(
        &Request::new("just", vec!["--list".into(), "--summary".into()])
            .cwd(root)
            .timeout(Duration::from_secs(30)),
    );
    match out {
        Ok(o) if o.success() => o.stdout.split_whitespace().map(str::to_string).collect(),
        Ok(o) => {
            notes.push(format!(
                "`just --list --summary` failed (exit {:?})",
                o.exit_code
            ));
            Vec::new()
        }
        Err(e) => {
            notes.push(format!("just could not run: {e}"));
            Vec::new()
        }
    }
}

fn package_scripts(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("scripts")
                .and_then(|s| s.as_object())
                .map(|s| s.keys().cloned().collect())
        })
        .unwrap_or_default()
}

fn pick_package_runner(root: &Path, notes: &mut Vec<String>) -> Option<String> {
    let preferred = [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
        ("package-lock.json", "npm"),
    ];
    for (lock, runner) in preferred {
        if root.join(lock).is_file() {
            if process::resolve_program(runner).is_ok() {
                return Some(runner.to_string());
            }
            notes.push(format!("{lock} present but {runner} is not installed"));
        }
    }
    for runner in ["npm", "pnpm", "yarn", "bun"] {
        if process::resolve_program(runner).is_ok() {
            return Some(runner.to_string());
        }
    }
    notes.push("no package runner (npm/pnpm/yarn/bun) found on PATH".into());
    None
}

// ---------------------------------------------------------------- commands

fn json_discovery(d: &Discovery) -> serde_json::Value {
    let mut tasks = d
        .authority
        .as_ref()
        .map(|a| a.tasks.clone())
        .unwrap_or_default();
    let tasks_truncated = tasks.len() > 100;
    tasks.truncate(100);
    json!({
        "root": d.root.to_string_lossy(),
        "config": {
            "path": d.config_path.as_ref().map(|p| p.to_string_lossy()),
            "present": d.config_path.is_some(),
        },
        "authority": d.authority.as_ref().map(|a| json!({
            "kind": a.kind,
            "source": a.source,
            "runner": a.runner,
            "task_count": a.tasks.len(),
            "tasks": tasks,
        })),
        "quick": d.quick,
        "full": d.full,
        "candidates": d.candidates,
        "ambiguous": !d.ambiguity.is_empty(),
        "ambiguity": d.ambiguity,
        "notes": d.notes,
        "tasks_truncated": tasks_truncated,
    })
}

pub fn discover_cmd() -> CmdResult {
    let operation = "verify.discover";
    let d = discover(operation)?;
    let data = json_discovery(&d);
    let config_evidence = d.config_path.as_ref().map(|p| {
        Evidence::file(
            &p.to_string_lossy(),
            "explicit project verification configuration",
        )
    });

    if !d.ambiguity.is_empty() {
        let list = d.ambiguity.join(", ");
        let mut b = Builder::new(
            operation,
            Status::Ambiguous,
            format!("ambiguous verification authorities: {list} (fail closed)"),
        )
        .data(data)
        .warning("Supertools refuses to guess which authority is canonical")
        .next(
            "create .supertools.toml with [verify] authority = \"<mise|just|package|cargo>\"",
            "declare the canonical authority explicitly",
        );
        b = b.line(format!("AMBIGUOUS: {list}"));
        b = b.line("Declare the canonical authority in .supertools.toml:");
        b = b.line("  [verify]");
        b = b.line(format!(
            "  authority = \"{}\"   # choose one",
            d.ambiguity.first().cloned().unwrap_or_default()
        ));
        return Ok(b);
    }

    let Some(a) = &d.authority else {
        let mut b = Builder::no_results(operation, "no verification authority discovered")
            .data(data)
            .line("no verification authority found (no mise/just/package/cargo project files)");
        if let Some(ev) = config_evidence {
            b = b.evidence(ev);
        }
        for n in &d.notes {
            b = b.line(format!("note: {n}"));
        }
        return Ok(b);
    };

    let quick_desc = d
        .quick
        .as_ref()
        .map(|q| format!("{} → {}", q.task, q.command.join(" ")))
        .unwrap_or_else(|| "(none declared)".into());
    let full_desc = d
        .full
        .as_ref()
        .map(|q| format!("{} → {}", q.task, q.command.join(" ")))
        .unwrap_or_else(|| "(none declared)".into());

    let mut b = Builder::ok(
        operation,
        format!(
            "authority: {} ({}); {} declared task(s)",
            a.kind.as_str(),
            a.source,
            a.tasks.len()
        ),
    )
    .data(data)
    .line(format!("authority:  {} ({})", a.kind.as_str(), a.source))
    .line(format!("quick:      {quick_desc}"))
    .line(format!("full:       {full_desc}"))
    .line(format!(
        "tasks ({}): {}",
        a.tasks.len(),
        a.tasks
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if let Some(ev) = config_evidence {
        b = b.evidence(ev);
    }
    for n in &d.notes {
        b = b.warning(n.clone());
    }
    if d.quick.is_none() {
        b = b.warning("no cheap verification task is declared; set quick in .supertools.toml");
    }
    b = b.next(
        "supertools verify quick",
        "run the cheapest authoritative verification",
    );
    Ok(b)
}

fn resolve_task_ref(
    d: &Discovery,
    which: &str,
    operation: &str,
) -> Result<(TaskRef, AuthorityKind, Authority), Failure> {
    let authority = d.authority.clone().ok_or_else(|| {
        Failure::new(
            operation,
            Status::NoResults,
            "no verification authority discovered — nothing to run",
        )
        .hint("supertools verify discover  # see what was looked for")
    })?;
    let tref = match which {
        "quick" => d.quick.clone(),
        "full" => d.full.clone(),
        other => d
            .authority
            .as_ref()
            .and_then(|a| a.tasks.iter().find(|t| *t == other).cloned())
            .map(|t| TaskRef { task: t, command: task_command(authority.kind, other), source: "declared task".into() }),
    }
    .ok_or_else(|| match which {
        "quick" | "full" => Failure::new(
            operation,
            Status::NoResults,
            format!(
                "this project declares no meaningful {which} verification task (authority: {})",
                authority.kind.as_str()
            ),
        )
        .hint("supertools verify discover  # list declared tasks")
        .hint("set quick/full explicitly in .supertools.toml"),
        name => {
            let mut f = Failure::new(
                operation,
                Status::Refused,
                format!(
                    "\"{name}\" is not a declared task of the {} authority — refusing to execute an arbitrary name",
                    authority.kind.as_str()
                ),
            );
            let declared = authority.tasks.iter().take(20).cloned().collect::<Vec<_>>().join(", ");
            f.hints.push(format!("declared tasks: {declared}"));
            f.hints.push("supertools verify discover".into());
            f
        }
    })?;

    // Config-declared quick/full names must also be declared tasks.
    if matches!(which, "quick" | "full")
        && !authority.tasks.is_empty()
        && !authority.tasks.contains(&tref.task)
    {
        return Err(Failure::invalid(
            operation,
            format!(
                ".supertools.toml maps {which} to \"{}\", which is not a declared task (fail closed)",
                tref.task
            ),
        )
        .hint("supertools verify discover"));
    }

    Ok((tref, authority.kind, authority))
}

fn run_task(
    tref: &TaskRef,
    kind: AuthorityKind,
    authority: &Authority,
    root: &Path,
    timeout: Duration,
    diag_cap: usize,
    operation: &str,
) -> CmdResult {
    // Resolve the runner program to an absolute path; substitute for package.
    let mut command = tref.command.clone();
    let program = if kind == AuthorityKind::Package {
        let runner = authority.runner.clone().ok_or_else(|| {
            Failure::unavailable(
                operation,
                "no package runner (npm/pnpm/yarn/bun) available to run this task",
            )
        })?;
        command[0] = runner.clone();
        runner
    } else {
        command[0].clone()
    };
    let resolved = process::resolve_program(&program).map_err(|_| {
        Failure::unavailable(
            operation,
            format!(
                "{program} ({} authority) is not available on PATH",
                kind.as_str()
            ),
        )
        .hint(format!("supertools find {program}"))
    })?;
    let resolved_str = resolved.to_string_lossy().into_owned();

    let req = Request::new(resolved_str.clone(), command[1..].to_vec())
        .cwd(root)
        .timeout(timeout)
        .max_stdout(MAX_STDOUT_CAP)
        .max_stderr(MAX_STDOUT_CAP);
    let outcome = process::run(&req).map_err(|e| {
        Failure::failed(
            operation,
            format!("verification command could not run: {e}"),
        )
    })?;

    let cap = if outcome.exit_code == Some(0) && !outcome.timed_out {
        diag_cap.min(DIAG_TAIL_OK)
    } else {
        diag_cap.max(DIAG_TAIL_FAIL)
    };
    let (stdout_tail, stdout_trunc) = tail(outcome.stdout.trim(), cap);
    let (stderr_tail, stderr_trunc) = tail(outcome.stderr.trim(), cap);
    let passed = outcome.success();
    let truncated =
        outcome.stdout_truncated || outcome.stderr_truncated || stdout_trunc || stderr_trunc;

    let status = if outcome.timed_out {
        Status::Timeout
    } else if passed {
        Status::Ok
    } else {
        Status::Failed
    };
    let summary = if outcome.timed_out {
        format!(
            "verify {task} TIMED OUT after {}s and was killed",
            timeout.as_secs(),
            task = tref.task
        )
    } else if passed {
        format!(
            "verify {} passed in {}ms (exit 0)",
            tref.task, outcome.duration_ms
        )
    } else {
        format!(
            "verify {} FAILED (exit {:?}) in {}ms",
            tref.task, outcome.exit_code, outcome.duration_ms
        )
    };

    let mut full_command = vec![resolved_str.clone()];
    full_command.extend(command[1..].iter().cloned());

    let data = json!({
        "authority": kind.as_str(),
        "authority_source": authority.source,
        "task": tref.task,
        "task_source": tref.source,
        "command": full_command,
        "cwd": root.to_string_lossy(),
        "exit_code": outcome.exit_code,
        "duration_ms": outcome.duration_ms,
        "timed_out": outcome.timed_out,
        "passed": passed,
        "diagnostics": {
            "stdout_tail": stdout_tail,
            "stderr_tail": stderr_tail,
        },
    });

    let mut b = Builder::new(operation, status, summary)
        .data(data)
        .evidence(Evidence::command(
            &resolved_str,
            &command[1..],
            outcome.exit_code,
            outcome.duration_ms,
            outcome.timed_out,
        ))
        .truncated(truncated);

    if passed {
        b = b.line(format!("PASSED  {} ({})", tref.task, command.join(" ")));
        b = b.line(format!("duration: {}ms", outcome.duration_ms));
        if !stdout_tail.is_empty() {
            for l in stdout_tail
                .lines()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                b = b.line(format!("  {l}"));
            }
        }
    } else {
        b = b.line(format!("FAILED  {} ({})", tref.task, command.join(" ")));
        b = b.line(format!(
            "exit: {:?}  duration: {}ms{}",
            outcome.exit_code,
            outcome.duration_ms,
            if outcome.timed_out {
                "  (TIMED OUT)"
            } else {
                ""
            }
        ));
        let fail_lines: Vec<&str> = if !stderr_tail.trim().is_empty() {
            stderr_tail.lines().collect()
        } else {
            stdout_tail.lines().collect()
        };
        for l in fail_lines
            .iter()
            .rev()
            .take(25)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            b = b.line(format!("  {l}"));
        }
    }
    if truncated {
        b = b.warning("diagnostics are bounded; the useful failure tail is preserved, earlier output was dropped");
    }
    Ok(b)
}

fn timeout_for(kind: &str, explicit: Option<u64>) -> Duration {
    let default = match kind {
        "quick" => DEFAULT_QUICK_TIMEOUT,
        "full" => DEFAULT_FULL_TIMEOUT,
        _ => DEFAULT_TASK_TIMEOUT,
    };
    Duration::from_secs(explicit.unwrap_or(default))
}

pub fn quick(timeout: Option<u64>) -> CmdResult {
    let operation = "verify.quick";
    let d = discover(operation)?;
    ensure_unambiguous(&d, operation)?;
    let cfg_timeout = config_timeout(&d);
    let diag_cap = diag_cap(&d);
    let (tref, kind, authority) = resolve_task_ref(&d, "quick", operation)?;
    run_task(
        &tref,
        kind,
        &authority,
        &d.root,
        timeout_for("quick", timeout.or(cfg_timeout)),
        diag_cap,
        operation,
    )
}

pub fn full(timeout: Option<u64>) -> CmdResult {
    let operation = "verify.full";
    let d = discover(operation)?;
    ensure_unambiguous(&d, operation)?;
    let cfg_timeout = config_timeout(&d);
    let diag_cap = diag_cap(&d);
    let (tref, kind, authority) = resolve_task_ref(&d, "full", operation)?;
    run_task(
        &tref,
        kind,
        &authority,
        &d.root,
        timeout_for("full", timeout.or(cfg_timeout)),
        diag_cap,
        operation,
    )
}

pub fn task(name: String, timeout: Option<u64>) -> CmdResult {
    let operation = "verify.task";
    let d = discover(operation)?;
    ensure_unambiguous(&d, operation)?;
    let cfg_timeout = config_timeout(&d);
    let diag_cap = diag_cap(&d);
    let (tref, kind, authority) = resolve_task_ref(&d, &name, operation)?;
    run_task(
        &tref,
        kind,
        &authority,
        &d.root,
        timeout_for("task", timeout.or(cfg_timeout)),
        diag_cap,
        operation,
    )
}

fn diag_cap(d: &Discovery) -> usize {
    d.config
        .as_ref()
        .and_then(|c| c.limits.output_bytes)
        .unwrap_or(DIAG_TAIL_FAIL)
}

fn config_timeout(d: &Discovery) -> Option<u64> {
    d.config.as_ref().and_then(|c| c.verify.timeout_seconds)
}

fn ensure_unambiguous(d: &Discovery, operation: &str) -> Result<(), Failure> {
    if d.ambiguity.is_empty() {
        return Ok(());
    }
    let list = d.ambiguity.join(", ");
    Err(Failure::new(
        operation,
        Status::Ambiguous,
        format!("ambiguous verification authorities ({list}); refusing to guess (fail closed)"),
    )
    .hint("declare the canonical authority in .supertools.toml under [verify]")
    .hint("supertools verify discover  # full explanation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let p = dir.path().join(name);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, content).unwrap();
        }
        dir
    }

    fn discover_fixture(files: &[(&str, &str)]) -> Discovery {
        let dir = fixture(files);
        discover_in(dir.path(), "verify.discover").unwrap()
    }

    #[test]
    fn cargo_authority_maps_quick_check_full_test() {
        let d = discover_fixture(&[("Cargo.toml", "[package]\nname=\"x\"\nversion=\"0.1.0\"\n")]);
        assert_eq!(d.authority.as_ref().unwrap().kind, AuthorityKind::Cargo);
        assert_eq!(d.quick.as_ref().unwrap().task, "check");
        assert_eq!(d.full.as_ref().unwrap().task, "test");
        assert!(d.ambiguity.is_empty());
    }

    #[test]
    fn conflicting_runners_fail_closed() {
        let d = discover_fixture(&[
            ("justfile", "check:\n    echo ok\n"),
            ("package.json", "{\"scripts\":{\"test\":\"echo ok\"}}"),
        ]);
        assert!(d.authority.is_none());
        assert!(d.ambiguity.len() >= 2);
    }

    #[test]
    fn config_resolves_ambiguity() {
        let d = discover_fixture(&[
            ("justfile", "check:\n    echo ok\n"),
            ("package.json", "{\"scripts\":{\"test\":\"echo ok\"}}"),
            (".supertools.toml", "[verify]\nauthority = \"just\"\n"),
        ]);
        assert_eq!(d.authority.as_ref().unwrap().kind, AuthorityKind::Just);
        assert!(d.ambiguity.is_empty());
    }

    #[test]
    fn package_scripts_discovered() {
        let d = discover_fixture(&[(
            "package.json",
            "{\"scripts\":{\"lint\":\"eslint .\",\"test\":\"vitest\"}}",
        )]);
        let a = d.authority.as_ref().unwrap();
        assert_eq!(a.kind, AuthorityKind::Package);
        assert!(a.tasks.contains(&"test".to_string()));
        assert_eq!(d.full.as_ref().unwrap().task, "test");
        assert!(d.quick.is_none(), "no cheap task declared by this project");
    }

    #[test]
    fn package_without_scripts_is_not_an_authority() {
        let d = discover_fixture(&[
            ("package.json", "{\"name\":\"docs\"}"),
            ("Cargo.toml", "[package]\nname=\"x\"\nversion=\"0.1.0\"\n"),
        ]);
        assert_eq!(d.authority.as_ref().unwrap().kind, AuthorityKind::Cargo);
        assert!(d.ambiguity.is_empty());
    }

    #[test]
    fn malformed_config_fails_closed() {
        let dir = fixture(&[
            ("Cargo.toml", "[package]\nname=\"x\"\nversion=\"0.1.0\"\n"),
            (".supertools.toml", "[verify\nauthority = "),
        ]);
        let err = discover_in(dir.path(), "verify.discover").unwrap_err();
        assert_eq!(err.status, Status::InvalidRequest);
    }

    #[test]
    fn unknown_authority_in_config_fails_closed() {
        let dir = fixture(&[(".supertools.toml", "[verify]\nauthority = \"make\"\n")]);
        let err = discover_in(dir.path(), "verify.discover").unwrap_err();
        assert_eq!(err.status, Status::InvalidRequest);
    }
}
