//! Search domain tests: bounded results, ignore semantics, unicode,
//! metacharacter safety, fallbacks and truncation.

mod common;

use common::*;

/// A PATH directory where `git` resolves but `rg` must not, for fallback
/// tests. Unix distros ship git+rg side by side, so an isolated dir with a
/// git symlink is built there; on Windows git's own directory suffices
/// (rg ships separately via the cargo bin).
/// Returns (guard dir to keep alive, path to put on PATH).
fn isolated_git_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let git = which::which("git").expect("git for fixtures");
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&git, dir.path().join("git")).unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }
    #[cfg(windows)]
    {
        let dir = tempfile::tempdir().unwrap();
        let path = git.parent().unwrap().to_path_buf();
        (dir, path)
    }
}

fn repo_with_content() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        "pub fn calculate_total(a: u32, b: u32) -> u32 {\n    a + b\n}\n\npub struct Needle;\n",
    );
    write(
        dir.path(),
        "notes.txt",
        "alpha line\nPermission granted here\nbeta line\ngamma line\n",
    );
    write(dir.path(), ".gitignore", "ignored.txt\ntarget/\n");
    write(
        dir.path(),
        "ignored.txt",
        "Permission should NOT be found here\n",
    );
    dir
}

#[test]
fn text_finds_matches_with_location() {
    let dir = repo_with_content();
    let out = run_json(dir.path(), &["search", "text", "Permission", "--fixed"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert!(d["match_count"].as_u64().unwrap() >= 1);
    let m = &d["matches"][0];
    assert_eq!(m["path"].as_str().unwrap().replace('\\', "/"), "notes.txt");
    assert_eq!(m["line"], 2);
    assert!(m["column"].as_u64().unwrap() >= 1);
    assert!(m["text"].as_str().unwrap().contains("Permission"));
}

#[test]
fn text_ignores_gitignored_files() {
    let dir = repo_with_content();
    let out = run_json(dir.path(), &["search", "text", "Permission", "--fixed"]);
    let d = out.data();
    let paths: Vec<String> = d["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["path"].as_str().unwrap().replace('\\', "/"))
        .collect();
    assert!(
        !paths.iter().any(|p| p.contains("ignored.txt")),
        "gitignored file was searched: {paths:?}"
    );
}

#[test]
fn text_no_matches_is_exit_4() {
    let dir = repo_with_content();
    let out = run_json(
        dir.path(),
        &["search", "text", "absent-needle-xyz", "--fixed"],
    );
    assert_eq!(out.code, 4);
    assert_eq!(out.status_str(), "no_results");
    let v = out.json();
    assert_eq!(v["ok"], true);
}

#[test]
fn text_limit_bounds_and_marks_truncation() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let content: String = (0..120).map(|i| format!("needle line {i}\n")).collect();
    write(dir.path(), "many.txt", &content);

    let out = run_json(
        dir.path(),
        &["search", "text", "needle", "--fixed", "--limit", "10"],
    );
    assert_eq!(out.code, 0);
    let v = out.json();
    assert_eq!(v["data"]["matches"].as_array().unwrap().len(), 10);
    assert_eq!(v["truncated"], true);
    assert!(v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| { w.as_str().unwrap().contains("bounded") }));
}

#[test]
fn text_regex_mode_works() {
    let dir = repo_with_content();
    let out = run_json(dir.path(), &["search", "text", "calc.*_total"]);
    assert_eq!(out.code, 0);
    assert!(out.data()["match_count"].as_u64().unwrap() >= 1);
    assert_eq!(out.data()["mode"], "regex");
}

#[test]
fn text_glob_narrows_results() {
    let dir = repo_with_content();
    let out = run_json(
        dir.path(),
        &["search", "text", "line", "--fixed", "--glob", "*.rs"],
    );
    assert_eq!(
        out.code, 4,
        "no .rs file contains bare 'line' word matches?"
    );
    let out = run_json(dir.path(), &["search", "text", "total", "--glob", "*.rs"]);
    assert_eq!(out.code, 0);
    let paths: Vec<String> = out.data()["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["path"].as_str().unwrap().to_string())
        .collect();
    assert!(paths.iter().all(|p| p.ends_with(".rs")));
}

#[test]
fn text_path_scoping() {
    let dir = repo_with_content();
    let out = run_json(
        dir.path(),
        &["search", "text", "calculate", "--path", "src"],
    );
    assert_eq!(out.code, 0);
    assert!(out.data()["match_count"].as_u64().unwrap() >= 1);
}

#[test]
fn text_unicode_content_and_filenames() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(dir.path(), "文档 Ünïcödé.txt", "日本語の Needle 行\n");
    let out = run_json(dir.path(), &["search", "text", "日本語", "--fixed"]);
    assert_eq!(out.code, 0);
    let m = &out.data()["matches"][0];
    assert!(m["path"].as_str().unwrap().contains("Ünïcödé"));
    assert!(m["text"].as_str().unwrap().contains("日本語"));
    // column is character-based, not byte-based
    assert_eq!(m["column"], 1);
}

#[test]
fn text_paths_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work dir with spaces");
    init_repo_with_commit(&work);
    write(&work, "my file.txt", "the needle is here\n");
    let out = run_json(&work, &["search", "text", "needle", "--fixed"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.data()["matches"][0]["path"], "my file.txt");
}

#[test]
fn context_returns_bounded_windows() {
    let dir = repo_with_content();
    let out = run_json(
        dir.path(),
        &[
            "search",
            "context",
            "Permission",
            "--fixed",
            "--context",
            "1",
        ],
    );
    assert_eq!(out.code, 0);
    let d = out.data();
    let blocks = d["blocks"].as_array().unwrap();
    assert!(!blocks.is_empty());
    let lines = blocks[0]["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l["is_match"] == true));
    assert!(lines.iter().any(|l| l["is_match"] == false));
    assert!(lines.len() <= 3);
}

#[test]
fn symbol_is_labeled_heuristic() {
    let dir = repo_with_content();
    if !tool_available("rg") {
        // heuristic symbol search requires rg; absence is a clean skip
        let out = run_json(dir.path(), &["search", "symbol", "calculate_total"]);
        assert_eq!(out.code, 3);
        return;
    }
    let out = run_json(dir.path(), &["search", "symbol", "calculate_total"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["heuristic"], true);
    assert_eq!(d["symbol"], "calculate_total");
    assert!(d["pattern"].as_str().unwrap().contains("calculate_total"));
    assert!(d["match_count"].as_u64().unwrap() >= 1);
}

#[test]
fn files_finds_by_name() {
    let dir = repo_with_content();
    let out = run_json(dir.path(), &["search", "files", "lib.rs"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    let paths: Vec<String> = d["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().replace('\\', "/"))
        .collect();
    assert!(paths.iter().any(|p| p.ends_with("src/lib.rs")), "{paths:?}");
    // ignored files are not listed
    let out = run_json(dir.path(), &["search", "files", "ignored.txt"]);
    assert_eq!(out.code, 4, "gitignored file must not be discovered");
}

#[test]
fn files_no_match_is_exit_4() {
    let dir = repo_with_content();
    let out = run_json(dir.path(), &["search", "files", "zzz-not-a-file"]);
    assert_eq!(out.code, 4);
}

#[test]
fn structural_uses_ast_grep_or_reports_unavailable() {
    let dir = repo_with_content();
    if tool_available("ast-grep") || tool_available("sg") {
        let out = run_json(
            dir.path(),
            &["search", "structural", "pub fn $NAME($$$) -> u32 { $$$ }"],
        );
        assert_eq!(out.code, 0, "stderr: {}", out.stderr);
        let d = out.data();
        assert_eq!(d["backend"], "ast-grep");
        assert!(d["match_count"].as_u64().unwrap() >= 1);
        let m = &d["matches"][0];
        assert!(m["text"].as_str().unwrap().contains("calculate_total"));
    } else {
        let out = run_json(dir.path(), &["search", "structural", "fn $NAME() { $$$ }"]);
        assert_eq!(out.code, 3);
        assert_eq!(out.status_str(), "capability_unavailable");
        // must suggest textual fallback without pretending equivalence
        let v = out.json();
        let hints = v["next_actions"].as_array().unwrap();
        assert!(
            hints
                .iter()
                .any(|h| h["reason"].as_str().unwrap().contains("search.text"))
                || out.stderr.contains("search.text"),
            "should point at search.text fallback"
        );
    }
}

#[test]
fn missing_rg_falls_back_for_fixed_and_refuses_regex() {
    let dir = repo_with_content();
    // Simulate rg absence while keeping git reachable. Unix distros often
    // ship git and rg side by side (/usr/bin), so PATH cannot simply be
    // "git's directory" there — isolate git behind a symlink instead.
    let (_iso, iso_dir) = isolated_git_dir();

    // fixed-substring fallback works with git only
    let out = run_json_env(
        dir.path(),
        &["search", "text", "Permission", "--fixed"],
        &[("PATH", Some(iso_dir.to_str().unwrap()))],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let d = out.data();
    assert_eq!(d["backend"], "internal-fallback");
    assert!(d["match_count"].as_u64().unwrap() >= 1);
    assert!(out.json()["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("rg unavailable")));

    // regex without rg is unavailable with a clear pointer
    let out = run_json_env(
        dir.path(),
        &["search", "text", "Perm.*ion"],
        &[("PATH", Some(iso_dir.to_str().unwrap()))],
    );
    assert_eq!(out.code, 3);
    assert_eq!(out.status_str(), "capability_unavailable");
}

#[test]
fn search_without_rg_or_git_reports_unavailable() {
    let dir = repo_with_content();
    let empty = tempfile::tempdir().unwrap();
    let out = run_json_env(
        dir.path(),
        &["search", "text", "Permission", "--fixed"],
        &[("PATH", Some(empty.path().to_str().unwrap()))],
    );
    assert_eq!(out.code, 3);
    assert_eq!(out.status_str(), "capability_unavailable");
}

#[test]
fn context_fallback_without_rg() {
    let dir = repo_with_content();
    let git_path = which::which("git").expect("git for fixtures");
    let git_dir = git_path.parent().unwrap().to_path_buf();
    let out = run_json_env(
        dir.path(),
        &[
            "search",
            "context",
            "Permission",
            "--fixed",
            "--context",
            "1",
        ],
        &[("PATH", Some(git_dir.to_str().unwrap()))],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.data()["backend"], "internal-fallback");
    assert!(out.data()["blocks"]
        .as_array()
        .is_some_and(|b| !b.is_empty()));
}
