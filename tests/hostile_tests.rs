//! Hostile/failure tests: query input is DATA, never shell. Marker files
//! must survive; no side-effect files may appear. Windows quoting/paths too.

mod common;

use common::*;

fn hostile_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(
        dir.path(),
        "marker.txt",
        "precious content - do not delete\n",
    );
    write(dir.path(), "code.rs", "let pair = \"a && b\";\n");
    dir
}

fn payloads() -> Vec<&'static str> {
    vec![
        "; rm -rf .",
        "&& del marker.txt",
        "| cat marker.txt",
        "$(touch pwned.txt)",
        "`touch pwned.txt`",
        "> pwned.txt",
        "a; echo pwned",
        "%COMSPEC% /c echo pwned",
        "$(echo pwned)",
        "'; DROP TABLE x; --",
    ]
}

fn assert_no_side_effects(dir: &std::path::Path) {
    assert_eq!(
        std::fs::read_to_string(dir.join("marker.txt")).unwrap(),
        "precious content - do not delete\n"
    );
    assert!(
        !dir.join("pwned.txt").exists(),
        "hostile input created a file!"
    );
    assert!(!dir.join("pwned").exists(), "hostile input created a file!");
}

#[test]
fn shell_metacharacters_are_treated_as_literal_queries() {
    let dir = hostile_repo();
    for q in payloads() {
        let out = run_json(dir.path(), &["search", "text", q, "--fixed"]);
        // literal query simply matches nothing (exit 4), never executes
        assert_eq!(out.code, 4, "query {q:?}: {}", out.stderr);
        assert_eq!(out.status_str(), "no_results");
        assert_no_side_effects(dir.path());

        // same for the other text-facing commands
        let out = run_json(dir.path(), &["search", "context", q, "--fixed"]);
        assert_eq!(out.code, 4, "context {q:?}");
        assert_no_side_effects(dir.path());

        let out = run_json(dir.path(), &["search", "files", q, "--fixed"]);
        assert_eq!(out.code, 4, "files {q:?}");
        assert_no_side_effects(dir.path());
    }
}

#[test]
fn fixed_metacharacters_still_match_literally() {
    let dir = hostile_repo();
    // the repo literally contains `a && b`
    let out = run_json(dir.path(), &["search", "text", "a && b", "--fixed"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(out.data()["match_count"].as_u64().unwrap() >= 1);
    assert_no_side_effects(dir.path());
}

#[test]
fn hostile_input_with_spaces_and_unicode_paths_is_safe() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("déjà vu dir");
    init_repo_with_commit(&work);
    write(&work, "marker.txt", "precious content - do not delete\n");
    for q in payloads() {
        let out = run_json(&work, &["search", "text", q, "--fixed"]);
        assert_eq!(out.code, 4, "query {q:?}");
        assert_no_side_effects(&work);
    }
}

#[test]
fn verify_task_names_are_never_executed_as_commands() {
    let dir = hostile_repo();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
    );
    for name in [
        "test; echo pwned",
        "test && echo pwned",
        "$(test)",
        "`test`",
        "| test",
        "..\\check",
        "check/../../x",
    ] {
        let out = run_json(dir.path(), &["verify", "task", name]);
        assert_eq!(out.code, 2, "task {name:?}: {}", out.stderr);
        assert_eq!(out.status_str(), "refused");
        assert_no_side_effects(dir.path());
    }
}

#[test]
fn hostile_structural_pattern_is_data() {
    let dir = hostile_repo();
    if !(tool_available("ast-grep") || tool_available("sg")) {
        return;
    }
    let out = run_json(
        dir.path(),
        &["search", "structural", "fn $X() { $$$ }; echo pwned"],
    );
    // either no match (4) or a parse error (1) — never execution
    assert!(
        out.code == 4 || out.code == 1,
        "unexpected code {}: {}",
        out.code,
        out.stderr
    );
    assert_no_side_effects(dir.path());
}

#[test]
fn hostile_path_arguments_do_not_escape() {
    // Controlled parent layout: the ".." target is OUR directory, bounded.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo_with_commit(&repo);
    write(&repo, "marker.txt", "precious content - do not delete\n");
    write(dir.path(), "sibling.txt", "sibling needle xyzzy\n");

    let out = run_json(
        &repo,
        &["search", "text", "xyzzy", "--fixed", "--path", ".."],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_no_side_effects(&repo);

    // a nonexistent path is a clear failure, not a crash or a wander
    let out = run_json(&repo, &["search", "text", "x", "--path", "no-such-dir"]);
    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    assert_no_side_effects(&repo);
}
