//! Canonical result/evidence model shared by every Supertools operation.
//!
//! Every operation produces exactly one [`Builder`] which is rendered either
//! as the canonical JSON envelope (`--json`) or as concise human output.

use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;
pub const TOOL_NAME: &str = "supertools";

/// Machine-readable outcome classification. Maps 1:1 onto the documented
/// exit-code contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Operation completed and produced a meaningful result.
    Ok,
    /// Operation completed correctly but there were no matches / no
    /// applicable result (e.g. clean search, no open PR).
    NoResults,
    /// The operation ran but the underlying execution failed.
    Failed,
    /// A bounded subprocess exceeded its timeout and was killed.
    Timeout,
    /// The request itself was invalid (bad arguments, unknown target).
    InvalidRequest,
    /// A required backend capability is not present in this environment.
    CapabilityUnavailable,
    /// Supertools refused to guess between conflicting authorities.
    Ambiguous,
    /// Supertools explicitly refused a potentially unsafe request.
    Refused,
}

impl Status {
    pub fn exit_code(self) -> i32 {
        match self {
            Status::Ok => 0,
            Status::NoResults => 4,
            Status::Failed => 1,
            Status::Timeout => 1,
            Status::InvalidRequest => 2,
            Status::CapabilityUnavailable => 3,
            Status::Ambiguous => 2,
            Status::Refused => 2,
        }
    }

    /// `ok` in the envelope means "the operation itself succeeded";
    /// `no_results` is a successful completion with an empty answer.
    pub fn is_ok(self) -> bool {
        matches!(self, Status::Ok | Status::NoResults)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::NoResults => "no_results",
            Status::Failed => "failed",
            Status::Timeout => "timeout",
            Status::InvalidRequest => "invalid_request",
            Status::CapabilityUnavailable => "capability_unavailable",
            Status::Ambiguous => "ambiguous",
            Status::Refused => "refused",
        }
    }
}

/// A single structured fact supporting the result.
#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    /// "command" | "file" | "fact"
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Evidence {
    pub fn command(
        program: &str,
        args: &[String],
        exit_code: Option<i32>,
        duration_ms: u64,
        timed_out: bool,
    ) -> Self {
        Self {
            kind: "command",
            program: Some(program.to_string()),
            args: Some(args.to_vec()),
            exit_code,
            duration_ms: Some(duration_ms),
            timed_out: Some(timed_out),
            truncated: None,
            name: None,
            value: None,
            path: None,
        }
    }

    pub fn fact(name: &str, value: serde_json::Value) -> Self {
        Self {
            kind: "fact",
            program: None,
            args: None,
            exit_code: None,
            duration_ms: None,
            timed_out: None,
            truncated: None,
            name: Some(name.to_string()),
            value: Some(value),
            path: None,
        }
    }

    pub fn file(path: &str, note: &str) -> Self {
        Self {
            kind: "file",
            program: None,
            args: None,
            exit_code: None,
            duration_ms: None,
            timed_out: None,
            truncated: None,
            name: Some(note.to_string()),
            value: None,
            path: Some(path.to_string()),
        }
    }
}

/// A suggested follow-up command.
#[derive(Debug, Clone, Serialize)]
pub struct NextAction {
    pub command: String,
    pub reason: String,
}

impl NextAction {
    pub fn new(command: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            reason: reason.into(),
        }
    }
}

/// The in-progress result of one operation.
#[derive(Debug, Clone)]
pub struct Builder {
    pub operation: String,
    pub status: Status,
    pub summary: String,
    pub data: serde_json::Value,
    pub evidence: Vec<Evidence>,
    pub warnings: Vec<String>,
    pub next_actions: Vec<NextAction>,
    pub truncated: bool,
    /// Lines for concise human rendering (default mode).
    pub human: Vec<String>,
}

impl Builder {
    pub fn new(operation: &str, status: Status, summary: impl Into<String>) -> Self {
        Self {
            operation: operation.to_string(),
            status,
            summary: summary.into(),
            data: serde_json::Value::Object(serde_json::Map::new()),
            evidence: Vec::new(),
            warnings: Vec::new(),
            next_actions: Vec::new(),
            truncated: false,
            human: Vec::new(),
        }
    }

    pub fn ok(operation: &str, summary: impl Into<String>) -> Self {
        Self::new(operation, Status::Ok, summary)
    }

    pub fn no_results(operation: &str, summary: impl Into<String>) -> Self {
        Self::new(operation, Status::NoResults, summary)
    }

    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }

    pub fn evidence(mut self, e: Evidence) -> Self {
        self.evidence.push(e);
        self
    }

    pub fn warning(mut self, w: impl Into<String>) -> Self {
        self.warnings.push(w.into());
        self
    }

    pub fn next(mut self, command: impl Into<String>, reason: impl Into<String>) -> Self {
        self.next_actions.push(NextAction::new(command, reason));
        self
    }

    pub fn truncated(mut self, t: bool) -> Self {
        self.truncated = t;
        self
    }

    pub fn line(mut self, l: impl Into<String>) -> Self {
        self.human.push(l.into());
        self
    }

    pub fn lines(mut self, ls: impl IntoIterator<Item = String>) -> Self {
        self.human.extend(ls);
        self
    }

    pub fn envelope(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "tool": TOOL_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "operation": self.operation,
            "ok": self.status.is_ok(),
            "status": self.status,
            "summary": self.summary,
            "data": self.data,
            "evidence": self.evidence,
            "warnings": self.warnings,
            "next_actions": self.next_actions,
            "truncated": self.truncated,
        })
    }
}

/// A structured failure. Rendered as an error envelope in JSON mode and as a
/// concise message + hints on stderr in human mode.
#[derive(Debug, Clone)]
pub struct Failure {
    pub operation: String,
    pub status: Status,
    pub message: String,
    pub hints: Vec<String>,
}

impl Failure {
    pub fn new(operation: &str, status: Status, message: impl Into<String>) -> Self {
        Self {
            operation: operation.to_string(),
            status,
            message: message.into(),
            hints: Vec::new(),
        }
    }

    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hints.push(h.into());
        self
    }

    pub fn invalid(operation: &str, message: impl Into<String>) -> Self {
        Self::new(operation, Status::InvalidRequest, message)
    }

    pub fn unavailable(operation: &str, message: impl Into<String>) -> Self {
        Self::new(operation, Status::CapabilityUnavailable, message)
    }

    pub fn failed(operation: &str, message: impl Into<String>) -> Self {
        Self::new(operation, Status::Failed, message)
    }

    pub fn envelope(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "tool": TOOL_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "operation": self.operation,
            "ok": false,
            "status": self.status,
            "summary": self.message,
            "data": { "error": self.message },
            "evidence": [],
            "warnings": [],
            "next_actions": self.hints.iter().map(|h| serde_json::json!({"command": "", "reason": h})).collect::<Vec<_>>(),
            "truncated": false,
        })
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Failure {}

pub type CmdResult = Result<Builder, Failure>;
