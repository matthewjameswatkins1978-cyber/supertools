//! Search domain: bounded, predictable repository discovery.
//!
//! Backends: ripgrep (text/context/symbol), fd preferred for files with a
//! bounded `git ls-files` fallback, ast-grep for structural. All queries are
//! passed as argument vectors — never through a shell.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::output::{Builder, CmdResult, Evidence, Failure, Status};
use crate::process::{self, head, Request};
use crate::registry::{quick_require, ToolId};

pub const MAX_CONTEXT_LINES: usize = 10;
const LINE_CAP_BYTES: usize = 500;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);

// Fallback walker limits (bounded reasoning).
const FALLBACK_MAX_FILES: usize = 2000;
const FALLBACK_MAX_FILE_BYTES: u64 = 1024 * 1024;

pub struct CommonOpts {
    pub query: String,
    pub limit: usize,
    pub path: Option<PathBuf>,
    pub fixed: bool,
    pub globs: Vec<String>,
    pub context: usize,
}

// ---------------------------------------------------------------- rg backend

fn rg_json_search(
    rg: &str,
    query: &str,
    opts: &CommonOpts,
    context: usize,
    limit: usize,
    operation: &str,
) -> CmdResult {
    let mut args = vec![
        "--json".to_string(),
        "--color".to_string(),
        "never".to_string(),
    ];
    if opts.fixed {
        args.push("--fixed-strings".to_string());
    }
    if context > 0 {
        args.push("--context".to_string());
        args.push(context.to_string());
    }
    for g in &opts.globs {
        args.push("--glob".to_string());
        args.push(g.clone());
    }
    args.push("-e".to_string());
    args.push(query.to_string());
    args.push(
        opts.path
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .into_owned(),
    );

    let req = Request::new(rg.to_string(), args.clone())
        .timeout(SEARCH_TIMEOUT)
        .max_stdout(8 * 1024 * 1024);
    let outcome = process::run(&req)
        .map_err(|e| Failure::failed(operation, format!("ripgrep could not run: {e}")))?;

    if outcome.timed_out {
        return Err(Failure::new(
            operation,
            Status::Timeout,
            format!(
                "ripgrep exceeded {}s and was killed",
                SEARCH_TIMEOUT.as_secs()
            ),
        ));
    }
    // rg: 0 = matches, 1 = no matches, 2+ = error.
    if let Some(code) = outcome.exit_code {
        if code >= 2 {
            return Err(Failure::failed(
                operation,
                format!(
                    "ripgrep failed (exit {code}): {}",
                    head(outcome.stderr.trim(), 400).0
                ),
            ));
        }
    }

    let mut matches_out: Vec<serde_json::Value> = Vec::new();
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut current_block: Option<(String, Vec<serde_json::Value>)> = None;
    let mut truncated = outcome.stdout_truncated;
    let mut match_events = 0usize;

    for line in outcome.stdout.lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let kind = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "match" && kind != "context" {
            continue;
        }
        let Some(data) = msg.get("data") else {
            continue;
        };
        let path = json_text_field(data, "path").unwrap_or_else(|| "<non-utf8-path>".to_string());
        let path = norm_search_path(&path);
        let text = json_text_field(data, "lines").unwrap_or_default();
        let line_no = data
            .get("line_number")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let (text, line_trunc) = head(text.trim_end_matches(['\n', '\r']), LINE_CAP_BYTES);
        if line_trunc {
            truncated = true;
        }

        if kind == "match" {
            match_events += 1;
            let column = data
                .get("submatches")
                .and_then(|s| s.get(0))
                .and_then(|s| s.get("start"))
                .and_then(|s| s.as_u64())
                .map(|byte_off| byte_offset_to_column(data, byte_off));
            let matched = data
                .get("submatches")
                .and_then(|s| s.get(0))
                .and_then(|s| s.get("match"))
                .and_then(|m| json_text_field(m, "text").as_deref().map(str::to_string))
                .or_else(|| json_text_field_of(data.get("submatches")?.get(0)?, "match"));
            if context > 0 {
                let start_new = match &current_block {
                    Some((p, lines)) => {
                        *p != path
                            || lines
                                .last()
                                .and_then(|l| l.get("line").and_then(|n| n.as_u64()))
                                .map(|last| line_no > last + 1)
                                .unwrap_or(true)
                    }
                    None => true,
                };
                if start_new {
                    if let Some((p, lines)) = current_block.take() {
                        blocks.push(json!({"path": p, "lines": lines}));
                    }
                    current_block = Some((path.clone(), Vec::new()));
                }
                if let Some((_, lines)) = &mut current_block {
                    lines.push(json!({"line": line_no, "text": text, "is_match": true}));
                }
            } else if matches_out.len() < limit {
                matches_out.push(json!({
                    "path": path,
                    "line": line_no,
                    "column": column,
                    "matched": matched,
                    "text": text,
                }));
            } else {
                truncated = true;
            }
        } else if context > 0 {
            // context line
            let attach = match &current_block {
                Some((p, lines)) => {
                    *p == path
                        && lines
                            .last()
                            .and_then(|l| l.get("line").and_then(|n| n.as_u64()))
                            .map(|last| line_no == last + 1)
                            .unwrap_or(false)
                }
                None => false,
            };
            if attach {
                if let Some((_, lines)) = &mut current_block {
                    lines.push(json!({"line": line_no, "text": text, "is_match": false}));
                }
            }
        }
    }
    if let Some((p, lines)) = current_block.take() {
        blocks.push(json!({"path": p, "lines": lines}));
    }

    let total_matches = match_events;
    if context > 0 && blocks.len() > limit {
        truncated = true;
        blocks.truncate(limit);
    }

    let mut b = if context > 0 {
        let mut b = if total_matches == 0 {
            Builder::no_results(operation, format!("no matches for {query:?}"))
        } else {
            Builder::ok(
                operation,
                format!(
                    "{total_matches} match(es) in {} context block(s)",
                    blocks.len()
                ),
            )
        };
        b.data = json!({
            "query": query,
            "mode": if opts.fixed { "fixed" } else { "regex" },
            "backend": "rg",
            "context_lines": context,
            "match_count": total_matches,
            "block_count": blocks.len(),
            "blocks": blocks,
            "root": opts.path.clone().unwrap_or_else(|| PathBuf::from(".")).to_string_lossy(),
        });
        for block in blocks.iter().take(3) {
            b = b.line(format!(
                "— {}",
                block.get("path").and_then(|p| p.as_str()).unwrap_or("?")
            ));
            for l in block
                .get("lines")
                .and_then(|l| l.as_array())
                .into_iter()
                .flatten()
            {
                let mark = if l.get("is_match").and_then(|v| v.as_bool()).unwrap_or(false) {
                    ">"
                } else {
                    " "
                };
                b = b.line(format!(
                    "  {mark} {:>5}: {}",
                    l.get("line").and_then(|n| n.as_u64()).unwrap_or(0),
                    l.get("text").and_then(|t| t.as_str()).unwrap_or("")
                ));
            }
        }
        b
    } else {
        let mut b = if total_matches == 0 {
            Builder::no_results(operation, format!("no matches for {query:?}"))
        } else {
            Builder::ok(
                operation,
                format!("{total_matches} match(es) for {query:?}"),
            )
        };
        b.data = json!({
            "query": query,
            "mode": if opts.fixed { "fixed" } else { "regex" },
            "backend": "rg",
            "match_count": matches_out.len(),
            "total_match_events": total_matches,
            "matches": matches_out,
            "root": opts.path.clone().unwrap_or_else(|| PathBuf::from(".")).to_string_lossy(),
        });
        for m in b
            .data
            .get("matches")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .take(20)
        {
            b = b.line(format!(
                "{}:{}:{}: {}",
                m.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
                m.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
                m.get("column")
                    .and_then(|v| v.as_u64())
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
                m.get("text").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
        b
    };

    b = b
        .evidence(Evidence::command(
            rg,
            &args,
            outcome.exit_code,
            outcome.duration_ms,
            outcome.timed_out,
        ))
        .truncated(truncated);
    if truncated {
        b = b.warning(format!(
            "results are bounded (limit {limit}); more matches exist — narrow the query or raise --limit"
        ));
    }
    // Silent-regex-trap recovery: a metacharacter-bearing query that matched
    // nothing in regex mode may have been intended literally. Surface the
    // bounded hint; never rerun or reinterpret the query automatically.
    if total_matches == 0
        && !opts.fixed
        && regex_needs_rg(query)
        && matches!(operation, "search.text" | "search.context")
    {
        let sub = if context > 0 { "context" } else { "text" };
        b = b.next(
            format!("supertools search {sub} {query:?} --fixed"),
            "zero matches in regex mode and the query contains regex metacharacters; if you meant it literally, retry with --fixed",
        );
    }
    Ok(b)
}

/// Normalise backend-reported paths for stable agent-facing output:
/// forward slashes, no "./" search-root prefix.
fn norm_search_path(p: &str) -> String {
    let unified = p.replace('\\', "/");
    match unified.strip_prefix("./") {
        Some(rest) => rest.to_string(),
        None => unified,
    }
}

/// Strip Windows verbatim (`\\?\`) prefixes and trailing slashes from a
/// unified-slash path string.
fn strip_verbatim(unified: &str) -> String {
    let t = unified.strip_prefix("//?/").unwrap_or(unified);
    t.trim_end_matches('/').to_string()
}

/// Current working directory as a unified-slash string — the relativisation
/// root that rg/fd output already follows.
fn cwd_prefix_unified() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    Some(strip_verbatim(&cwd.to_string_lossy().replace('\\', "/")))
}

/// Does `unified` start with `root` + "/" ? ASCII case-insensitive on Windows
/// (byte-level, so the prefix length stays valid for slicing the original).
fn path_prefix_matches(unified: &str, root: &str) -> bool {
    let (u, r) = (unified.as_bytes(), root.as_bytes());
    if u.len() <= r.len() || u[r.len()] != b'/' {
        return false;
    }
    #[cfg(windows)]
    {
        u[..r.len()]
            .iter()
            .zip(r)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
    #[cfg(not(windows))]
    {
        u.starts_with(root)
    }
}

/// Normalise an absolute backend path to the same repository(cwd)-relative
/// convention used by rg/fd results, so search.text, search.context and
/// search.structural hits can be correlated without manual reconciliation.
/// Paths outside the cwd stay absolute — truthful, never invented relatives.
fn relativise_backend_path(p: &str, cwd_prefix: Option<&str>) -> String {
    let unified = strip_verbatim(&p.replace('\\', "/"));
    if let Some(root) = cwd_prefix {
        if path_prefix_matches(&unified, root) {
            return unified[root.len() + 1..].to_string();
        }
    }
    norm_search_path(&unified)
}

fn parses_as_json_array(s: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(s.trim())
        .map(|v| v.is_array())
        .unwrap_or(false)
}

fn json_text_field(obj: &serde_json::Value, key: &str) -> Option<String> {
    let v = obj.get(key)?;
    v.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            v.get("bytes")
                .and_then(|b| b.as_str())
                .map(|_| "<non-utf8>".to_string())
        })
}

fn json_text_field_of(obj: &serde_json::Value, key: &str) -> Option<String> {
    json_text_field(obj, key).or_else(|| obj.get(key).and_then(|t| t.as_str()).map(str::to_string))
}

fn byte_offset_to_column(data: &serde_json::Value, byte_off: u64) -> u64 {
    // Column in characters, 1-based, computed from the line text.
    if let Some(text) = data
        .get("lines")
        .and_then(|l| l.get("text"))
        .and_then(|t| t.as_str())
    {
        let off = (byte_off as usize).min(text.len());
        if text.is_char_boundary(off) {
            return text[..off].chars().count() as u64 + 1;
        }
    }
    0
}

// ------------------------------------------------- internal fallback backend

fn git_ls_files(cwd: &Path, git: &str, operation: &str) -> Result<Vec<PathBuf>, Failure> {
    let req = Request::new(
        git.to_string(),
        vec![
            "ls-files".into(),
            "-z".into(),
            "--cached".into(),
            "--others".into(),
            "--exclude-standard".into(),
        ],
    )
    .cwd(cwd)
    .timeout(SEARCH_TIMEOUT);
    let outcome = process::run(&req)
        .map_err(|e| Failure::failed(operation, format!("git ls-files could not run: {e}")))?;
    if !outcome.success() {
        return Err(Failure::failed(
            operation,
            format!("git ls-files failed (exit {:?})", outcome.exit_code),
        ));
    }
    Ok(outcome
        .stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

struct FallbackHit {
    path: String,
    line: u64,
    column: u64,
    text: String,
}

/// Bounded internal fixed-substring scan over git-listed files. Used only
/// when rg is unavailable. Never follows symlinks; respects ignore rules via
/// git's own listing.
fn internal_scan(
    root: &Path,
    needle: &str,
    context: usize,
    limit: usize,
    operation: &str,
) -> Result<(Vec<FallbackHit>, Vec<serde_json::Value>, usize, bool), Failure> {
    let git = quick_require(ToolId::Git, operation)?;
    let files = git_ls_files(root, &git, operation)?;
    let mut hits: Vec<FallbackHit> = Vec::new();
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut truncated = files.len() > FALLBACK_MAX_FILES;
    let mut scanned = 0usize;

    for rel in files.iter().take(FALLBACK_MAX_FILES) {
        let full = root.join(rel);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > FALLBACK_MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&full) else {
            continue;
        };
        if looks_binary(&bytes) {
            continue;
        }
        scanned += 1;
        let display_path = if root == Path::new(".") {
            rel.to_string_lossy().into_owned()
        } else {
            format!(
                "{}/{}",
                root.to_string_lossy().replace('\\', "/"),
                rel.to_string_lossy()
            )
        };
        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = content.lines().collect();
        let mut marked: Vec<bool> = vec![false; lines.len()];
        for (i, line) in lines.iter().enumerate() {
            if let Some(byte_idx) = line.find(needle) {
                marked[i] = true;
                if hits.len() < limit {
                    hits.push(FallbackHit {
                        path: display_path.clone(),
                        line: i as u64 + 1,
                        column: line[..byte_idx].chars().count() as u64 + 1,
                        text: head(line.trim_end(), LINE_CAP_BYTES).0,
                    });
                } else {
                    truncated = true;
                }
            }
        }
        if context > 0 {
            let mut i = 0;
            while i < lines.len() {
                if marked[i] {
                    let start = i.saturating_sub(context);
                    let mut end = (i + context).min(lines.len() - 1);
                    // merge with following matches
                    let mut j = i + 1;
                    while j < lines.len() {
                        if marked[j] {
                            end = (j + context).min(lines.len() - 1);
                            j += 1;
                        } else if j <= end {
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    if blocks.len() < limit {
                        let win: Vec<serde_json::Value> = (start..=end)
                            .map(|n| {
                                json!({
                                    "line": n as u64 + 1,
                                    "text": head(lines[n].trim_end(), LINE_CAP_BYTES).0,
                                    "is_match": marked[n],
                                })
                            })
                            .collect();
                        blocks.push(json!({"path": display_path.clone(), "lines": win}));
                    }
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
        }
    }
    Ok((hits, blocks, scanned, truncated))
}

#[allow(clippy::too_many_arguments)]
fn fallback_builder(
    operation: &str,
    query: &str,
    opts: &CommonOpts,
    context: usize,
    scanned: usize,
    truncated: bool,
    hits: Vec<FallbackHit>,
    blocks: Vec<serde_json::Value>,
    warnings_extra: Vec<String>,
) -> Builder {
    let total = if context > 0 {
        blocks
            .iter()
            .map(|b| {
                b.get("lines")
                    .and_then(|l| l.as_array())
                    .map(|ls| {
                        ls.iter()
                            .filter(|l| {
                                l.get("is_match").and_then(|v| v.as_bool()).unwrap_or(false)
                            })
                            .count()
                    })
                    .unwrap_or(0)
            })
            .sum()
    } else {
        hits.len()
    };
    let mut b = if total == 0 {
        Builder::no_results(operation, format!("no matches for {query:?}"))
    } else {
        Builder::ok(operation, format!("{total} match(es) for {query:?}"))
    };

    if context > 0 {
        b.data = json!({
            "query": query,
            "mode": "fixed",
            "backend": "internal-fallback",
            "context_lines": context,
            "match_count": total,
            "block_count": blocks.len(),
            "blocks": blocks,
            "files_scanned": scanned,
            "root": opts.path.clone().unwrap_or_else(|| PathBuf::from(".")).to_string_lossy(),
        });
    } else {
        let matches: Vec<serde_json::Value> = hits
            .iter()
            .map(|h| json!({"path": h.path, "line": h.line, "column": h.column, "matched": query, "text": h.text}))
            .collect();
        b.data = json!({
            "query": query,
            "mode": "fixed",
            "backend": "internal-fallback",
            "match_count": matches.len(),
            "matches": matches,
            "files_scanned": scanned,
            "root": opts.path.clone().unwrap_or_else(|| PathBuf::from(".")).to_string_lossy(),
        });
        for h in hits.iter().take(20) {
            b = b.line(format!("{}:{}:{}: {}", h.path, h.line, h.column, h.text));
        }
    }

    b = b.warning(
        "rg unavailable: bounded internal fixed-substring fallback (slower, no regex, git-listed files only)",
    )
    .truncated(truncated);
    for w in warnings_extra {
        b = b.warning(w);
    }
    if truncated {
        b = b.warning("results or file scan were bounded; more matches may exist");
    }
    b = b.next(
        "supertools tools install-missing --yes",
        "install rg for full text search capability",
    );
    b
}

// ------------------------------------------------------------- operations

pub fn text(opts: CommonOpts) -> CmdResult {
    let operation = "search.text";
    if !opts.fixed && opts.globs.is_empty() && regex_needs_rg(&opts.query) {
        // default mode is regex; regex without rg is unavailable
        if process::resolve_program("rg").is_err() {
            return Err(Failure::unavailable(
                operation,
                "regex search requires rg; the internal fallback supports fixed strings only",
            )
            .hint("retry with --fixed for substring search")
            .hint("supertools tools install-missing --yes  # install ripgrep"));
        }
    }
    if let Ok(rg) = quick_require(ToolId::Rg, operation) {
        return rg_json_search(&rg, &opts.query, &opts, 0, opts.limit, operation);
    }
    if !opts.fixed {
        return Err(Failure::unavailable(
            operation,
            "regex search requires rg; the internal fallback supports fixed strings only",
        )
        .hint("retry with --fixed for substring search"));
    }
    let root = opts.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let (hits, _, scanned, truncated) =
        internal_scan(&root, &opts.query, 0, opts.limit, operation)?;
    Ok(fallback_builder(
        operation,
        &opts.query,
        &opts,
        0,
        scanned,
        truncated,
        hits,
        Vec::new(),
        vec![],
    ))
}

fn regex_needs_rg(query: &str) -> bool {
    query.chars().any(|c| {
        matches!(
            c,
            '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\'
        )
    })
}

pub fn context(opts: CommonOpts) -> CmdResult {
    let operation = "search.context";
    let ctx = opts.context.clamp(1, MAX_CONTEXT_LINES);
    if let Ok(rg) = quick_require(ToolId::Rg, operation) {
        return rg_json_search(&rg, &opts.query, &opts, ctx, opts.limit.max(1), operation);
    }
    if !opts.fixed {
        return Err(Failure::unavailable(
            operation,
            "regex search requires rg; the internal fallback supports fixed strings only",
        )
        .hint("retry with --fixed"));
    }
    let root = opts.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let block_limit = opts.limit.max(1);
    let (hits, blocks, scanned, truncated) =
        internal_scan(&root, &opts.query, ctx, block_limit, operation)?;
    let _ = hits;
    Ok(fallback_builder(
        operation,
        &opts.query,
        &opts,
        ctx,
        scanned,
        truncated,
        Vec::new(),
        blocks,
        vec![],
    ))
}

/// Escape a literal for inclusion in a regex.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn symbol(opts: CommonOpts) -> CmdResult {
    let operation = "search.symbol";
    let rg = quick_require(ToolId::Rg, operation)?;
    let original = opts.query.clone();
    let esc = regex_escape(&original);
    let pattern = format!(
        "\\b(fn|def|func|function|class|struct|enum|trait|interface|type|record|const|static|let|var|sub|module|namespace|impl)\\s+{esc}\\b|#\\s*define\\s+{esc}\\b|\\b{esc}\\s*[:=]\\s*(async\\s*)?(function|fn|\\()"
    );
    let sub = CommonOpts {
        fixed: false,
        query: pattern.clone(),
        ..opts
    };
    let mut b = rg_json_search(&rg, &pattern, &sub, 0, sub.limit, operation)?;
    if let Some(obj) = b.data.as_object_mut() {
        obj.insert("heuristic".to_string(), json!(true));
        obj.insert("symbol".to_string(), json!(original));
        obj.insert("pattern".to_string(), json!(pattern));
    }
    b = b.warning(
        "search.symbol is an explicitly labeled HEURISTIC (definition-pattern grep), not semantic symbol resolution",
    );
    Ok(b)
}

pub fn structural(opts: CommonOpts) -> CmdResult {
    let operation = "search.structural";
    let sg = match quick_require(ToolId::AstGrep, operation) {
        Ok(sg) => sg,
        Err(mut f) => {
            f.hints.push(
                "search.text can help only where plain text is semantically sufficient — it is NOT equivalent to structural search".into(),
            );
            return Err(f);
        }
    };
    let target = opts.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let mut args = vec![
        "run".to_string(),
        "--pattern".to_string(),
        opts.query.clone(),
        "--json=compact".to_string(),
        target.to_string_lossy().into_owned(),
    ];
    let req = Request::new(sg.clone(), args.clone())
        .timeout(SEARCH_TIMEOUT)
        .max_stdout(8 * 1024 * 1024);
    let mut outcome = process::run(&req)
        .map_err(|e| Failure::failed(operation, format!("ast-grep could not run: {e}")))?;
    if !parses_as_json_array(&outcome.stdout)
        && (outcome.stderr.contains("json") || outcome.stderr.contains("unexpected"))
    {
        // Older/newer flag shape: retry with plain --json.
        args = vec![
            "run".to_string(),
            "--pattern".to_string(),
            opts.query.clone(),
            "--json".to_string(),
            target.to_string_lossy().into_owned(),
        ];
        let req = Request::new(sg.clone(), args.clone())
            .timeout(SEARCH_TIMEOUT)
            .max_stdout(8 * 1024 * 1024);
        outcome = process::run(&req)
            .map_err(|e| Failure::failed(operation, format!("ast-grep could not run: {e}")))?;
    }
    if outcome.timed_out {
        return Err(Failure::new(
            operation,
            Status::Timeout,
            "ast-grep exceeded its timeout and was killed",
        ));
    }
    // ast-grep's exit code does not reliably distinguish "no matches" from
    // success, so the parsed JSON stream is the authority here.
    let parsed: serde_json::Value = serde_json::from_str(outcome.stdout.trim()).map_err(|_| {
        Failure::failed(
            operation,
            format!(
                "ast-grep failed (exit {:?}): {}",
                outcome.exit_code,
                head(outcome.stderr.trim(), 400).0
            ),
        )
    })?;
    let arr = parsed.as_array().cloned().unwrap_or_default();
    let total = arr.len();
    let truncated = total > opts.limit || outcome.stdout_truncated;
    // ast-grep reports absolute paths; relativise to the cwd so results
    // correlate with search.text/search.context paths.
    let cwd = cwd_prefix_unified();
    let kept: Vec<serde_json::Value> = arr
        .into_iter()
        .take(opts.limit)
        .map(|m| {
            json!({
                "path": m
                    .get("file")
                    .and_then(|f| f.as_str())
                    .map(|f| relativise_backend_path(f, cwd.as_deref()))
                    .unwrap_or_default(),
                "start": m.get("range").and_then(|r| r.get("start")).cloned().unwrap_or(json!(null)),
                "end": m.get("range").and_then(|r| r.get("end")).cloned().unwrap_or(json!(null)),
                "text": head(m.get("text").and_then(|t| t.as_str()).unwrap_or(""), LINE_CAP_BYTES).0,
            })
        })
        .collect();

    let mut b = if total == 0 {
        Builder::no_results(
            operation,
            format!("no structural matches for pattern {:?}", opts.query),
        )
    } else {
        Builder::ok(
            operation,
            format!("{total} structural match(es) for {:?}", opts.query),
        )
    };
    b.data = json!({
        "pattern": opts.query,
        "backend": "ast-grep",
        "match_count": kept.len(),
        "matches": kept,
        "root": target.to_string_lossy(),
    });
    for m in kept.iter().take(20) {
        b = b.line(format!(
            "{}:{}: {}",
            m.get("path").and_then(|p| p.as_str()).unwrap_or("?"),
            m.get("start")
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_u64())
                .unwrap_or(0),
            m.get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .replace('\n', " ⏎ ")
        ));
    }
    b = b
        .evidence(Evidence::command(
            &sg,
            &args,
            outcome.exit_code,
            outcome.duration_ms,
            outcome.timed_out,
        ))
        .truncated(truncated)
        .warning("ast-grep range line/column values are 0-based as reported by ast-grep");
    if truncated {
        b = b.warning(format!("results bounded at limit {}", opts.limit));
    }
    Ok(b)
}

pub fn files(opts: CommonOpts) -> CmdResult {
    let operation = "search.files";
    let limit = opts.limit;
    let target = opts.path.clone().unwrap_or_else(|| PathBuf::from("."));

    // Preferred backend: fd.
    if let Ok(fd) = quick_require(ToolId::Fd, operation) {
        let mut args = vec![
            "--color".to_string(),
            "never".to_string(),
            "--type".to_string(),
            "f".to_string(),
        ];
        if opts.fixed {
            args.push("--fixed-strings".to_string());
        }
        args.push("--".to_string());
        args.push(opts.query.clone());
        args.push(target.to_string_lossy().into_owned());
        let req = Request::new(fd.clone(), args.clone())
            .timeout(SEARCH_TIMEOUT)
            .max_stdout(2 * 1024 * 1024);
        let outcome = process::run(&req)
            .map_err(|e| Failure::failed(operation, format!("fd could not run: {e}")))?;
        if outcome.timed_out {
            return Err(Failure::new(
                operation,
                Status::Timeout,
                "fd exceeded its timeout and was killed",
            ));
        }
        // fd exit codes: 0 results, 1 no results, >1 error.
        if let Some(code) = outcome.exit_code {
            if code > 1 {
                return Err(Failure::failed(
                    operation,
                    format!(
                        "fd failed (exit {code}): {}",
                        head(outcome.stderr.trim(), 400).0
                    ),
                ));
            }
        }
        let all: Vec<String> = outcome
            .stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(norm_search_path)
            .collect();
        let truncated = all.len() > limit || outcome.stdout_truncated;
        let paths: Vec<String> = all.into_iter().take(limit).collect();
        let mut b = if paths.is_empty() {
            Builder::no_results(operation, format!("no files matching {:?}", opts.query))
        } else {
            Builder::ok(
                operation,
                format!("{} file(s) matching {:?}", paths.len(), opts.query),
            )
        };
        b.data = json!({
            "query": opts.query,
            "mode": if opts.fixed { "fixed" } else { "regex" },
            "backend": "fd",
            "match_count": paths.len(),
            "paths": paths,
            "root": target.to_string_lossy(),
        });
        for p in paths.iter().take(30) {
            b = b.line(p.clone());
        }
        b = b
            .evidence(Evidence::command(
                &fd,
                &args,
                outcome.exit_code,
                outcome.duration_ms,
                outcome.timed_out,
            ))
            .truncated(truncated);
        if truncated {
            b = b.warning(format!(
                "results bounded at limit {limit}; raise --limit to see more"
            ));
        }
        return Ok(b);
    }

    // Fallback: bounded git ls-files + substring filter.
    let git = quick_require(ToolId::Git, operation).map_err(|mut f| {
        f.hints.push("install fd (supertools tools install-missing --yes) for file discovery outside git repositories".into());
        f.message = format!("{}; neither fd nor git is available for bounded file discovery", f.message);
        f
    })?;
    let req = Request::new(
        git.clone(),
        vec![
            "ls-files".into(),
            "-z".into(),
            "--cached".into(),
            "--others".into(),
            "--exclude-standard".into(),
        ],
    )
    .cwd(target.clone())
    .timeout(SEARCH_TIMEOUT);
    let outcome = process::run(&req)
        .map_err(|e| Failure::failed(operation, format!("git ls-files could not run: {e}")))?;
    if !outcome.success() {
        return Err(
            Failure::failed(operation, "git ls-files failed; is this a git repository?")
                .hint("supertools repo state")
                .hint("install fd for file discovery without git"),
        );
    }
    let needle = opts.query.to_lowercase();
    let all: Vec<String> = outcome
        .stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .filter(|s| s.to_lowercase().contains(&needle))
        .map(str::to_string)
        .collect();
    let truncated = all.len() > limit || outcome.stdout_truncated;
    let paths: Vec<String> = all.into_iter().take(limit).collect();
    let mut b = if paths.is_empty() {
        Builder::no_results(operation, format!("no files matching {:?}", opts.query))
    } else {
        Builder::ok(
            operation,
            format!("{} file(s) matching {:?}", paths.len(), opts.query),
        )
    };
    b.data = json!({
        "query": opts.query,
        "mode": "substring (case-insensitive)",
        "backend": "git-ls-files-fallback",
        "match_count": paths.len(),
        "paths": paths,
        "root": target.to_string_lossy(),
    });
    for p in b
        .data
        .get("paths")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(30)
    {
        if let Some(s) = p.as_str() {
            b = b.line(s.to_string());
        }
    }
    b = b
        .evidence(Evidence::command(&git, &req.args, outcome.exit_code, outcome.duration_ms, outcome.timed_out))
        .warning("fd unavailable: bounded git ls-files fallback (substring match, git-listed files only)")
        .truncated(truncated)
        .next("supertools tools install-missing --yes", "install fd for faster, richer file discovery");
    if truncated {
        b = b.warning(format!("results bounded at limit {limit}"));
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_dot_slash(s: &str) {
        assert!(!s.contains("./"), "unexpected ./ in {s:?}");
        assert!(!s.contains(".\\"), "unexpected .\\ in {s:?}");
    }

    #[test]
    fn windows_style_backend_path_becomes_relative() {
        let got = relativise_backend_path("D:\\proj\\src\\main.rs", Some("D:/proj"));
        assert_eq!(got, "src/main.rs");
        no_dot_slash(&got);
    }

    #[test]
    fn unix_style_backend_path_becomes_relative() {
        let got = relativise_backend_path("/home/u/proj/src/main.rs", Some("/home/u/proj"));
        assert_eq!(got, "src/main.rs");
        no_dot_slash(&got);
    }

    #[test]
    fn already_relative_dot_prefixed_path_is_cleaned() {
        let got = relativise_backend_path("./src/main.rs", Some("/home/u/proj"));
        assert_eq!(got, "src/main.rs");
        no_dot_slash(&got);
        let got = relativise_backend_path(".\\src\\main.rs", Some("D:/proj"));
        assert_eq!(got, "src/main.rs");
        no_dot_slash(&got);
    }

    #[test]
    fn path_outside_root_stays_absolute_and_truthful() {
        let got = relativise_backend_path("/elsewhere/x.rs", Some("/home/u/proj"));
        assert_eq!(got, "/elsewhere/x.rs");
        let got = relativise_backend_path("D:\\other\\x.rs", Some("D:/proj"));
        assert_eq!(got, "D:/other/x.rs");
        no_dot_slash(&got);
    }

    #[test]
    fn verbatim_windows_prefix_is_stripped() {
        let got = relativise_backend_path("\\\\?\\D:\\proj\\src\\x.rs", Some("D:/proj"));
        assert_eq!(got, "src/x.rs");
    }

    #[test]
    fn root_match_requires_separator_not_partial_name() {
        // "D:/project" must not be treated as inside "D:/proj".
        let got = relativise_backend_path("D:/project/src/x.rs", Some("D:/proj"));
        assert_eq!(got, "D:/project/src/x.rs");
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefix_match_is_case_insensitive() {
        let got = relativise_backend_path("d:/PROJ/src/x.rs", Some("D:/proj"));
        assert_eq!(got, "src/x.rs");
    }

    #[test]
    fn zero_match_regex_query_gains_fixed_hint() {
        // The hint predicate: regex mode + metacharacters. Fixed mode and
        // plain literal queries must not gain the hint.
        assert!(regex_needs_rg("map[string]"));
        assert!(regex_needs_rg("foo.*bar"));
        assert!(!regex_needs_rg("plain_identifier"));
    }
}
