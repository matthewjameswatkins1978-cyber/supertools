//! Repo domain: trustworthy Git state via stable machine-readable interfaces
//! (`status --porcelain=v2 -z`, `log -z`, `ls-files -z`). All operations are
//! strictly read-only. Optional GitHub information via `gh --json`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::output::{Builder, CmdResult, Evidence, Failure};
use crate::process::{self, head, Request};
use crate::registry::{quick_require, ToolId};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GH_TIMEOUT: Duration = Duration::from_secs(45);

fn git(args: Vec<String>, operation: &str) -> Result<process::Outcome, Failure> {
    let program = quick_require(ToolId::Git, operation)?;
    let req = Request::new(program, args).timeout(GIT_TIMEOUT);
    let outcome = process::run(&req)
        .map_err(|e| Failure::failed(operation, format!("git could not run: {e}")))?;
    if outcome.exit_code == Some(128)
        && (outcome.stderr.contains("not a git repository")
            || outcome.stderr.contains("cannot change to"))
    {
        return Err(Failure::failed(operation, "not inside a git repository")
            .hint("supertools repo state confirms this once git is available")
            .hint("git init  # if this directory should be a repository"));
    }
    Ok(outcome)
}

fn git_ok(args: Vec<String>, operation: &str) -> Result<process::Outcome, Failure> {
    let outcome = git(args.clone(), operation)?;
    if !outcome.success() {
        return Err(Failure::failed(
            operation,
            format!(
                "git {} failed (exit {:?}): {}",
                args.first().cloned().unwrap_or_default(),
                outcome.exit_code,
                head(outcome.stderr.trim(), 400).0
            ),
        ));
    }
    Ok(outcome)
}

// ------------------------------------------------------- porcelain v2 model

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Ordinary,
    Rename,
    Copy,
    Unmerged,
    Untracked,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub kind: EntryKind,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub conflict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_change: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_change: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_score: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GitStatus {
    pub oid: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    pub entries: Vec<StatusEntry>,
}

impl GitStatus {
    pub fn staged_count(&self) -> usize {
        self.entries.iter().filter(|e| e.staged).count()
    }
    pub fn unstaged_count(&self) -> usize {
        self.entries.iter().filter(|e| e.unstaged).count()
    }
    pub fn untracked_count(&self) -> usize {
        self.entries.iter().filter(|e| e.untracked).count()
    }
    pub fn conflict_count(&self) -> usize {
        self.entries.iter().filter(|e| e.conflict).count()
    }
    pub fn dirty(&self) -> bool {
        !self.entries.is_empty()
    }
    pub fn detached(&self) -> bool {
        self.head.as_deref() == Some("(detached)")
    }
}

fn change_word(c: char) -> Option<String> {
    match c {
        '.' => None,
        'M' => Some("modified".into()),
        'A' => Some("added".into()),
        'D' => Some("deleted".into()),
        'R' => Some("renamed".into()),
        'C' => Some("copied".into()),
        'T' => Some("type_changed".into()),
        'U' => Some("unmerged".into()),
        other => Some(format!("other({other})")),
    }
}

/// Parse `git status --porcelain=v2 -z` output. Pure function; tested
/// directly against synthetic NUL-separated fixtures.
pub fn parse_porcelain_v2_z(raw: &str) -> GitStatus {
    let mut st = GitStatus::default();
    let records: Vec<&str> = raw.split('\0').collect();
    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;
        if rec.is_empty() {
            continue;
        }
        if let Some(header) = rec.strip_prefix("# branch.") {
            let mut parts = header.splitn(2, ' ');
            let key = parts.next().unwrap_or("");
            let val = parts.next().unwrap_or("");
            match key {
                "oid" => st.oid = Some(val.to_string()),
                "head" => st.head = Some(val.to_string()),
                "upstream" => st.upstream = Some(val.to_string()),
                "ab" => {
                    // "+<ahead> -<behind>"
                    let mut it = val.split_whitespace();
                    if let Some(a) = it
                        .next()
                        .and_then(|s| s.trim_start_matches('+').parse::<u64>().ok())
                    {
                        st.ahead = Some(a);
                    }
                    if let Some(b) = it
                        .next()
                        .and_then(|s| s.trim_start_matches('-').parse::<u64>().ok())
                    {
                        st.behind = Some(b);
                    }
                }
                _ => {}
            }
            continue;
        }
        let mut c = rec.chars();
        match c.next() {
            Some('?') => {
                st.entries.push(StatusEntry {
                    path: rec[2..].to_string(),
                    orig_path: None,
                    kind: EntryKind::Untracked,
                    staged: false,
                    unstaged: false,
                    untracked: true,
                    conflict: false,
                    index_change: None,
                    worktree_change: None,
                    rename_score: None,
                });
            }
            Some('!') => { /* ignored entries are not requested */ }
            Some('1') => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                if f.len() < 9 {
                    continue;
                }
                let xy: Vec<char> = f[1].chars().collect();
                let (x, y) = (
                    xy.first().copied().unwrap_or('.'),
                    xy.get(1).copied().unwrap_or('.'),
                );
                st.entries.push(StatusEntry {
                    path: f[8].to_string(),
                    orig_path: None,
                    kind: EntryKind::Ordinary,
                    staged: x != '.',
                    unstaged: y != '.',
                    untracked: false,
                    conflict: false,
                    index_change: change_word(x),
                    worktree_change: change_word(y),
                    rename_score: None,
                });
            }
            Some('2') => {
                let f: Vec<&str> = rec.splitn(10, ' ').collect();
                if f.len() < 10 {
                    continue;
                }
                let xy: Vec<char> = f[1].chars().collect();
                let (x, y) = (
                    xy.first().copied().unwrap_or('.'),
                    xy.get(1).copied().unwrap_or('.'),
                );
                let orig = if i < records.len() {
                    let o = records[i].to_string();
                    i += 1;
                    Some(o)
                } else {
                    None
                };
                st.entries.push(StatusEntry {
                    path: f[9].to_string(),
                    orig_path: orig,
                    kind: if f[8].starts_with('R') {
                        EntryKind::Rename
                    } else {
                        EntryKind::Copy
                    },
                    staged: x != '.',
                    unstaged: y != '.',
                    untracked: false,
                    conflict: false,
                    index_change: change_word(x),
                    worktree_change: change_word(y),
                    rename_score: f[8].get(1..).and_then(|s| s.parse::<u32>().ok()),
                });
            }
            Some('u') => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                if f.len() < 9 {
                    continue;
                }
                st.entries.push(StatusEntry {
                    path: f[8].to_string(),
                    orig_path: None,
                    kind: EntryKind::Unmerged,
                    staged: false,
                    unstaged: false,
                    untracked: false,
                    conflict: true,
                    index_change: Some("unmerged".into()),
                    worktree_change: None,
                    rename_score: None,
                });
            }
            _ => {}
        }
    }
    st
}

fn run_status(operation: &str) -> Result<(GitStatus, Evidence), Failure> {
    let args = vec![
        "status".to_string(),
        "--porcelain=v2".to_string(),
        "--branch".to_string(),
        "-z".to_string(),
    ];
    let outcome = git_ok(args.clone(), operation)?;
    let ev = Evidence::command(
        "git",
        &args,
        outcome.exit_code,
        outcome.duration_ms,
        outcome.timed_out,
    );
    Ok((parse_porcelain_v2_z(&outcome.stdout), ev))
}

fn repo_root(operation: &str) -> Result<String, Failure> {
    let args = vec!["rev-parse".to_string(), "--show-toplevel".to_string()];
    let outcome = git_ok(args, operation)?;
    Ok(outcome.stdout.trim().to_string())
}

fn detect_operation_state(operation: &str) -> Option<String> {
    let args = vec!["rev-parse".to_string(), "--absolute-git-dir".to_string()];
    let outcome = git(args, operation).ok()?;
    if !outcome.success() {
        return None;
    }
    let dir = PathBuf::from(outcome.stdout.trim());
    let checks = [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry_pick"),
        ("REVERT_HEAD", "revert"),
        ("BISECT_LOG", "bisect"),
    ];
    for (file, name) in checks {
        if dir.join(file).exists() {
            return Some(name.to_string());
        }
    }
    None
}

// ------------------------------------------------------------- repo state

pub fn state() -> CmdResult {
    let operation = "repo.state";
    let root = repo_root(operation)?;
    let (st, ev) = run_status(operation)?;
    let op_state = detect_operation_state(operation);

    let mut remote_url: Option<String> = None;
    let mut github: Option<(String, String)> = None;
    if let Ok(out) = git(
        vec![
            "remote".to_string(),
            "get-url".to_string(),
            "origin".to_string(),
        ],
        operation,
    ) {
        if out.success() {
            let url = out.stdout.trim().to_string();
            github = parse_github_remote(&url);
            remote_url = Some(url);
        }
    }

    let branch = st.head.clone();
    let dirty = st.dirty();
    let summary = if dirty {
        format!(
            "branch {} — dirty ({} staged, {} unstaged, {} untracked{})",
            branch.clone().unwrap_or_else(|| "?".into()),
            st.staged_count(),
            st.unstaged_count(),
            st.untracked_count(),
            if st.conflict_count() > 0 {
                format!(", {} conflicted", st.conflict_count())
            } else {
                String::new()
            }
        )
    } else {
        format!(
            "branch {} — clean",
            branch.clone().unwrap_or_else(|| "?".into())
        )
    };

    let data = json!({
        "root": root,
        "branch": if st.detached() { serde_json::Value::Null } else { branch.clone().into() },
        "head": st.oid,
        "detached": st.detached(),
        "upstream": st.upstream,
        "ahead": st.ahead,
        "behind": st.behind,
        "dirty": dirty,
        "staged": st.staged_count(),
        "unstaged": st.unstaged_count(),
        "untracked": st.untracked_count(),
        "conflicts": st.conflict_count(),
        "operation_state": op_state.clone().unwrap_or_else(|| "none".into()),
        "remote_url": remote_url,
        "github": github.map(|(o, r)| json!({"owner": o, "repo": r})),
    });

    let mut b = Builder::ok(operation, summary)
        .data(data)
        .evidence(ev)
        .line(format!(
            "branch: {}{}  head: {}",
            branch.unwrap_or_else(|| "?".into()),
            if st.detached() { " (detached)" } else { "" },
            st.oid.as_deref().map(|s| head(s, 12).0).unwrap_or_default()
        ))
        .line(format!(
            "dirty: {dirty}  staged: {}  unstaged: {}  untracked: {}  conflicts: {}",
            st.staged_count(),
            st.unstaged_count(),
            st.untracked_count(),
            st.conflict_count()
        ));
    if let Some(u) = &st.upstream {
        b = b.line(format!(
            "upstream: {u}  ahead: {}  behind: {}",
            st.ahead
                .map(|a| a.to_string())
                .unwrap_or_else(|| "?".into()),
            st.behind
                .map(|a| a.to_string())
                .unwrap_or_else(|| "?".into())
        ));
    }
    if op_state.is_some() {
        b = b.line(format!(
            "in-progress operation: {}",
            op_state.clone().unwrap_or_default()
        ));
    }
    b = b.line(format!("root: {root}"));
    Ok(b)
}

// ------------------------------------------------------------ repo changed

pub fn changed(limit: usize) -> CmdResult {
    let operation = "repo.changed";
    let (st, ev) = run_status(operation)?;
    let total = st.entries.len();
    let truncated = total > limit;
    let staged = st.staged_count();
    let unstaged = st.unstaged_count();
    let untracked = st.untracked_count();
    let conflicts = st.conflict_count();
    let entries: Vec<StatusEntry> = st.entries.into_iter().take(limit).collect();

    let mut b =
        if total == 0 {
            Builder::no_results(operation, "no changed files — working tree is clean")
        } else {
            Builder::ok(
                operation,
                format!(
                "{} changed file(s): {staged} staged, {unstaged} unstaged, {untracked} untracked{}",
                total,
                if conflicts > 0 { format!(", {conflicts} conflicted") } else { String::new() }
            ),
            )
        };
    b.data = json!({
        "total": total,
        "staged": staged,
        "unstaged": unstaged,
        "untracked": untracked,
        "conflicts": conflicts,
        "files": entries,
    });
    for e in entries.iter().take(30) {
        let mut flags = String::new();
        if e.staged {
            flags.push('S');
        }
        if e.unstaged {
            flags.push('U');
        }
        if e.untracked {
            flags.push('?');
        }
        if e.conflict {
            flags.push('!');
        }
        let rename = e
            .orig_path
            .as_ref()
            .map(|o| format!(" (from {o})"))
            .unwrap_or_default();
        b = b.line(format!("[{flags:>3}] {}{rename}", e.path));
    }
    b = b.evidence(ev).truncated(truncated);
    if truncated {
        b = b.warning(format!(
            "showing {limit} of {total} entries; raise --limit to see more"
        ));
    }
    Ok(b)
}

// ------------------------------------------------------------ repo history

pub fn history(target: Option<String>, limit: usize) -> CmdResult {
    let operation = "repo.history";
    let mut args = vec![
        "log".to_string(),
        "-z".to_string(),
        format!("-n{}", limit + 1),
        "--no-color".to_string(),
        "--pretty=format:%H%x1f%an%x1f%aI%x1f%D%x1f%s".to_string(),
    ];
    if let Some(t) = &target {
        args.push("--".to_string());
        args.push(t.clone());
    }
    let outcome = git_ok(args.clone(), operation)?;
    let ev = Evidence::command(
        "git",
        &args,
        outcome.exit_code,
        outcome.duration_ms,
        outcome.timed_out,
    );

    let mut commits = Vec::new();
    for rec in outcome.stdout.split('\0') {
        if rec.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = rec.split('\x1f').collect();
        if f.len() < 5 {
            continue;
        }
        commits.push(json!({
            "sha": f[0],
            "author": f[1],
            "timestamp": f[2],
            "refs": if f[3].is_empty() { serde_json::Value::Null } else { json!(f[3]) },
            "subject": f[4],
        }));
    }
    let more = commits.len() > limit;
    if more {
        commits.truncate(limit);
    }

    let mut b = if commits.is_empty() {
        Builder::no_results(
            operation,
            if target.is_some() {
                "no commits touch this target"
            } else {
                "no commits yet"
            },
        )
    } else {
        Builder::ok(
            operation,
            format!("{} most recent commit(s)", commits.len()),
        )
    };
    b.data = json!({
        "count": commits.len(),
        "target": target,
        "commits": commits.clone(),
    });
    for c in commits.iter().take(20) {
        b = b.line(format!(
            "{} {} {}{}",
            head(c.get("sha").and_then(|s| s.as_str()).unwrap_or("?"), 8).0,
            c.get("timestamp").and_then(|s| s.as_str()).unwrap_or("?"),
            c.get("subject").and_then(|s| s.as_str()).unwrap_or(""),
            c.get("refs")
                .and_then(|r| r.as_str())
                .map(|r| format!("  [{r}]"))
                .unwrap_or_default()
        ));
    }
    b = b.evidence(ev).truncated(more);
    if more {
        b = b.warning(format!(
            "history bounded at {limit}; raise --limit for more"
        ));
    }
    Ok(b)
}

// ------------------------------------------------------- GitHub via gh CLI

pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix(".git").unwrap_or(u);
    // https://github.com/owner/repo | ssh://git@github.com/owner/repo
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
    ] {
        if let Some(rest) = u.strip_prefix(prefix) {
            let mut parts = rest.split('/');
            if let (Some(owner), Some(repo)) = (parts.next(), parts.next()) {
                if !owner.is_empty() && !repo.is_empty() {
                    return Some((owner.to_string(), repo.to_string()));
                }
            }
        }
    }
    // git@github.com:owner/repo
    if let Some(rest) = u.strip_prefix("git@github.com:") {
        let mut parts = rest.split('/');
        if let (Some(owner), Some(repo)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !repo.is_empty() {
                return Some((owner.to_string(), repo.to_string()));
            }
        }
    }
    None
}

pub fn remote() -> CmdResult {
    let operation = "repo.remote";
    let names_out = git_ok(vec!["remote".to_string()], operation)?;
    let names: Vec<String> = names_out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let mut remotes = Vec::new();
    for name in &names {
        let url_out = git(
            vec!["remote".to_string(), "get-url".to_string(), name.clone()],
            operation,
        )?;
        let url = if url_out.success() {
            url_out.stdout.trim().to_string()
        } else {
            String::new()
        };
        let gh = parse_github_remote(&url);
        remotes.push(json!({
            "name": name,
            "url": url,
            "github": gh.is_some(),
            "owner": gh.as_ref().map(|(o, _)| o.clone()),
            "repo": gh.as_ref().map(|(_, r)| r.clone()),
        }));
    }
    let mut b = if remotes.is_empty() {
        Builder::no_results(operation, "no remotes configured")
    } else {
        Builder::ok(operation, format!("{} remote(s)", remotes.len()))
    };
    b.data = json!({ "remotes": remotes.clone() });
    for r in &remotes {
        b = b.line(format!(
            "{}  {}{}",
            r.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            r.get("url").and_then(|v| v.as_str()).unwrap_or(""),
            if r.get("github").and_then(|v| v.as_bool()).unwrap_or(false) {
                "  (github)"
            } else {
                ""
            }
        ));
    }
    b = b.next(
        "supertools repo pr",
        "PR state for the current branch (needs gh + auth)",
    );
    Ok(b)
}

fn github_context(operation: &str) -> Result<(String, String), Failure> {
    // Requires: git repo, a GitHub remote, gh installed, gh authenticated.
    let names_out = git_ok(vec!["remote".to_string()], operation)?;
    let names: Vec<String> = names_out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let mut found: Option<(String, String)> = None;
    for name in &names {
        if let Ok(out) = git(
            vec!["remote".to_string(), "get-url".to_string(), name.clone()],
            operation,
        ) {
            if out.success() {
                if let Some(gh) = parse_github_remote(out.stdout.trim()) {
                    if name == "origin" || found.is_none() {
                        found = Some(gh);
                    }
                }
            }
        }
    }
    let Some(gh_repo) = found else {
        return Err(
            Failure::unavailable(operation, "no GitHub remote found on this repository")
                .hint("supertools repo remote  # inspect configured remotes"),
        );
    };

    let gh = quick_require(ToolId::Gh, operation)?;
    let auth = process::run(
        &Request::new(gh.clone(), vec!["auth".to_string(), "status".to_string()])
            .timeout(GH_TIMEOUT),
    )
    .map_err(|e| Failure::failed(operation, format!("gh could not run: {e}")))?;
    if !auth.success() {
        return Err(
            Failure::unavailable(operation, "gh is not authenticated in this environment")
                .hint("gh auth login"),
        );
    }
    Ok((gh, format!("{}/{}", gh_repo.0, gh_repo.1)))
}

const PR_FIELDS: &str = "number,title,state,isDraft,url,author,baseRefName,headRefName,mergeable,reviewDecision,statusCheckRollup";

/// `gh pr view` requires a positional `<number> | <url> | <branch>` selector
/// whenever `--repo` is passed explicitly (gh: "argument required when using
/// the --repo flag"). The current branch is the selector; the repository
/// identity comes from the existing parsed-remote authority.
fn pr_view_args(branch: &str, repo: &str) -> Vec<String> {
    vec![
        "pr".to_string(),
        "view".to_string(),
        branch.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        PR_FIELDS.to_string(),
    ]
}

pub fn pr() -> CmdResult {
    let operation = "repo.pr";
    // Branch truth comes from the same porcelain v2 status authority as
    // repo.state — never a second parser. Checked before any gh probing so a
    // detached HEAD is answered locally, without tools or network.
    let (st, status_ev) = run_status(operation)?;
    let Some(branch) = st.head.clone().filter(|_| !st.detached()) else {
        return Ok(Builder::no_results(
            operation,
            "no current branch to select a PR for (detached HEAD)",
        )
        .evidence(status_ev)
        .line("detached HEAD: gh pr view needs <number> | <url> | <branch> — Supertools will not guess one"));
    };
    let (gh, repo) = github_context(operation)?;
    let args = pr_view_args(&branch, &repo);
    let outcome = process::run(
        &Request::new(gh.clone(), args.clone())
            .timeout(GH_TIMEOUT)
            .max_stdout(512 * 1024),
    )
    .map_err(|e| Failure::failed(operation, format!("gh could not run: {e}")))?;
    let ev = Evidence::command(
        &gh,
        &args,
        outcome.exit_code,
        outcome.duration_ms,
        outcome.timed_out,
    );
    if !outcome.success() {
        let msg = outcome.stderr.to_lowercase();
        if msg.contains("no pull requests found") || msg.contains("could not find") {
            return Ok(
                Builder::no_results(operation, "no PR for the current branch")
                    .evidence(status_ev)
                    .evidence(ev)
                    .line("no open PR found for the current branch"),
            );
        }
        return Err(Failure::failed(
            operation,
            format!(
                "gh pr view failed (exit {:?}): {}",
                outcome.exit_code,
                head(outcome.stderr.trim(), 400).0
            ),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(outcome.stdout.trim())
        .map_err(|e| Failure::failed(operation, format!("could not parse gh JSON: {e}")))?;
    let title = parsed.get("title").and_then(|t| t.as_str()).unwrap_or("?");
    let state = parsed.get("state").and_then(|t| t.as_str()).unwrap_or("?");
    let number = parsed.get("number").and_then(|t| t.as_u64()).unwrap_or(0);
    let b = Builder::ok(operation, format!("PR #{number} \"{title}\" — {state}"))
        .data(parsed.clone())
        .evidence(status_ev)
        .evidence(ev)
        .line(format!("#{number} {title}"))
        .line(format!(
            "state: {state}  draft: {}  mergeable: {}",
            parsed
                .get("isDraft")
                .and_then(|d| d.as_bool())
                .unwrap_or(false),
            parsed
                .get("mergeable")
                .and_then(|m| m.as_str())
                .unwrap_or("?")
        ))
        .line(
            parsed
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
        );
    Ok(b)
}

pub fn ci(limit: usize) -> CmdResult {
    let operation = "repo.ci";
    let (gh, repo) = github_context(operation)?;
    let args = vec![
        "run".to_string(),
        "list".to_string(),
        "--repo".to_string(),
        repo.clone(),
        "--limit".to_string(),
        limit.to_string(),
        "--json".to_string(),
        "databaseId,name,displayTitle,status,conclusion,headBranch,event,createdAt,url".to_string(),
    ];
    let outcome = process::run(
        &Request::new(gh.clone(), args.clone())
            .timeout(GH_TIMEOUT)
            .max_stdout(512 * 1024),
    )
    .map_err(|e| Failure::failed(operation, format!("gh could not run: {e}")))?;
    let ev = Evidence::command(
        &gh,
        &args,
        outcome.exit_code,
        outcome.duration_ms,
        outcome.timed_out,
    );
    if !outcome.success() {
        return Err(Failure::failed(
            operation,
            format!(
                "gh run list failed (exit {:?}): {}",
                outcome.exit_code,
                head(outcome.stderr.trim(), 400).0
            ),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(outcome.stdout.trim())
        .map_err(|e| Failure::failed(operation, format!("could not parse gh JSON: {e}")))?;
    let runs = parsed.as_array().cloned().unwrap_or_default();
    let mut b = if runs.is_empty() {
        Builder::no_results(operation, "no workflow runs found")
    } else {
        Builder::ok(operation, format!("{} recent workflow run(s)", runs.len()))
    };
    b.data = json!({ "runs": runs.clone(), "repo": repo });
    for r in runs.iter().take(15) {
        b = b.line(format!(
            "{:<28} {:<12} {:<10} {} — {}",
            r.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            r.get("headBranch").and_then(|v| v.as_str()).unwrap_or("?"),
            r.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
            r.get("conclusion").and_then(|v| v.as_str()).unwrap_or("-"),
            head(
                r.get("displayTitle").and_then(|v| v.as_str()).unwrap_or(""),
                60
            )
            .0
        ));
    }
    b = b.evidence(ev);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_status() {
        let raw = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\0";
        let st = parse_porcelain_v2_z(raw);
        assert_eq!(st.oid.as_deref(), Some("abc123"));
        assert_eq!(st.head.as_deref(), Some("main"));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!(st.ahead, Some(0));
        assert_eq!(st.behind, Some(0));
        assert!(!st.dirty());
        assert!(!st.detached());
    }

    #[test]
    fn parse_detached_and_ab() {
        let raw = "# branch.oid deadbeef\0# branch.head (detached)\0# branch.ab +2 -3\0";
        let st = parse_porcelain_v2_z(raw);
        assert!(st.detached());
        assert_eq!(st.ahead, Some(2));
        assert_eq!(st.behind, Some(3));
    }

    #[test]
    fn parse_mixed_entries_with_spaces_and_unicode() {
        let raw = "1 M. N... 100644 100644 100644 aaa bbb staged file.txt\0\
                   1 .M N... 100644 100644 100644 ccc ddd dir/ünïcode name.rs\0\
                   ? untracked file with spaces.md\0\
                   u UU N... 100644 100644 100644 eee fff conflicted.txt\0";
        let st = parse_porcelain_v2_z(raw);
        assert_eq!(st.entries.len(), 4);
        assert_eq!(st.staged_count(), 1);
        assert_eq!(st.unstaged_count(), 1);
        assert_eq!(st.untracked_count(), 1);
        assert_eq!(st.conflict_count(), 1);
        assert_eq!(st.entries[0].path, "staged file.txt");
        assert_eq!(st.entries[1].path, "dir/ünïcode name.rs");
        assert!(st.dirty());
    }

    #[test]
    fn parse_rename_takes_origin_from_next_record() {
        // porcelain v2 "2" record: 2 XY sub mH mI mW hH hI X<score> path \0 origPath
        let raw =
            "2 R. N... 100644 100644 100644 aaaa1111 bbbb2222 R100 new name.txt\0old name.txt\0";
        let st = parse_porcelain_v2_z(raw);
        assert_eq!(st.entries.len(), 1);
        let e = &st.entries[0];
        assert!(matches!(e.kind, EntryKind::Rename));
        assert_eq!(e.path, "new name.txt");
        assert_eq!(e.orig_path.as_deref(), Some("old name.txt"));
        assert_eq!(e.rename_score, Some(100));
        assert!(e.staged);
    }

    #[test]
    fn github_remote_parsing() {
        assert_eq!(
            parse_github_remote("https://github.com/o/r.git"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(
            parse_github_remote("git@github.com:o/r.git"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(
            parse_github_remote("ssh://git@github.com/o/r"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(parse_github_remote("https://gitlab.com/o/r.git"), None);
        // Malformed / unsupported identities never produce a repo value.
        assert_eq!(parse_github_remote("https://github.com/o/"), None);
        assert_eq!(parse_github_remote("https://github.com//r"), None);
        assert_eq!(parse_github_remote("https://github.com"), None);
        assert_eq!(parse_github_remote("git@gitlab.com:o/r.git"), None);
        assert_eq!(parse_github_remote(""), None);
    }

    #[test]
    fn pr_view_args_carry_branch_selector_and_repo_value() {
        let args = pr_view_args("feature/x", "o/r");
        assert_eq!(
            args,
            vec![
                "pr".to_string(),
                "view".to_string(),
                "feature/x".to_string(),
                "--repo".to_string(),
                "o/r".to_string(),
                "--json".to_string(),
                PR_FIELDS.to_string(),
            ]
        );
        // gh requires the positional selector when --repo is explicit.
        assert_eq!(args[2], "feature/x");
        // --repo is never valueless, never empty.
        let i = args.iter().position(|a| a == "--repo").unwrap();
        assert!(!args[i + 1].is_empty());
        assert!(args[i + 1].contains('/'));
    }
}
