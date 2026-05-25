//! Extension trait and built-in extensions for the terminal agent.
//!
//! Extensions decide execution strategy (sync or async/streaming) per tool call.
//! The host polls `ToolEventStream::try_recv()` each frame to receive updates
//! and completion events without blocking the UI.

use pi_core::{ToolCall, ToolDefinition, ToolError, ToolExecutionUpdate, ToolResult, ToolRunMode};

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

impl std::fmt::Debug for ExtensionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionOutcome::Complete(r) => write!(f, "Complete({:?})", r),
            ExtensionOutcome::Running(_) => write!(f, "Running(<stream>)",),
        }
    }
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
            tool_run_mode: ToolRunMode::Deferred,
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
            let seq = std::sync::Arc::new(std::sync::Mutex::new(0u64));
            let stdout_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

            let tx_stdout = tx.clone();
            let seq_stdout = seq.clone();
            let stdout_buf_clone = stdout_buf.clone();
            let tool_call_id_stdout = tool_call_id.clone();
            let stdout_handle = std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(stdout);
                let mut buf = [0u8; 1024];
                loop {
                    match std::io::Read::read(&mut reader, &mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                            stdout_buf_clone.lock().unwrap().push_str(&chunk);
                            let mut seq = seq_stdout.lock().unwrap();
                            let update = ToolExecutionUpdate {
                                tool_call_id: tool_call_id_stdout.clone(),
                                stream: pi_core::ToolOutputStream::Stdout,
                                chunk,
                                sequence: *seq,
                                timestamp: pi_core::timestamp::current_timestamp(),
                            };
                            *seq += 1;
                            if tx_stdout.send(ToolEvent::Update(update)).is_err() {
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            let tx_stderr = tx.clone();
            let seq_stderr = seq.clone();
            let stderr_buf_clone = stderr_buf.clone();
            let tool_call_id_stderr = tool_call_id.clone();
            let stderr_handle = std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(stderr);
                let mut buf = [0u8; 1024];
                loop {
                    match std::io::Read::read(&mut reader, &mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                            stderr_buf_clone.lock().unwrap().push_str(&chunk);
                            let mut seq = seq_stderr.lock().unwrap();
                            let update = ToolExecutionUpdate {
                                tool_call_id: tool_call_id_stderr.clone(),
                                stream: pi_core::ToolOutputStream::Stderr,
                                chunk,
                                sequence: *seq,
                                timestamp: pi_core::timestamp::current_timestamp(),
                            };
                            *seq += 1;
                            if tx_stderr.send(ToolEvent::Update(update)).is_err() {
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            let _ = stdout_handle.join();
            let _ = stderr_handle.join();

            // Wait for exit and build final result from accumulated buffers
            let result = match child.wait() {
                Ok(status) => {
                    let mut text = stdout_buf.lock().unwrap().clone();
                    let stderr_text = stderr_buf.lock().unwrap().clone();
                    if !stderr_text.is_empty() {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&stderr_text);
                    }
                    if !status.success() {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&format!("exit code: {}", status.code().unwrap_or(-1)));
                    }
                    Ok(ToolResult::text(text))
                }
                Err(e) => Err(ToolError::new("wait_failed", e.to_string())),
            };
            let _ = tx.send(ToolEvent::Done(result));
        });

        ExtensionOutcome::Running(Box::new(rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_extension_completes() {
        let call = pi_core::ToolCall {
            id: pi_core::ToolCallId::new("test-id"),
            name: pi_core::ToolName::new("bash"),
            arguments: pi_core::ToolArguments::new(
                serde_json::json!({"command": "sleep 1 && echo hello"}),
            ),
        };
        let ctx = ExtensionContext {
            cwd: std::path::PathBuf::from("/tmp"),
        };
        let ext = BashExtension::new();
        let outcome = ext.execute(&call, &ctx);

        let mut stream = match outcome {
            ExtensionOutcome::Running(s) => s,
            other => panic!("expected Running, got {:?}", other),
        };

        let mut done = false;
        let mut chunks = Vec::new();
        let start = std::time::Instant::now();
        while !done && start.elapsed() < std::time::Duration::from_secs(10) {
            while let Some(event) = stream.try_recv() {
                match event {
                    ToolEvent::Update(u) => chunks.push(u.chunk),
                    ToolEvent::Done(result) => {
                        assert!(result.is_ok(), "bash failed: {:?}", result);
                        done = true;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(done, "bash did not complete within 10s");
        let output = chunks.join("");
        assert!(output.contains("hello"), "expected 'hello' in output, got: {:?}", output);
    }
}
