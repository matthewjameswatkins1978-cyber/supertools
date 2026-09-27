//! Common contract tests: JSON envelope shape, exit codes, describe/teach
//! catalogue behaviour, ANSI cleanliness, stable operation ids.

mod common;

use common::*;

#[test]
fn version_flag_works() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_human(dir.path(), &["--version"]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn envelope_shape_is_canonical() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_eq!(out.code, 0);
    let v = assert_envelope(&out, "repo.state");
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"], "ok");
}

#[test]
fn human_mode_has_no_ansi_and_is_compact() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_human(dir.path(), &["repo", "state"]);
    assert_eq!(out.code, 0);
    assert_no_ansi(&out.stdout);
    assert!(out.stdout.lines().count() < 25, "human output too verbose");
}

#[test]
fn exit_code_contract() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(dir.path(), "notes.txt", "alpha\n");

    // 0: success with results
    let out = run_json(dir.path(), &["search", "text", "alpha", "--fixed"]);
    assert_eq!(out.code, 0);

    // 4: completed, no matches
    let out = run_json(
        dir.path(),
        &["search", "text", "zzz-no-such-needle", "--fixed"],
    );
    assert_eq!(out.code, 4);
    assert_eq!(out.status_str(), "no_results");
    let v = out.json();
    assert_eq!(v["ok"], true, "no_results is still a successful completion");

    // 2: invalid request
    let out = run_json(dir.path(), &["describe", "no-such-target"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.status_str(), "invalid_request");

    // 1: failure (not a git repository)
    let plain = tempfile::tempdir().unwrap();
    let out = run_json(plain.path(), &["repo", "state"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.status_str(), "failed");
}

#[test]
fn describe_lists_operations_and_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["describe"]);
    assert_eq!(out.code, 0);
    let v = assert_envelope(&out, "describe");
    let ops = v["data"]["operations"].as_array().unwrap();
    assert!(ops.len() >= 20);
    let ids: Vec<&str> = ops.iter().map(|o| o["id"].as_str().unwrap()).collect();
    for expected in [
        "search.text",
        "repo.state",
        "verify.discover",
        "tools.list",
        "find.tool",
        "common.doctor",
    ] {
        assert!(ids.contains(&expected), "missing {expected}");
    }
    let codes = v["data"]["exit_codes"].as_array().unwrap();
    assert!(codes.iter().any(|c| c["code"] == "0"));
    assert!(codes.iter().any(|c| c["code"] == "4"));
}

#[test]
fn describe_domain_and_operation() {
    let dir = tempfile::tempdir().unwrap();

    let out = run_json(dir.path(), &["describe", "search"]);
    assert_eq!(out.code, 0);
    let ops = out.data()["operations"].as_array().unwrap().clone();
    assert!(ops.iter().all(|o| o["domain"] == "search"));
    assert!(ops.iter().any(|o| o["id"] == "search.text"));

    let out = run_json(dir.path(), &["describe", "search.text"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["id"], "search.text");
    assert_eq!(d["read_only"], true);
    assert_eq!(d["destructive"], false);
    assert!(d["when_to_use"].as_array().is_some_and(|a| !a.is_empty()));
    assert!(d["requires"].as_array().is_some());

    // schema documentation is available for agents
    let out = run_json(dir.path(), &["describe", "schema"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.data()["envelope"]["schema_version"], 1);
}

#[test]
fn describe_and_cli_agree_on_operation_safety_flags() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["describe"]);
    let ops = out.data()["operations"].as_array().unwrap().clone();
    for o in &ops {
        let id = o["id"].as_str().unwrap();
        if id.starts_with("search.") || id.starts_with("repo.") {
            assert_eq!(o["read_only"], true, "{id} must be read-only in v0.1");
            assert_eq!(o["destructive"], false, "{id} must not be destructive");
        }
    }
}

#[test]
fn teach_domains_are_compact_and_canonical() {
    let dir = tempfile::tempdir().unwrap();
    for topic in ["search", "repo", "verify", "tools"] {
        let out = run_human(dir.path(), &["teach", topic]);
        assert_eq!(out.code, 0, "teach {topic} failed: {}", out.stderr);
        assert_no_ansi(&out.stdout);
        assert!(
            out.stdout.lines().count() <= 60,
            "teach {topic} must stay compact"
        );
        assert!(
            out.stdout.contains("supertools"),
            "canonical commands missing"
        );
    }
    let out = run_json(dir.path(), &["teach", "verify"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert!(d["progression"].as_array().is_some_and(|p| !p.is_empty()));
}

#[test]
fn teach_unknown_topic_is_invalid_request() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["teach", "nonsense"]);
    assert_eq!(out.code, 2);
}

#[test]
fn teach_install_emits_tiny_block_and_managed_file_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();

    // stdout mode: the block itself, small.
    let out = run_json(dir.path(), &["teach", "install", "--target", "stdout"]);
    assert_eq!(out.code, 0);
    let block = out.data()["block"].as_str().unwrap().to_string();
    assert!(block.contains("supertools search"));
    assert!(block.contains("supertools repo"));
    assert!(block.contains("supertools verify"));
    assert!(block.contains("supertools teach <domain>"));
    assert!(
        block.lines().count() < 30,
        "persistent instruction must stay tiny"
    );
    assert!(block.contains("supertools:begin"));
    assert!(block.contains("supertools:end"));

    // managed block into an existing file: preserves content, idempotent.
    let agents = dir.path().join("AGENTS.md");
    std::fs::write(&agents, "# USER CONTENT\n\nDo not lose me.\n").unwrap();

    let out = run_json(
        dir.path(),
        &[
            "teach",
            "install",
            "--path",
            agents.to_str().unwrap(),
            "--apply",
        ],
    );
    assert_eq!(out.code, 0);
    let content = std::fs::read_to_string(&agents).unwrap();
    assert!(content.contains("Do not lose me."));
    assert!(content.contains("supertools:begin"));
    assert_eq!(content.matches("supertools:begin").count(), 1);

    // Second apply: unchanged.
    let out = run_json(
        dir.path(),
        &[
            "teach",
            "install",
            "--path",
            agents.to_str().unwrap(),
            "--apply",
        ],
    );
    assert_eq!(out.code, 0);
    let actions: Vec<String> = out.data()["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["action"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(actions, vec!["unchanged".to_string()]);
    let content2 = std::fs::read_to_string(&agents).unwrap();
    assert_eq!(content, content2);
    assert_eq!(content2.matches("supertools:begin").count(), 1);

    // Preview (no --apply) never writes.
    let fresh = dir.path().join("FRESH.md");
    let out = run_json(
        dir.path(),
        &["teach", "install", "--path", fresh.to_str().unwrap()],
    );
    assert_eq!(out.code, 0);
    assert!(!fresh.exists(), "preview must not create files");
    assert_eq!(out.data()["applied"], false);
}

#[test]
fn capabilities_reports_operation_readiness_not_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["capabilities"]);
    assert_eq!(out.code, 0);
    let caps = out.data()["capabilities"].as_array().unwrap().clone();
    assert!(caps.len() >= 15);
    for c in &caps {
        assert!(c["id"].is_string());
        let s = c["status"].as_str().unwrap();
        assert!(
            ["ready", "degraded", "unavailable"].contains(&s),
            "bad status {s}"
        );
    }
    // git is installed on this machine and in CI => repo.state must be ready.
    let repo_state = caps.iter().find(|c| c["id"] == "repo.state").unwrap();
    assert_eq!(repo_state["status"], "ready");
}

#[test]
fn doctor_classifies_health_and_never_mutates() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_json(dir.path(), &["doctor"]);
    assert!(out.code == 0 || out.code == 3);
    let v = assert_envelope(&out, "doctor");
    let overall = v["data"]["overall"].as_str().unwrap();
    assert!([
        "healthy",
        "healthy_with_optional_gaps",
        "degraded",
        "broken"
    ]
    .contains(&overall));
    assert!(v["data"]["checks"]
        .as_array()
        .is_some_and(|c| !c.is_empty()));
    // repo context is present when inside a repository
    assert!(v["data"]["repo_context"].is_object());
    assert_eq!(v["data"]["repo_context"]["dirty"], false);
}

#[test]
fn json_stdout_is_pure_json() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    let out = run_json(dir.path(), &["repo", "state"]);
    let trimmed = out.stdout.trim();
    assert!(trimmed.starts_with('{') && trimmed.ends_with('}'));
    assert_no_ansi(&out.stdout);
}
