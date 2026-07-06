//! Tool definitions and execution for the terminal agent.
//!
//! Four tools: bash, read, write, edit — the minimum viable coding agent.

use std::io::Read;
use std::path::{Path, PathBuf};

use pi_core::{
    ExecutionMode, JsonSchema, ToolArguments, ToolCall, ToolDefinition, ToolError, ToolName,
    ToolRunMode,
};
use tracing::{debug, error};

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
        ToolDefinition {
            name: ToolName::new("grep"),
            label: "Grep Files".into(),
            description: "Search files using regex patterns. Respects .gitignore.".into(),
            parameters: JsonSchema::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for"
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Files, directories, or globs to search. Defaults to workspace root if omitted."
                    }
                },
                "required": ["pattern"]
            })),
            execution_mode: ExecutionMode::Parallel,
            tool_run_mode: ToolRunMode::Immediate,
        },
        ToolDefinition {
            name: ToolName::new("glob"),
            label: "Glob Files".into(),
            description:
                "Find files matching glob patterns (e.g. src/**/*.ts). Respects .gitignore.".into(),
            parameters: JsonSchema::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Glob patterns to match"
                    }
                },
                "required": ["paths"]
            })),
            execution_mode: ExecutionMode::Parallel,
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
        "grep" => exec_grep(&call.arguments, cwd),
        "glob" => exec_glob(&call.arguments, cwd),
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

    debug!(tool = "read", path = %resolved_str, "tool started");

    let content = std::fs::read_to_string(&resolved).map_err(|e| {
        error!(tool = "read", path = %resolved_str, error = ?e, "tool failed");
        ToolError::new("read_failed", format!("{resolved_str}: {e}"))
    })?;

    let lines: Vec<&str> = content.lines().collect();
    let start = offset.min(lines.len());
    let end = match limit {
        Some(l) => (start + l).min(lines.len()),
        None => lines.len(),
    };

    let selected: Vec<&str> = lines[start..end].to_vec();
    let result = selected.join("\n");

    debug!(tool = "read", path = %resolved_str, lines = selected.len(), "tool completed");
    Ok(pi_core::ToolResult::text(result))
}

fn exec_write(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let path = get_str(args, "path")?;
    let resolved = resolve_path(path, cwd);
    let resolved_str = resolved.to_str().unwrap_or(path);
    let content = get_str(args, "content")?;

    debug!(tool = "write", path = %resolved_str, "tool started");

    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!(tool = "write", path = %resolved_str, error = ?e, "tool failed");
            ToolError::new("mkdir_failed", format!("{resolved_str}: {e}"))
        })?;
    }

    std::fs::write(&resolved, content).map_err(|e| {
        error!(tool = "write", path = %resolved_str, error = ?e, "tool failed");
        ToolError::new("write_failed", format!("{resolved_str}: {e}"))
    })?;

    debug!(tool = "write", path = %resolved_str, bytes = content.len(), "tool completed");
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

    debug!(tool = "edit", path = %resolved_str, "tool started");

    let content = std::fs::read_to_string(&resolved).map_err(|e| {
        error!(tool = "edit", path = %resolved_str, error = ?e, "tool failed");
        ToolError::new("read_failed", format!("{resolved_str}: {e}"))
    })?;

    let count = content.matches(old_string).count();
    if count == 0 {
        error!(tool = "edit", path = %resolved_str, "tool failed: old_string not found");
        return Err(ToolError::new(
            "not_found",
            format!("old_string not found in {resolved_str}"),
        ));
    }
    if count > 1 {
        error!(tool = "edit", path = %resolved_str, matches = count, "tool failed: ambiguous match");
        return Err(ToolError::new(
            "ambiguous",
            format!(
                "old_string found {count} times in {resolved_str}. Provide more context to make it unique."
            ),
        ));
    }

    let new_content = content.replacen(old_string, new_string, 1);
    std::fs::write(&resolved, &new_content).map_err(|e| {
        error!(tool = "edit", path = %resolved_str, error = ?e, "tool failed");
        ToolError::new("write_failed", format!("{resolved_str}: {e}"))
    })?;

    debug!(tool = "edit", path = %resolved_str, "tool completed");
    Ok(pi_core::ToolResult::text(format!("Edited {resolved_str}")))
}

fn exec_grep(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let pattern = get_str(args, "pattern")?;
    let re = regex::Regex::new(pattern).map_err(|e| {
        error!(tool = "grep", pattern = %pattern, error = ?e, "tool failed");
        ToolError::new("invalid_regex", format!("Invalid regex: {e}"))
    })?;

    debug!(tool = "grep", pattern = %pattern, "tool started");

    let paths: Vec<String> = args
        .0
        .get("paths")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec![".".to_string()]);

    let mut results: Vec<(String, u64, String)> = Vec::new();

    for path_str in &paths {
        let path = resolve_path(path_str, cwd);

        if path.is_dir() {
            let walker = ignore::WalkBuilder::new(&path)
                .standard_filters(true)
                .hidden(false)
                .build();

            for entry in walker {
                let entry = entry.map_err(|e| ToolError::new("walk_error", format!("{e}")))?;
                let file_path = entry.path();

                if !file_path.is_file() {
                    continue;
                }

                if is_binary(file_path) {
                    continue;
                }

                match std::fs::read_to_string(file_path) {
                    Ok(content) => {
                        for (line_idx, line) in content.lines().enumerate() {
                            if re.is_match(line) {
                                let rel_path = file_path
                                    .strip_prefix(cwd)
                                    .unwrap_or(file_path)
                                    .to_string_lossy()
                                    .to_string();
                                results.push((rel_path, (line_idx + 1) as u64, line.to_string()));
                            }
                        }
                    }
                    Err(_) => { /* skip unreadable files */ }
                }
            }
        } else if path.is_file() {
            if is_binary(&path) {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let rel_path = path
                        .strip_prefix(cwd)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();
                    for (line_idx, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            results.push((
                                rel_path.clone(),
                                (line_idx + 1) as u64,
                                line.to_string(),
                            ));
                        }
                    }
                }
                Err(_) => { /* skip unreadable files */ }
            }
        }
    }

    let mut output = String::new();
    let mut current_file: Option<String> = None;

    for (file, line_num, line) in &results {
        if current_file.as_deref() != Some(file.as_str()) {
            current_file = Some(file.clone());
            output.push_str(&format!("[{file}]\n"));
        }
        output.push_str(&format!("*{line_num}:{line}\n"));
    }

    if results.is_empty() {
        debug!(tool = "grep", pattern = %pattern, "tool completed: no matches");
        return Ok(pi_core::ToolResult::text("No matches found."));
    }

    debug!(tool = "grep", pattern = %pattern, matches = results.len(), "tool completed");
    let file_count = results
        .iter()
        .map(|(f, _, _)| f)
        .collect::<std::collections::HashSet<_>>()
        .len();
    Ok(pi_core::ToolResult::text(format!(
        "Found {} match{} in {} file{}:\n{}",
        results.len(),
        if results.len() == 1 { "" } else { "es" },
        file_count,
        if file_count == 1 { "" } else { "s" },
        output
    )))
}

fn exec_glob(args: &ToolArguments, cwd: &Path) -> Result<pi_core::ToolResult, ToolError> {
    let patterns: Vec<String> = args
        .0
        .get("paths")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .ok_or_else(|| {
            error!(tool = "glob", "tool failed: missing paths parameter");
            ToolError::new("missing_param", "Missing parameter: paths")
        })?;

    if patterns.is_empty() {
        error!(tool = "glob", "tool failed: paths array is empty");
        return Err(ToolError::new(
            "invalid_param",
            "paths array must not be empty",
        ));
    }

    debug!(
        tool = "glob",
        patterns_count = patterns.len(),
        "tool started"
    );

    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &patterns {
        let g = globset::Glob::new(pattern).map_err(|e| {
            error!(tool = "glob", pattern = %pattern, error = ?e, "tool failed");
            ToolError::new("invalid_glob", format!("Invalid glob pattern: {e}"))
        })?;
        builder.add(g);
    }
    let glob_set = builder.build().map_err(|e| {
        error!(tool = "glob", error = ?e, "tool failed");
        ToolError::new("glob_build_error", format!("{e}"))
    })?;

    let mut results: Vec<String> = Vec::new();

    let walker = ignore::WalkBuilder::new(cwd)
        .standard_filters(true)
        .hidden(false)
        .build();

    for entry in walker {
        let entry = entry.map_err(|e| ToolError::new("walk_error", format!("{e}")))?;
        let path = entry.path();

        let rel_path = path
            .strip_prefix(cwd)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        if glob_set.is_match(&rel_path) {
            results.push(rel_path);
        }
    }

    results.sort();
    results.dedup();

    if results.is_empty() {
        debug!(tool = "glob", "tool completed: no matches");
        return Ok(pi_core::ToolResult::text("No matches found."));
    }

    debug!(tool = "glob", matches = results.len(), "tool completed");
    Ok(pi_core::ToolResult::text(format!(
        "Found {} match{}:\n{}",
        results.len(),
        if results.len() == 1 { "" } else { "es" },
        results.join("\n")
    )))
}

fn is_binary(path: &Path) -> bool {
    // Only read first 8KB to detect binary - avoid loading whole file into memory
    match std::fs::File::open(path) {
        Ok(f) => {
            let mut reader = std::io::BufReader::with_capacity(8192, f);
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(n) => buf[..n].contains(&0),
                Err(_) => true,
            }
        }
        Err(_) => true,
    }
}
