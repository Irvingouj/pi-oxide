//! Append-only session event log.
//!
//! Writes one JSON line per event to `~/.pi-oxide/sessions/{id}/events.jsonl`.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    TurnStart {
        turn: u32,
    },
    LlmRequest {
        turn: u32,
        model: String,
        message_count: usize,
    },
    LlmResponse {
        turn: u32,
        stop_reason: String,
    },
    ToolCall {
        turn: u32,
        tool_call_id: String,
        tool_name: String,
    },
    ToolResult {
        turn: u32,
        tool_call_id: String,
        truncated: bool,
    },
    Error {
        turn: u32,
        message: String,
    },
    TurnEnd {
        turn: u32,
    },
}

#[derive(Debug, Serialize)]
struct JsonlLine<'a> {
    ts: String,
    #[serde(flatten)]
    event: &'a SessionEvent,
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// Lightweight append-only JSONL writer.
///
/// Keeps a single `File` handle open and flushes on each write.
/// The `Mutex` allows shared access without worrying about lifetimes.
pub struct SessionEventLogger {
    file: Mutex<io::BufWriter<File>>,
}

impl SessionEventLogger {
    /// Create a logger for the given session ID.
    ///
    /// Creates the parent directory if needed. Returns `Err` only if the file
    /// cannot be opened.
    pub fn new(session_id: &str) -> io::Result<Self> {
        let path = session_log_path(session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            file: Mutex::new(io::BufWriter::new(file)),
        })
    }

    /// Append a single event line. Flushes on each call.
    pub fn append(&self, event: &SessionEvent) -> io::Result<()> {
        let line = serde_json::to_string(&JsonlLine {
            ts: iso8601_now(),
            event,
        })?;
        let mut guard = self.file.lock().unwrap();
        writeln!(guard, "{line}")?;
        guard.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

fn session_log_path(session_id: &str) -> PathBuf {
    home_dir()
        .join(".pi-oxide")
        .join("sessions")
        .join(session_id)
        .join("events.jsonl")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Timestamp
// ---------------------------------------------------------------------------

/// Format the current UTC time as an ISO 8601 string (e.g. `2025-07-06T01:23:45.123Z`).
fn iso8601_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S.%3fZ").to_string()
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_format() {
        let ts = iso8601_now();
        // Should match YYYY-MM-DDTHH:MM:SS.mmmZ (24 chars)
        assert!(
            ts.len() == 24,
            "expected 24 chars, got {}: {}",
            ts.len(),
            ts
        );
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
    }

    #[test]
    fn event_serialization() {
        let event = SessionEvent::TurnStart { turn: 1 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"turn_start\""));
        assert!(json.contains("\"turn\":1"));

        let event = SessionEvent::LlmRequest {
            turn: 2,
            model: "claude-sonnet-4-20250514".into(),
            message_count: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"llm_request\""));
        assert!(json.contains("\"message_count\":5"));
    }

    #[test]
    fn logger_append_to_temp() {
        let tmp_dir = std::env::temp_dir().join(format!(
            "pi-oxide-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let session_id = tmp_dir.to_string_lossy().to_string();

        // Manually create parent for the test
        if let Some(parent) = session_log_path(&session_id).parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let logger = SessionEventLogger::new(&session_id).unwrap();
        logger.append(&SessionEvent::TurnStart { turn: 1 }).unwrap();
        logger.append(&SessionEvent::TurnEnd { turn: 1 }).unwrap();

        let path = session_log_path(&session_id);
        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"turn_start\""));
        assert!(lines[1].contains("\"turn_end\""));

        // Cleanup
        std::fs::remove_dir_all(&tmp_dir).ok();
    }
}
