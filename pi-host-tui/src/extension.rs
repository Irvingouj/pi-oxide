//! Extension trait and built-in extensions for the terminal agent.
//!
//! Extensions decide execution strategy (sync or async/streaming) per tool call.
//! The host polls `ToolEventStream::try_recv()` each frame to receive updates
//! and completion events without blocking the UI.

use pi_core::{ToolCall, ToolDefinition, ToolError, ToolExecutionUpdate, ToolResult};

pub trait Extension: Send + Sync {
    fn name(&self) -> &str;
    fn tools(&self) -> Vec<ToolDefinition>;
    fn execute(&self, call: &ToolCall, ctx: &ExtensionContext) -> ExtensionOutcome;
}

pub struct ExtensionContext {
    pub cwd: std::path::PathBuf,
}

pub enum ExtensionOutcome {
    Complete(Result<ToolResult, ToolError>),
    Running(Box<dyn ToolEventStream>),
}

pub trait ToolEventStream: Send {
    fn try_recv(&mut self) -> Option<ToolEvent>;
}

pub enum ToolEvent {
    Update(ToolExecutionUpdate),
    Done(Result<ToolResult, ToolError>),
}

impl ToolEventStream for std::sync::mpsc::Receiver<ToolEvent> {
    fn try_recv(&mut self) -> Option<ToolEvent> {
        std::sync::mpsc::Receiver::try_recv(self).ok()
    }
}

// ---------------------------------------------------------------------------
// BuiltinExtension — read, write, edit (sync)
// ---------------------------------------------------------------------------

pub struct BuiltinExtension;

impl BuiltinExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BuiltinExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl Extension for BuiltinExtension {
    fn name(&self) -> &str {
        "builtin"
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        crate::tools::definitions()
    }

    fn execute(&self, call: &ToolCall, ctx: &ExtensionContext) -> ExtensionOutcome {
        ExtensionOutcome::Complete(crate::tools::execute(call, &ctx.cwd))
    }
}

// ---------------------------------------------------------------------------
// BashExtension — bash (async, runs in a background thread)
// ---------------------------------------------------------------------------

pub struct BashExtension;

impl BashExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BashExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl Extension for BashExtension {
    fn name(&self) -> &str {
        "bash"
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: pi_core::ToolName::new("bash"),
            label: "Bash".into(),
            description: "Run a bash command. Returns stdout and stderr.".into(),
            parameters: pi_core::JsonSchema::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to run"
                    }
                },
                "required": ["command"]
            })),
            execution_mode: pi_core::ExecutionMode::Sequential,
        }]
    }

    fn execute(&self, call: &ToolCall, ctx: &ExtensionContext) -> ExtensionOutcome {
        let command = call
            .arguments
            .0
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cwd = ctx.cwd.clone();
        let tool_call_id = call.id.clone();

        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let mut child = match std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(ToolEvent::Done(Err(ToolError::new(
                        "exec_failed",
                        e.to_string(),
                    ))));
                    return;
                }
            };

            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            let mut seq = 0u64;

            // Stream stdout
            let mut reader = std::io::BufReader::new(stdout);
            let mut buf = [0u8; 1024];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        let update = ToolExecutionUpdate {
                            tool_call_id: tool_call_id.clone(),
                            stream: pi_core::ToolOutputStream::Stdout,
                            chunk,
                            sequence: seq,
                            timestamp: pi_core::timestamp::current_timestamp(),
                        };
                        seq += 1;
                        if tx.send(ToolEvent::Update(update)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }

            // Stream stderr
            let mut reader = std::io::BufReader::new(stderr);
            let mut buf = [0u8; 1024];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        let update = ToolExecutionUpdate {
                            tool_call_id: tool_call_id.clone(),
                            stream: pi_core::ToolOutputStream::Stderr,
                            chunk,
                            sequence: seq,
                            timestamp: pi_core::timestamp::current_timestamp(),
                        };
                        seq += 1;
                        if tx.send(ToolEvent::Update(update)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }

            // Wait for exit and send final result
            let result = match child.wait() {
                Ok(status) => {
                    let text = if status.success() {
                        String::new()
                    } else {
                        format!("exit code: {}", status.code().unwrap_or(-1))
                    };
                    Ok(ToolResult::text(text))
                }
                Err(e) => Err(ToolError::new("wait_failed", e.to_string())),
            };
            let _ = tx.send(ToolEvent::Done(result));
        });

        ExtensionOutcome::Running(Box::new(rx))
    }
}
