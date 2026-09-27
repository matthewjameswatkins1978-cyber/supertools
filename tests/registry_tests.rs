//! Registry/inventory tests: tools/find/doctor JSON schemas on the real
//! machine, alias resolution, fix-path safety. Real environment facts only —
//! synthetic environments are covered by unit tests.

mod common;

use common::*;

#[test]
fn tools_inventory_schema_and_summary_consistency() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["tools"]);
    assert_eq!(out.code, 0);
    let v = assert_envelope(&out, "tools.list");
    let tools = v["data"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 10, "the recommended toolset is fixed in v0.1");
    for t in tools {
        for field in [
            "id",
            "name",
            "homepage",
            "status",
            "recommended",
            "required",
            "role",
            "on_process_path",
            "on_user_path",
            "on_machine_path",
            "alternates",
            "capabilities",
            "notes",
            "used_by",
            "install_available",
        ] {
            assert!(t.get(field).is_some(), "tool record missing {field}: {t}");
        }
        assert!(["available", "hidden", "missing", "ambiguous", "broken"]
            .contains(&t["status"].as_str().unwrap()));
        assert_eq!(t["recommended"], true);
    }
    let s = &v["data"]["summary"];
    let count = |st: &str| tools.iter().filter(|t| t["status"] == st).count() as u64;
    assert_eq!(s["available"], count("available"));
    assert_eq!(s["hidden"], count("hidden"));
    assert_eq!(s["missing"], count("missing"));
    assert_eq!(s["ambiguous"], count("ambiguous"));
    assert_eq!(s["broken"], count("broken"));
}

#[test]
fn find_git_is_available_with_version() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["find", "git"]);
    assert_eq!(out.code, 0);
    let v = assert_envelope(&out, "find.tool");
    let d = &v["data"];
    assert_eq!(d["id"], "git");
    assert_eq!(d["status"], "available");
    assert_eq!(d["on_process_path"], true);
    assert!(d["path"].as_str().is_some_and(|p| !p.is_empty()));
    assert!(d["version"].as_str().is_some_and(|x| x.contains("git")));
    assert!(d["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "repo.state"));
}

#[test]
fn find_unknown_tool_is_invalid_request() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["find", "definitely-not-a-tool"]);
    assert_eq!(out.code, 2);
}

#[test]
fn tools_show_unknown_tool_is_invalid_request() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["tools", "show", "definitely-not-a-tool"]);
    assert_eq!(out.code, 2);
}

#[test]
fn tools_filters_report_their_own_filter() {
    let dir = tempfile::tempdir().unwrap();
    for filter in ["available", "hidden", "missing"] {
        let out = run_json(dir.path(), &["tools", filter]);
        assert!(out.code == 0 || out.code == 4, "{filter}: {}", out.code);
        let v = out.json();
        assert_eq!(v["data"]["filter"], filter);
        assert_eq!(
            v["data"]["count"],
            v["data"]["tools"].as_array().unwrap().len() as u64
        );
        for t in v["data"]["tools"].as_array().unwrap() {
            assert_eq!(t["status"], filter, "filter {filter} leaked another status");
        }
    }
}

#[test]
fn sg_resolves_to_ast_grep_record() {
    if !(tool_available("ast-grep") || tool_available("sg")) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["find", "sg"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.data()["id"], "ast-grep");
}

#[test]
fn conditional_tool_records_match_reality() {
    if !tool_available("rg") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["tools", "show", "rg"]);
    assert_eq!(out.code, 0);
    let d = out.data();
    assert_eq!(d["status"], "available");
    assert!(d["version"].as_str().unwrap().contains("ripgrep"));
    assert_eq!(d["on_process_path"], true);
}

#[test]
fn capabilities_follow_registry_reality() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["capabilities"]);
    assert_eq!(out.code, 0);
    let caps = out.data()["capabilities"].as_array().unwrap().clone();
    let get = |id: &str| caps.iter().find(|c| c["id"] == id).unwrap().clone();

    // git present everywhere (dev + CI) => repo.state ready
    assert_eq!(get("repo.state")["status"], "ready");
    assert_eq!(get("repo.state")["backend"], "git");

    // search.text readiness tracks rg
    if tool_available("rg") {
        assert_eq!(get("search.text")["status"], "ready");
    } else {
        assert_eq!(get("search.text")["status"], "degraded");
        assert_eq!(get("search.text")["backend"], "internal-fallback");
    }

    // search.files readiness tracks fd, else git fallback
    if tool_available("fd") {
        assert_eq!(get("search.files")["status"], "ready");
        assert_eq!(get("search.files")["backend"], "fd");
    } else {
        assert_eq!(get("search.files")["status"], "degraded");
    }
}

#[test]
fn tools_missing_json_lists_only_missing() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["tools", "missing"]);
    assert!(out.code == 0 || out.code == 4);
    for t in out.data()["tools"].as_array().unwrap() {
        assert_eq!(t["status"], "missing");
    }
}

#[test]
fn install_missing_without_consent_refuses_safely() {
    let dir = tempfile::tempdir().unwrap();
    let before = dir_registry_tools_snapshot(dir.path());
    // non-interactive (piped stdin) without --yes: must refuse, change nothing
    let out = run_human(dir.path(), &["tools", "install-missing"]);
    // Either everything is installed (no_results), or consent is refused (2).
    assert!(
        out.code == 4 || out.code == 2,
        "unexpected code {}: {} {}",
        out.code,
        out.stdout,
        out.stderr
    );
    if out.code == 2 {
        assert!(out.stderr.contains("--yes") || out.stdout.contains("--yes"));
    }
    let after = dir_registry_tools_snapshot(dir.path());
    assert_eq!(before, after, "refused install must not change tool state");
}

fn dir_registry_tools_snapshot(dir: &std::path::Path) -> serde_json::Value {
    run_json(dir, &["tools"]).json()["data"]["tools"].clone()
}

#[cfg(windows)]
#[test]
fn fix_path_dry_run_never_writes() {
    let before = persisted_user_path_raw();
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["tools", "fix-path", "--dry-run"]);
    assert!(
        out.code == 0 || out.code == 4,
        "code {}: {}",
        out.code,
        out.stderr
    );
    assert_eq!(out.data()["dry_run"], true);
    assert_eq!(out.data()["applied"], false);
    let after = persisted_user_path_raw();
    assert_eq!(before, after, "dry-run must not modify USER PATH");
}

#[cfg(windows)]
fn persisted_user_path_raw() -> String {
    let out = std::process::Command::new("reg")
        .args(["query", "HKCU\\Environment", "/v", "Path"])
        .output()
        .expect("reg query");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[cfg(not(windows))]
#[test]
fn fix_path_unsupported_off_windows() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_json(dir.path(), &["tools", "fix-path"]);
    assert_eq!(out.code, 3);
}
