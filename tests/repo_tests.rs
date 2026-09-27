//! Repo domain tests: fixture repositories in every interesting state,
//! NUL-safe paths, upstream tracking, conflicts, and failure boundaries.

mod common;

use common::*;
use std::path::PathBuf;

#[test]
fn state_clean_repo_is_obviously_clean() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.code, 0);
    let v = assert_envelope(&out, "repo.state");
    let d = &v["data"];
    assert_eq!(d["dirty"], false);
    assert_eq!(d["staged"], 0);
    assert_eq!(d["unstaged"], 0);
    assert_eq!(d["untracked"], 0);
    assert_eq!(d["conflicts"], 0);
    assert_eq!(d["branch"], "main");
    assert_eq!(d["detached"], false);
    assert!(!d["head"].as_str().unwrap().is_empty());
    // root matches the fixture directory (canonicalised both sides for
    // Windows path-form differences)
    let got = PathBuf::from(d["root"].as_str().unwrap());
    let got = std::fs::canonicalize(&got).unwrap_or(got);
    let want = std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
    assert_eq!(got, want);
    assert_eq!(d["operation_state"], "none");
}

#[test]
fn state_untracked_unstaged_staged() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());

    // untracked
    write(dir.path(), "new.txt", "new\n");
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.data()["untracked"], 1);
    assert_eq!(out.data()["dirty"], true);

    // stage everything -> staged clean
    git(dir.path(), &["add", "-A"]);
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.data()["untracked"], 0);
    assert_eq!(out.data()["staged"], 1);

    // unstaged modification on top of a committed file
    git(dir.path(), &["commit", "-m", "add new"]);
    write(dir.path(), "README.md", "# fixture\nedited\n");
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.data()["unstaged"], 1);
    assert_eq!(out.data()["staged"], 0);
}

#[test]
fn state_staged_and_unstaged_simultaneously() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(dir.path(), "README.md", "# fixture\nedit one\n");
    git(dir.path(), &["add", "README.md"]);
    write(dir.path(), "README.md", "# fixture\nedit one\nedit two\n");
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.data()["staged"], 1);
    assert_eq!(out.data()["unstaged"], 1);
}

#[test]
fn state_detached_head() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    git(dir.path(), &["checkout", "--detach", "HEAD"]);
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["detached"], true);
    assert!(d["branch"].is_null());
}

#[test]
fn state_upstream_ahead_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("origin.git");
    git(tmp.path(), &["init", "--bare", bare.to_str().unwrap()]);

    // seed the remote
    let seed = tmp.path().join("seed");
    init_repo_with_commit(&seed);
    git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&seed, &["push", "-u", "origin", "main"]);
    // point the bare repo's HEAD at main so clones check out main deterministically
    git(
        tmp.path(),
        &[
            "--git-dir",
            bare.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );

    git(
        tmp.path(),
        &["clone", "--quiet", bare.to_str().unwrap(), "work"],
    );
    let work = tmp.path().join("work");
    git_identity(&work);

    // new commit on the remote side
    write(&seed, "remote.txt", "remote\n");
    git(&seed, &["add", "-A"]);
    git(&seed, &["commit", "-m", "remote change"]);
    git(&seed, &["push", "origin", "main"]);

    // work fetches (behind 1) and commits locally (ahead 1)
    git(&work, &["fetch", "origin"]);
    write(&work, "local.txt", "local\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "local change"]);

    let out = run_json(&work, &["repo", "state"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let d = out.data();
    assert_eq!(d["upstream"], "origin/main");
    assert_eq!(d["ahead"], 1);
    assert_eq!(d["behind"], 1);
}

#[test]
fn state_paths_with_spaces_and_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work dir");
    init_repo_with_commit(&work);
    write(&work, "my file Ünï.txt", "changed\n");
    let out = run_json(&work, &["repo", "state"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["untracked"], 1);
    assert_eq!(d["dirty"], true);

    let out = run_json(&work, &["repo", "changed"]);
    assert_eq!(out.code, 0);
    let files = out.data()["files"].as_array().unwrap().clone();
    assert!(files
        .iter()
        .any(|f| f["path"].as_str().unwrap().contains("my file")));
    assert!(files.iter().all(|f| f["untracked"] == true));
}

#[test]
fn changed_reports_staged_and_renames() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());

    // clean tree -> no results
    let out = run_json(dir.path(), &["repo", "changed"]);
    assert_eq!(out.code, 4);
    assert_eq!(out.status_str(), "no_results");

    write(dir.path(), "a.txt", "a\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "add a"]);
    git(dir.path(), &["mv", "a.txt", "b name.txt"]);

    let out = run_json(dir.path(), &["repo", "changed"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["staged"], 1);
    let f = &d["files"][0];
    assert_eq!(f["path"], "b name.txt");
    assert_eq!(f["orig_path"], "a.txt");
    assert_eq!(f["staged"], true);
}

#[test]
fn state_and_changed_see_merge_conflict() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    write(dir.path(), "f.txt", "line1\nline2 base\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "base"]);

    git(dir.path(), &["checkout", "-b", "feature"]);
    write(dir.path(), "f.txt", "line1\nline2 feature\n");
    git(dir.path(), &["commit", "-am", "feature change"]);

    git(dir.path(), &["checkout", "main"]);
    write(dir.path(), "f.txt", "line1\nline2 main\n");
    git(dir.path(), &["commit", "-am", "main change"]);

    let (code, _, _) = git(dir.path(), &["merge", "feature"]);
    assert_ne!(code, 0, "merge should conflict");

    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["conflicts"], 1);
    assert_eq!(d["operation_state"], "merge");
    assert_eq!(d["dirty"], true);

    let out = run_json(dir.path(), &["repo", "changed"]);
    assert_eq!(out.code, 0);
    let files = out.data()["files"].as_array().unwrap().clone();
    assert!(files.iter().any(|f| f["conflict"] == true));
}

#[test]
fn changed_is_bounded_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    for i in 0..12 {
        write(dir.path(), &format!("f{i}.txt"), "x\n");
    }
    let out = run_json(dir.path(), &["repo", "changed", "--limit", "5"]);
    assert_eq!(out.code, 0);
    let v = out.json();
    assert_eq!(v["data"]["files"].as_array().unwrap().len(), 5);
    assert_eq!(v["truncated"], true);
    assert_eq!(v["data"]["total"], 12);
}

#[test]
fn history_lists_commits_and_respects_limit() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    git(
        dir.path(),
        &["commit", "--allow-empty", "-m", "second commit"],
    );
    git(
        dir.path(),
        &["commit", "--allow-empty", "-m", "third commit"],
    );

    let out = run_json(dir.path(), &["repo", "history"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["count"], 3);
    let subjects: Vec<String> = d["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["subject"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        subjects,
        vec!["third commit", "second commit", "initial commit"]
    );
    for c in d["commits"].as_array().unwrap() {
        assert_eq!(c["sha"].as_str().unwrap().len(), 40);
        assert!(!c["author"].as_str().unwrap().is_empty());
        assert!(!c["timestamp"].as_str().unwrap().is_empty());
    }

    let out = run_json(dir.path(), &["repo", "history", "--limit", "2"]);
    assert_eq!(out.code, 0);
    let v = out.json();
    assert_eq!(v["data"]["count"], 2);
    assert_eq!(v["truncated"], true);
}

#[test]
fn history_scoped_to_path() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(dir.path(), "special.txt", "v1\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "touch special"]);

    let out = run_json(dir.path(), &["repo", "history", "special.txt"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["target"], "special.txt");
    assert_eq!(d["count"], 1);
    assert_eq!(d["commits"][0]["subject"], "touch special");
}

#[test]
fn remote_reports_github_identity() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());

    let out = run_json(dir.path(), &["repo", "remote"]);
    assert_eq!(out.code, 4);

    git(
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/octocat/Hello-World.git",
        ],
    );
    let out = run_json(dir.path(), &["repo", "remote"]);
    assert_eq!(out.code, 0);
    let remotes = out.data()["remotes"].as_array().unwrap().clone();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0]["github"], true);
    assert_eq!(remotes[0]["owner"], "octocat");
    assert_eq!(remotes[0]["repo"], "Hello-World");

    // ssh form
    git(
        dir.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "git@github.com:acme/widgets.git",
        ],
    );
    let out = run_json(dir.path(), &["repo", "remote"]);
    assert_eq!(out.data()["remotes"][0]["owner"], "acme");
    assert_eq!(out.data()["remotes"][0]["repo"], "widgets");
}

#[test]
fn pr_and_ci_without_auth_report_cleanly_no_network() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/octocat/Hello-World.git",
        ],
    );

    // Point gh at an empty config dir: unauthenticated, zero network I/O.
    let cfg = tempfile::tempdir().unwrap();
    for cmd in [["repo", "pr"], ["repo", "ci"]] {
        let out = run_json_env(
            dir.path(),
            &cmd,
            &[
                ("GH_CONFIG_DIR", Some(cfg.path().to_str().unwrap())),
                ("GH_TOKEN", None),
                ("GITHUB_TOKEN", None),
            ],
        );
        assert_eq!(
            out.code, 3,
            "{cmd:?} should be capability_unavailable: {}",
            out.stderr
        );
        assert_eq!(out.status_str(), "capability_unavailable");
    }
}

#[test]
fn not_a_repository_is_a_clear_failure() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.status_str(), "failed");
    assert!(out.json()["summary"]
        .as_str()
        .unwrap()
        .contains("not inside a git repository"));
}

#[test]
fn missing_git_is_capability_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let empty = tempfile::tempdir().unwrap();
    for cmd in [
        vec!["repo", "state"],
        vec!["repo", "changed"],
        vec!["repo", "history"],
        vec!["repo", "remote"],
    ] {
        let out = run_json_env(
            dir.path(),
            &cmd,
            &[("PATH", Some(empty.path().to_str().unwrap()))],
        );
        assert_eq!(out.code, 3, "{cmd:?}: {}", out.stderr);
        assert_eq!(out.status_str(), "capability_unavailable");
    }
}

#[test]
fn state_envelope_carries_evidence() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_json(dir.path(), &["repo", "state"]);
    let v = out.json();
    let ev = v["evidence"].as_array().unwrap();
    assert!(ev
        .iter()
        .any(|e| e["kind"] == "command" && e["program"] == "git"));
    assert!(ev.iter().all(|e| e["duration_ms"].as_u64().is_some()));
}

#[test]
fn repo_in_directory_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("my work");
    init_repo_with_commit(&work);
    let out = run_json(&work, &["repo", "state"]);
    assert_eq!(out.code, 0);
    assert!(out.data()["root"]
        .as_str()
        .unwrap()
        .replace('\\', "/")
        .contains("my work"));
}

// ------------------------------------------------------- repo pr repair (0.1.1)

fn github_remote(dir: &std::path::Path) {
    git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/octocat/Hello-World.git",
        ],
    );
}

/// PATH where `git` resolves but `gh` must not (hermetic gh-absence proof).
/// Mirrors the isolation pattern used by the search fallback tests.
fn git_only_path() -> (tempfile::TempDir, PathBuf) {
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

#[test]
fn pr_without_any_remote_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_json(dir.path(), &["repo", "pr"]);
    assert_eq!(out.code, 3);
    assert_eq!(out.status_str(), "capability_unavailable");
    assert!(out.json()["summary"]
        .as_str()
        .unwrap()
        .contains("no GitHub remote"));
}

#[test]
fn pr_with_non_github_remote_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    git(
        dir.path(),
        &["remote", "add", "origin", "https://gitlab.com/o/r.git"],
    );
    let out = run_json(dir.path(), &["repo", "pr"]);
    assert_eq!(out.code, 3);
    assert_eq!(out.status_str(), "capability_unavailable");
    assert!(out.json()["summary"]
        .as_str()
        .unwrap()
        .contains("no GitHub remote"));
}

#[test]
fn pr_with_malformed_github_remote_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    // Parses as a remote, but carries no owner/repo identity.
    git(
        dir.path(),
        &["remote", "add", "origin", "https://github.com/o/"],
    );
    let out = run_json(dir.path(), &["repo", "pr"]);
    assert_eq!(out.code, 3);
    assert_eq!(out.status_str(), "capability_unavailable");
}

#[test]
fn pr_without_gh_is_capability_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    github_remote(dir.path());
    let (_guard, path) = git_only_path();
    let out = run_json_env(
        dir.path(),
        &["repo", "pr"],
        &[
            ("PATH", Some(path.to_str().unwrap())),
            ("GH_TOKEN", None),
            ("GITHUB_TOKEN", None),
        ],
    );
    assert_eq!(out.code, 3, "stderr: {}", out.stderr);
    assert_eq!(out.status_str(), "capability_unavailable");
}

#[test]
fn pr_on_detached_head_never_guesses_a_selector() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    github_remote(dir.path());
    git_ok_cmd(dir.path(), &["checkout", "--detach"]);
    let out = run_json(dir.path(), &["repo", "pr"]);
    // Answered from local repository truth alone: no gh, no auth, no network.
    assert_eq!(out.code, 4, "stderr: {}", out.stderr);
    assert_eq!(out.status_str(), "no_results");
    assert!(out.json()["summary"].as_str().unwrap().contains("detached"));
}

fn git_ok_cmd(dir: &std::path::Path, args: &[&str]) {
    let (code, so, se) = git(dir, args);
    assert_eq!(code, 0, "git {args:?} failed: {se}{so}");
}
