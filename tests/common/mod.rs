//! Shared helpers for integration tests. Every test runs the real
//! `supertools` binary against temporary fixture directories — never
//! against Matthew's real repositories.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_supertools"))
}

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    /// Panics with context if stdout is not clean, parseable JSON.
    pub fn json(&self) -> serde_json::Value {
        assert_no_ansi(&self.stdout);
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not valid JSON ({e}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.stdout, self.stderr
            )
        })
    }

    pub fn status_str(&self) -> String {
        self.json()["status"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    pub fn data(&self) -> serde_json::Value {
        self.json()["data"].clone()
    }
}

pub fn assert_no_ansi(s: &str) {
    assert!(
        !s.contains('\u{1b}'),
        "output contains ANSI escape sequences: {:?}",
        s.chars().take(200).collect::<String>()
    );
}

fn base(dir: &Path, env: &[(&str, Option<&str>)]) -> Command {
    let mut cmd = Command::new(exe());
    cmd.current_dir(dir);
    for (k, v) in env {
        match v {
            Some(v) => {
                cmd.env(k, v);
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
    cmd
}

/// Run with `--json` (canonical machine output).
pub fn run_json(dir: &Path, args: &[&str]) -> Out {
    run_json_env(dir, args, &[])
}

pub fn run_json_env(dir: &Path, args: &[&str], env: &[(&str, Option<&str>)]) -> Out {
    let out = base(dir, env)
        .arg("--json")
        .args(args)
        .output()
        .expect("spawn supertools");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Run in default human mode.
pub fn run_human(dir: &Path, args: &[&str]) -> Out {
    run_human_env(dir, args, &[])
}

pub fn run_human_env(dir: &Path, args: &[&str], env: &[(&str, Option<&str>)]) -> Out {
    let out = base(dir, env)
        .args(args)
        .output()
        .expect("spawn supertools");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

// ------------------------------------------------------------ git fixtures

pub fn git(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn git");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn git_ok(dir: &Path, args: &[&str]) {
    let (code, so, se) = git(dir, args);
    assert_eq!(code, 0, "git {args:?} failed: {se}{so}");
}

/// Deterministic local identity for fixture repositories.
pub fn git_identity(dir: &Path) {
    git_ok(dir, &["config", "user.name", "Fixture"]);
    git_ok(dir, &["config", "user.email", "fixture@example.invalid"]);
    git_ok(dir, &["config", "commit.gpgsign", "false"]);
}

pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git_ok(dir, &["init", "-b", "main"]);
    git_identity(dir);
}

pub fn init_repo_with_commit(dir: &Path) {
    init_repo(dir);
    write(dir, "README.md", "# fixture\n");
    git_ok(dir, &["add", "-A"]);
    git_ok(dir, &["commit", "-m", "initial commit"]);
}

pub fn write(dir: &Path, name: &str, content: &str) {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

pub fn tool_available(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Envelope contract assertions shared by many tests.
pub fn assert_envelope(out: &Out, operation: &str) -> serde_json::Value {
    let v = out.json();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["tool"], "supertools");
    assert!(v["version"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(v["operation"], operation);
    assert!(v["ok"].is_boolean());
    assert!(v["status"].is_string());
    assert!(v["summary"].is_string());
    assert!(v["data"].is_object() || v["data"].is_null());
    assert!(v["evidence"].is_array());
    assert!(v["warnings"].is_array());
    assert!(v["next_actions"].is_array());
    assert!(v["truncated"].is_boolean());
    v
}
