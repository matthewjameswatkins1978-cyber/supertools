//! Verify domain tests: authority discovery, precedence, fail-closed
//! ambiguity, declared-task-only execution, timeouts and truncation.
//!
//! Execution tests use real minimal cargo fixtures (no dependencies, so no
//! network). Discovery-only tests cover the other authorities.

mod common;

use common::*;

fn cargo_crate(dir: &std::path::Path, lib_rs: &str) {
    write(
        dir,
        "Cargo.toml",
        "[package]\nname = \"stfx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(dir, "src/lib.rs", lib_rs);
}

fn passing_crate(dir: &std::path::Path) {
    cargo_crate(
        dir,
        "pub fn ok() -> bool { true }\n\n#[cfg(test)]\nmod t {\n    #[test]\n    fn it() { assert!(super::ok()); }\n}\n",
    );
}

#[test]
fn cargo_discovery_maps_quick_check_full_test() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 0);
    let v = assert_envelope(&out, "verify.discover");
    let d = &v["data"];
    assert_eq!(d["authority"]["kind"], "cargo");
    assert_eq!(d["authority"]["source"], "Cargo.toml");
    assert_eq!(d["quick"]["task"], "check");
    assert!(d["quick"]["command"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "check"));
    assert_eq!(d["full"]["task"], "test");
    assert_eq!(d["ambiguous"], false);
}

#[test]
fn verify_quick_passes_with_evidence() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    let out = run_json(dir.path(), &["verify", "quick"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let v = assert_envelope(&out, "verify.quick");
    let d = &v["data"];
    assert_eq!(d["passed"], true);
    assert_eq!(d["exit_code"], 0);
    assert_eq!(d["timed_out"], false);
    assert_eq!(d["authority"], "cargo");
    assert!(d["duration_ms"].as_u64().is_some());
    assert!(v["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "command"));
}

#[test]
fn verify_quick_failure_preserves_diagnostics_tail() {
    let dir = tempfile::tempdir().unwrap();
    cargo_crate(dir.path(), "pub fn broken( -> \n");
    let out = run_json(dir.path(), &["verify", "quick"]);
    assert_eq!(out.code, 1);
    let v = assert_envelope(&out, "verify.quick");
    assert_eq!(v["status"], "failed");
    assert_eq!(v["data"]["passed"], false);
    assert_ne!(v["data"]["exit_code"], 0);
    let diag = &v["data"]["diagnostics"];
    let combined = format!(
        "{}{}",
        diag["stdout_tail"].as_str().unwrap_or(""),
        diag["stderr_tail"].as_str().unwrap_or("")
    );
    assert!(
        combined.to_lowercase().contains("error"),
        "failure tail should mention the error: {combined:.400}"
    );
}

#[test]
fn verify_full_runs_the_test_suite() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    let out = run_json(dir.path(), &["verify", "full"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.data()["passed"], true);
    assert_eq!(out.data()["task"], "test");
}

#[test]
fn verify_task_refuses_arbitrary_names() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    for hostile in [
        "rm -rf /",
        "check; echo hi",
        "$(check)",
        "check && echo hi",
        "../../check",
    ] {
        let out = run_json(dir.path(), &["verify", "task", hostile]);
        assert_eq!(out.code, 2, "{hostile:?}: {}", out.stderr);
        assert_eq!(out.status_str(), "refused");
    }
    // ...but runs a declared task by exact name
    let out = run_json(dir.path(), &["verify", "task", "check"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.data()["passed"], true);
}

#[test]
fn verify_task_timeout_kills_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    write(
        dir.path(),
        "tests/slow.rs",
        "#[test]\nfn slow() { std::thread::sleep(std::time::Duration::from_secs(60)); }\n",
    );
    let out = run_json(dir.path(), &["verify", "task", "test", "--timeout", "1"]);
    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    assert_eq!(out.status_str(), "timeout");
    assert_eq!(out.data()["timed_out"], true);
}

#[test]
fn verify_output_is_truncated_with_tail_preserved() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    write(
        dir.path(),
        "tests/spam.rs",
        "#[test]\nfn spam() {\n    for i in 0..30000 { println!(\"line{i:05}\"); }\n    panic!(\"boom\");\n}\n",
    );
    let out = run_json(dir.path(), &["verify", "task", "test"]);
    assert_eq!(out.code, 1);
    let v = out.json();
    assert_eq!(v["truncated"], true);
    let tail = v["data"]["diagnostics"]["stdout_tail"]
        .as_str()
        .unwrap_or("")
        .to_string()
        + v["data"]["diagnostics"]["stderr_tail"]
            .as_str()
            .unwrap_or("");
    assert!(tail.len() <= 17_000, "tail too large: {}", tail.len());
    assert!(
        tail.contains("boom"),
        "failure tail must preserve the panic"
    );
}

#[test]
fn package_scripts_are_discovered_without_execution() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "package.json",
        "{\"name\":\"x\",\"scripts\":{\"check\":\"echo check\",\"test\":\"echo test\"}}",
    );
    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["authority"]["kind"], "package");
    assert!(d["authority"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "test"));
    // heuristic: quick=check exists here, full=test
    assert_eq!(d["quick"]["task"], "check");
    assert_eq!(d["full"]["task"], "test");
}

#[test]
fn conflicting_runners_fail_closed_with_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "justfile", "check:\n    @echo ok\n");
    write(
        dir.path(),
        "package.json",
        "{\"scripts\":{\"test\":\"echo ok\"}}",
    );

    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.status_str(), "ambiguous");
    let v = out.json();
    assert!(v["data"]["ambiguity"].as_array().unwrap().len() >= 2);
    assert!(v["summary"].as_str().unwrap().contains(".supertools.toml"));

    // quick/full refuse to guess as well
    let out = run_json(dir.path(), &["verify", "quick"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.status_str(), "ambiguous");
}

#[test]
fn explicit_config_resolves_ambiguity() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "justfile", "check:\n    @echo ok\n");
    write(
        dir.path(),
        "package.json",
        "{\"scripts\":{\"test\":\"echo ok\"}}",
    );
    write(
        dir.path(),
        ".supertools.toml",
        "[verify]\nauthority = \"package\"\n",
    );

    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.data()["authority"]["kind"], "package");
}

#[test]
fn malformed_or_unknown_config_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
    );
    write(dir.path(), ".supertools.toml", "[verify\nauthority = ");
    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 2);

    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".supertools.toml",
        "[verify]\nauthority = \"make\"\n",
    );
    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 2);
    assert!(out.json()["summary"]
        .as_str()
        .unwrap()
        .contains("unknown authority"));
}

#[test]
fn config_quick_must_name_a_declared_task() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    write(
        dir.path(),
        ".supertools.toml",
        "[verify]\nquick = \"no-such-task\"\n",
    );
    let out = run_json(dir.path(), &["verify", "quick"]);
    assert_eq!(out.code, 2, "stderr: {}", out.stderr);
}

#[test]
fn no_authority_is_exit_4_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_eq!(out.code, 4);
    assert_eq!(out.status_str(), "no_results");
    assert!(out.data()["authority"].is_null());

    let out = run_json(dir.path(), &["verify", "quick"]);
    assert_eq!(out.code, 4);
}

#[test]
fn timeout_flag_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    passing_crate(dir.path());
    let out = run_json(dir.path(), &["verify", "quick", "--timeout", "300"]);
    assert_eq!(out.code, 0);
}
