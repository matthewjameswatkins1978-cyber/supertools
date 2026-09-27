//! Centralised subprocess execution.
//!
//! Every external tool invocation in Supertools goes through [`run`]:
//! argument vectors only (never a shell), bounded captured output,
//! bounded waiting, separate stdout/stderr, Windows-safe invocation.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const DEFAULT_STDOUT_CAP: usize = 4 * 1024 * 1024;
pub const DEFAULT_STDERR_CAP: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    /// Executable name (resolved via PATH) or absolute path.
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
}

impl Request {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: None,
            timeout: Duration::from_secs(60),
            max_stdout: DEFAULT_STDOUT_CAP,
            max_stderr: DEFAULT_STDERR_CAP,
        }
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    pub fn max_stdout(mut self, n: usize) -> Self {
        self.max_stdout = n;
        self
    }

    pub fn max_stderr(mut self, n: usize) -> Self {
        self.max_stderr = n;
        self
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    /// Lossy UTF-8 stdout, bounded by `max_stdout` (leading bytes kept).
    pub stdout: String,
    /// Lossy UTF-8 stderr, bounded by `max_stderr`.
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
}

impl Outcome {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcError {
    #[error("executable not found: {0}")]
    NotFound(String),
    #[error("failed to spawn {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

/// Resolve `program` to an absolute path when it is a bare name.
pub fn resolve_program(program: &str) -> Result<PathBuf, ProcError> {
    if Path::new(program).is_absolute() {
        return Ok(PathBuf::from(program));
    }
    which::which(program).map_err(|_| ProcError::NotFound(program.to_string()))
}

fn read_bounded<R: Read>(mut r: R, cap: usize) -> (Vec<u8>, bool) {
    let mut out: Vec<u8> = Vec::with_capacity(cap.min(64 * 1024));
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(out.len());
                let take = n.min(room);
                if take > 0 {
                    out.extend_from_slice(&chunk[..take]);
                }
                if take < n {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (out, truncated)
}

/// Execute a bounded subprocess. Never uses a shell; arguments are passed
/// directly as a vector, so query content cannot be interpreted as commands.
pub fn run(req: &Request) -> Result<Outcome, ProcError> {
    let resolved = resolve_program(&req.program)?;
    let mut cmd = std::process::Command::new(&resolved);
    cmd.args(&req.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &req.cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|source| ProcError::Spawn {
        program: req.program.clone(),
        source,
    })?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (max_out, max_err) = (req.max_stdout, req.max_stderr);
    let out_handle = stdout_pipe.map(|p| std::thread::spawn(move || read_bounded(p, max_out)));
    let err_handle = stderr_pipe.map(|p| std::thread::spawn(move || read_bounded(p, max_err)));

    let mut timed_out = false;
    let mut exit_status: Option<std::process::ExitStatus> = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                if start.elapsed() >= req.timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
        }
    }

    let (stdout_bytes, stdout_truncated) = out_handle
        .and_then(|h| h.join().ok())
        .unwrap_or((Vec::new(), false));
    let (stderr_bytes, stderr_truncated) = err_handle
        .and_then(|h| h.join().ok())
        .unwrap_or((Vec::new(), false));

    Ok(Outcome {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code: exit_status.and_then(|s| s.code()),
        timed_out,
        stdout_truncated,
        stderr_truncated,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Keep at most `max_bytes` trailing bytes of `s` (on a char boundary).
/// Returns (tail, truncated).
pub fn tail(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    (s[start..].to_string(), true)
}

/// Keep at most `max_bytes` leading bytes of `s` (on a char boundary).
pub fn head(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_is_bounded_and_marks_truncation() {
        let (t, trunc) = tail("abcdefgh", 3);
        assert_eq!(t, "fgh");
        assert!(trunc);
        let (t, trunc) = tail("abc", 10);
        assert_eq!(t, "abc");
        assert!(!trunc);
    }

    #[test]
    fn head_is_bounded() {
        let (h, trunc) = head("abcdefgh", 3);
        assert_eq!(h, "abc");
        assert!(trunc);
    }

    #[test]
    fn tail_respects_char_boundaries() {
        let s = "日本語テキスト";
        let (t, trunc) = tail(s, 4);
        assert!(trunc);
        assert!(t.chars().all(|c| !c.is_ascii()));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_sleeper() {
        let req = Request::new("sleep", vec!["30".to_string()]).timeout(Duration::from_millis(500));
        let out = run(&req).expect("spawn sleep");
        assert!(out.timed_out);
        assert!(out.duration_ms < 10_000);
    }

    #[cfg(windows)]
    #[test]
    fn timeout_kills_sleeper() {
        // ping is present on every Windows machine; -n 30 takes ~29s.
        let req = Request::new(
            "ping",
            vec!["-n".to_string(), "30".to_string(), "127.0.0.1".to_string()],
        )
        .timeout(Duration::from_millis(700));
        let out = run(&req).expect("spawn ping");
        assert!(out.timed_out);
    }

    #[test]
    fn missing_executable_is_not_found() {
        let req = Request::new("supertools-definitely-not-installed-xyz", vec![]);
        let err = run(&req).unwrap_err();
        assert!(matches!(err, ProcError::NotFound(_)));
    }

    #[test]
    fn bounded_stdout_reports_truncation() {
        // git hash-object without a file reads stdin; instead use a program
        // that emits a lot: `git --help`? Too big/slow. Use rustc -vV twice?
        // Simplest portable heavy writer: the test binary itself is complex,
        // so assert the pure helper instead.
        let (kept, truncated) = read_bounded("aaaaaaaaaa".as_bytes(), 4);
        assert_eq!(kept.len(), 4);
        assert!(truncated);
    }
}
