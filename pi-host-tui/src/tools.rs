//! Tool definitions and execution for the terminal agent.
//!
//! Four tools: bash, read, write, edit — the minimum viable coding agent.

use std::path::{Path, PathBuf};

use pi_core::{
    ExecutionMode, JsonSchema, ToolArguments, ToolCall, ToolDefinition, ToolError, ToolName,
    ToolRunMode,
};

// --- Definitions ---

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: ToolName::new("read"),
            label: "Read File".into(),
            description: "Read a file's contents. Use offset/limit for large files.".into(),
            parameters: JsonSchema::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to read"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start from (0-indexed)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max lines to read"
                    }
                },
                "required": ["path"]
            })),
            execution_mode: ExecutionMode::Parallel,
            tool_run_mode: ToolRunMode::Immediate,
        },
        ToolDefinition {
            name: ToolName::new("write"),
            label: "Write File".into(),
            description: "Write content to a file. Creates parent directories.".into(),
            parameters: JsonSchema::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write"
                    }
                },
                "required": ["path", "content"]
            })),
            execution_mode: ExecutionMode::Sequential,
            tool_run_mode: ToolRunMode::Immediate,
        },
        ToolDefinition {
            name: ToolName::new("edit"),
            label: "Edit File".into(),
            description:
                "Replace exact text in a file. Finds old_string and replaces with new_string."
                    .into(),
            parameters: JsonSchema::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact text to find"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Text to replace with"
                    }
                },
                "required": ["path", "old_string", "new_string"]
            })),
            execution_mode: ExecutionMode::Sequential,
            tool_run_mode: ToolRunMode::Immediate,
        },
    ]
}

// --- Execution ---

fn resolve_path(path_str: &str, cwd: &Path) -> PathBuf {
    let path = Path::new(path_str);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

pub fn execute(call: &ToolCall, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    match call.name.as_str() {
        "bash" => exec_bash(&call.arguments, cwd),
        "read" => exec_read(&call.arguments, cwd),
        "write" => exec_write(&call.arguments, cwd),
        "edit" => exec_edit(&call.arguments, cwd),
        name => Err(ToolError::new(
            "unknown_tool",
            format!("Unknown tool: {name}"),
        )),
    }
}

fn get_str<'a>(args: &'a ToolArguments, key: &str) -> Result<&'a str, ToolError> {
    args.0
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::new("missing_param", format!("Missing parameter: {key}")))
}

fn exec_bash(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let command = get_str(args, "command")?;

    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|e| ToolError::new("exec_failed", e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let text = if output.status.success() {
        stdout.to_string()
    } else {
        format!(
            "exit code: {}\n{}{}",
            output.status.code().unwrap_or(-1),
            stderr,
            if stdout.is_empty() {
                String::new()
            } else {
                format!("\n{}", stdout)
            }
        )
    };

    Ok(pi_core::ToolResult::text(text))
}

fn exec_read(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let path = get_str(args, "path")?;
    let resolved = resolve_path(path, cwd);
    let resolved_str = resolved.to_str().unwrap_or(path);
    let offset = args.0.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = args
        .0
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|l| l as usize);

    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| ToolError::new("read_failed", format!("{resolved_str}: {e}")))?;

    let lines: Vec<&str> = content.lines().collect();
    let start = offset.min(lines.len());
    let end = match limit {
        Some(l) => (start + l).min(lines.len()),
        None => lines.len(),
    };

    let selected: Vec<&str> = lines[start..end].to_vec();
    let result = selected.join("\n");

    Ok(pi_core::ToolResult::text(result))
}

fn exec_write(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let path = get_str(args, "path")?;
    let resolved = resolve_path(path, cwd);
    let resolved_str = resolved.to_str().unwrap_or(path);
    let content = get_str(args, "content")?;

    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ToolError::new("mkdir_failed", format!("{resolved_str}: {e}")))?;
    }

    std::fs::write(&resolved, content)
        .map_err(|e| ToolError::new("write_failed", format!("{resolved_str}: {e}")))?;

    Ok(pi_core::ToolResult::text(format!(
        "Wrote {} bytes to {path}",
        content.len()
    )))
}

fn exec_edit(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let path = get_str(args, "path")?;
    let resolved = resolve_path(path, cwd);
    let resolved_str = resolved.to_str().unwrap_or(path);
    let old_string = get_str(args, "old_string")?;
    let new_string = get_str(args, "new_string")?;

    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| ToolError::new("read_failed", format!("{resolved_str}: {e}")))?;

    let count = content.matches(old_string).count();
    if count == 0 {
        return Err(ToolError::new(
            "not_found",
            format!("old_string not found in {resolved_str}"),
        ));
    }
    if count > 1 {
        return Err(ToolError::new(
            "ambiguous",
            format!(
                "old_string found {count} times in {resolved_str}. Provide more context to make it unique."
            ),
        ));
    }

    let new_content = content.replacen(old_string, new_string, 1);
    std::fs::write(&resolved, &new_content)
        .map_err(|e| ToolError::new("write_failed", format!("{resolved_str}: {e}")))?;

    Ok(pi_core::ToolResult::text(format!("Edited {resolved_str}")))
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::{Content, ToolCallId};

    fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::new("test-id"),
            name: ToolName::new(name),
            arguments: ToolArguments::new(args),
        }
    }

    #[test]
    fn test_bash_echo() {
        let call = make_call("bash", serde_json::json!({"command": "echo hello world"}));
        let result = execute(&call, Path::new(".")).unwrap();
        let text = result
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text(t) = c {
                    Some(t.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("hello world"));
    }

    #[test]
    fn test_bash_failure() {
        let call = make_call("bash", serde_json::json!({"command": "exit 42"}));
        let result = execute(&call, Path::new(".")).unwrap();
        let text = result
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text(t) = c {
                    Some(t.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("exit code: 42"));
    }

    #[test]
    fn test_read_write() {
        let dir = std::env::temp_dir().join("pi-test-tools");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.txt");
        let path_str = path.to_str().unwrap();

        // Write
        let write_call = make_call(
            "write",
            serde_json::json!({
                "path": path_str,
                "content": "line 1\nline 2\nline 3\n"
            }),
        );
        execute(&write_call, Path::new(".")).unwrap();

        // Read all
        let read_call = make_call("read", serde_json::json!({"path": path_str}));
        let result = execute(&read_call, Path::new(".")).unwrap();
        let text = result
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text(t) = c {
                    Some(t.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("line 1"));
        assert!(text.contains("line 3"));

        // Read with offset/limit
        let read_partial = make_call(
            "read",
            serde_json::json!({
                "path": path_str,
                "offset": 1,
                "limit": 1
            }),
        );
        let result = execute(&read_partial, Path::new(".")).unwrap();
        let text = result
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text(t) = c {
                    Some(t.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("line 2"));
        assert!(!text.contains("line 1"));

        // Cleanup
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_edit() {
        let dir = std::env::temp_dir().join("pi-test-edit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("edit.txt");
        std::fs::write(&path, "hello world\nfoo bar\n").unwrap();
        let path_str = path.to_str().unwrap();

        let edit_call = make_call(
            "edit",
            serde_json::json!({
                "path": path_str,
                "old_string": "hello world",
                "new_string": "hello rust"
            }),
        );
        execute(&edit_call, Path::new(".")).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("hello rust"));
        assert!(!content.contains("hello world"));
        assert!(content.contains("foo bar"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_edit_not_found() {
        let dir = std::env::temp_dir().join("pi-test-edit-nf");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nf.txt");
        std::fs::write(&path, "abc\n").unwrap();

        let call = make_call(
            "edit",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "old_string": "xyz",
                "new_string": "123"
            }),
        );
        let err = execute(&call, Path::new(".")).unwrap_err();
        assert_eq!(err.code, "not_found");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_edit_ambiguous() {
        let dir = std::env::temp_dir().join("pi-test-edit-amb");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("amb.txt");
        std::fs::write(&path, "abc abc\n").unwrap();

        let call = make_call(
            "edit",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "old_string": "abc",
                "new_string": "xyz"
            }),
        );
        let err = execute(&call, Path::new(".")).unwrap_err();
        assert_eq!(err.code, "ambiguous");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_unknown_tool() {
        let call = make_call("fly", serde_json::json!({}));
        let err = execute(&call, Path::new(".")).unwrap_err();
        assert_eq!(err.code, "unknown_tool");
    }
}
