//! JSON non-regression contract.
//!
//! The `--json` envelope is the machine/agent surface and the presentation
//! layer (Sartorial) must never touch it. These tests pin the EXACT data-key
//! sets of representative operations as captured before the presentation
//! adapter existed. Any accidental key addition/removal/rename — including
//! ANSI or human-only labels leaking into data — fails here.

mod common;

use common::*;

fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v
        .as_object()
        .expect("data is an object")
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

const ENVELOPE_KEYS: &[&str] = &[
    "data",
    "evidence",
    "next_actions",
    "ok",
    "operation",
    "schema_version",
    "status",
    "summary",
    "tool",
    "truncated",
    "version",
    "warnings",
];

fn assert_envelope_keys(out: &Out) -> serde_json::Value {
    let v = out.json(); // also asserts stdout is ANSI-free parseable JSON
    let mut top: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    top.sort();
    assert_eq!(top, ENVELOPE_KEYS, "envelope key set changed");
    v
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_commit(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        "pub fn calculate_total() -> u32 { 1 }\n",
    );
    dir
}

#[test]
fn doctor_data_keys_are_stable() {
    let dir = fixture();
    let out = run_json(dir.path(), &["doctor"]);
    assert_envelope_keys(&out);
    assert_eq!(
        sorted_keys(&out.data()),
        [
            "capability_summary", // deliberate v0.1 closeout addition (Part D)
            "checks",
            "optional_improvements",
            "overall",
            "platform",
            "repo_context",
            "supertools_version",
        ]
    );
    // capability_summary itself is counts only — derived, bounded, no truth.
    let cs = &out.data()["capability_summary"];
    assert_eq!(sorted_keys(cs), ["degraded", "ready", "unavailable"]);
}

#[test]
fn capabilities_data_keys_are_stable() {
    let dir = fixture();
    let out = run_json(dir.path(), &["capabilities"]);
    assert_envelope_keys(&out);
    assert_eq!(sorted_keys(&out.data()), ["capabilities", "summary"]);
    let cap = &out.data()["capabilities"][0];
    assert_eq!(
        sorted_keys(cap),
        ["backend", "id", "missing_tools", "note", "status"]
    );
}

#[test]
fn tools_data_keys_are_stable() {
    let dir = fixture();
    let out = run_json(dir.path(), &["tools"]);
    assert_envelope_keys(&out);
    assert_eq!(
        sorted_keys(&out.data()),
        ["deep", "platform", "summary", "tools"]
    );
    let tool = &out.data()["tools"][0];
    for required in ["id", "name", "status", "recommended", "required", "role"] {
        assert!(tool.get(required).is_some(), "tool record lost {required}");
    }
}

#[test]
fn repo_state_data_keys_are_stable() {
    let dir = fixture();
    let out = run_json(dir.path(), &["repo", "state"]);
    assert_envelope_keys(&out);
    assert_eq!(
        sorted_keys(&out.data()),
        [
            "ahead",
            "behind",
            "branch",
            "conflicts",
            "detached",
            "dirty",
            "github",
            "head",
            "operation_state",
            "remote_url",
            "root",
            "staged",
            "unstaged",
            "untracked",
            "upstream",
        ]
    );
}

#[test]
fn search_text_data_keys_are_stable() {
    let dir = fixture();
    if !tool_available("rg") {
        return; // fallback backend has its own documented shape
    }
    let out = run_json(dir.path(), &["search", "text", "calculate_total"]);
    assert_envelope_keys(&out);
    assert_eq!(
        sorted_keys(&out.data()),
        [
            "backend",
            "match_count",
            "matches",
            "mode",
            "query",
            "root",
            "total_match_events",
        ]
    );
    let m = &out.data()["matches"][0];
    assert_eq!(
        sorted_keys(m),
        ["column", "line", "matched", "path", "text"]
    );
}

#[test]
fn verify_discover_data_keys_are_stable() {
    let dir = fixture();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
    );
    let out = run_json(dir.path(), &["verify", "discover"]);
    assert_envelope_keys(&out);
    assert_eq!(
        sorted_keys(&out.data()),
        [
            "ambiguity",
            "ambiguous",
            "authority",
            "candidates",
            "config",
            "full",
            "notes",
            "quick",
            "root",
            "tasks_truncated",
        ]
    );
}

#[test]
fn json_never_contains_presentation_vocabulary() {
    let dir = fixture();
    for args in [
        &["doctor"][..],
        &["capabilities"][..],
        &["tools"][..],
        &["repo", "state"][..],
        &["verify", "discover"][..],
    ] {
        let out = run_json(dir.path(), args);
        let raw = &out.stdout;
        assert_no_ansi(raw);
        // Human-only decoration vocabulary must never appear in JSON.
        for banned in [
            "\u{2500}",
            "\u{2502}",
            "\u{250c}",
            "\u{2514}",
            "READY",
            "ATTENTION",
        ] {
            assert!(
                !raw.contains(banned),
                "{args:?} JSON contains presentation fragment {banned:?}"
            );
        }
    }
}
