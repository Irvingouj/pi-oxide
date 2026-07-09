//! Tests for tools module.

use std::path::Path;

use pi_core::{Content, ToolArguments, ToolCall, ToolCallId, ToolName};

use crate::tools::execute;

fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("test-id"),
        name: ToolName::new(name),
        arguments: ToolArguments::new(args),
    }
}

fn extract_text(result: pi_core::ToolResult) -> String {
    result
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
        .join("")
}

// --- bash ---

#[test]
fn test_bash_echo() {
    let call = make_call("bash", serde_json::json!({"command": "echo hello world"}));
    let result = execute(&call, Path::new(".")).unwrap();
    assert!(extract_text(result).contains("hello world"));
}

#[test]
fn test_bash_failure() {
    let call = make_call("bash", serde_json::json!({"command": "exit 42"}));
    let result = execute(&call, Path::new(".")).unwrap();
    assert!(extract_text(result).contains("exit code: 42"));
}

// --- read/write ---

#[test]
fn test_read_write() {
    let dir = std::env::temp_dir().join("pi-test-tools");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.txt");
    let path_str = path.to_str().unwrap();

    let write_call = make_call(
        "write",
        serde_json::json!({
            "path": path_str,
            "content": "line 1\nline 2\nline 3\n"
        }),
    );
    execute(&write_call, Path::new(".")).unwrap();

    let read_call = make_call("read", serde_json::json!({"path": path_str}));
    let result = execute(&read_call, Path::new(".")).unwrap();
    let text = extract_text(result);
    assert!(text.contains("line 1"));
    assert!(text.contains("line 3"));

    let read_partial = make_call(
        "read",
        serde_json::json!({
            "path": path_str,
            "offset": 1,
            "limit": 1
        }),
    );
    let result = execute(&read_partial, Path::new(".")).unwrap();
    let text = extract_text(result);
    assert!(text.contains("line 2"));
    assert!(!text.contains("line 1"));

    std::fs::remove_dir_all(&dir).ok();
}

// --- edit ---

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

// --- unknown tool ---

#[test]
fn test_unknown_tool() {
    let call = make_call("fly", serde_json::json!({}));
    let err = execute(&call, Path::new(".")).unwrap_err();
    assert_eq!(err.code, "unknown_tool");
}

// --- grep ---

#[test]
fn test_grep_basic() {
    let dir = std::env::temp_dir().join("pi-test-grep");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.rs");
    std::fs::write(&path, "fn hello() {}\nfn world() {}\nfn hello_again() {}\n").unwrap();

    let call = make_call(
        "grep",
        serde_json::json!({
            "pattern": "hello",
            "paths": [path.to_str().unwrap()]
        }),
    );
    let result = execute(&call, &dir).unwrap();
    let text = extract_text(result);
    assert!(text.contains("hello"));
    assert!(text.contains("hello_again"));
    assert!(!text.contains("world"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_grep_no_match() {
    let dir = std::env::temp_dir().join("pi-test-grep-nm");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.txt");
    std::fs::write(&path, "no matches here\n").unwrap();

    let call = make_call(
        "grep",
        serde_json::json!({
            "pattern": "xyz123",
            "paths": [path.to_str().unwrap()]
        }),
    );
    let result = execute(&call, &dir).unwrap();
    assert!(extract_text(result).contains("No matches"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_grep_with_gitignore_e2e() {
    let dir = std::env::temp_dir().join("pi-test-grep-git-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join(".gitignore"), "*.log\nnode_modules/\n").unwrap();

    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\nfn helper() {}\n").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "pub fn lib_func() {}\n").unwrap();
    std::fs::write(dir.join("app.log"), "fn main() {}\nfn helper() {}\n").unwrap();
    std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    std::fs::write(dir.join("node_modules/pkg/index.js"), "fn main() {}\n").unwrap();

    let call = make_call(
        "grep",
        serde_json::json!({
            "pattern": "fn main"
        }),
    );
    let result = execute(&call, &dir).unwrap();
    let text = extract_text(result);

    assert!(text.contains("src/main.rs"), "should find in src/main.rs");
    assert!(!text.contains("app.log"), "should ignore *.log files");
    assert!(
        !text.contains("node_modules"),
        "should ignore node_modules/"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_grep_directory_walk_e2e() {
    let dir = std::env::temp_dir().join("pi-test-grep-dir-e2e");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::create_dir_all(dir.join("src/utils")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(
        dir.join("src/utils/helper.rs"),
        "fn main() {}\npub fn help() {}\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(
        dir.join("tests/integration.rs"),
        "fn main() {}\n#[test]\nfn test_it() {}\n",
    )
    .unwrap();

    let call = make_call(
        "grep",
        serde_json::json!({
            "pattern": "fn main"
        }),
    );
    let result = execute(&call, &dir).unwrap();
    let text = extract_text(result);

    assert!(text.contains("src/main.rs"));
    assert!(text.contains("src/utils/helper.rs"));
    assert!(text.contains("tests/integration.rs"));

    std::fs::remove_dir_all(&dir).ok();
}

// --- glob ---

#[test]
fn test_glob_basic() {
    let dir = std::env::temp_dir().join("pi-test-glob");
    std::fs::create_dir_all(&dir).unwrap();
    let sub = dir.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("main.rs"), "").unwrap();
    std::fs::write(sub.join("lib.rs"), "").unwrap();
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();

    let call = make_call(
        "glob",
        serde_json::json!({
            "paths": ["src/*.rs"]
        }),
    );
    let result = execute(&call, &dir).unwrap();
    let text = extract_text(result);
    assert!(text.contains("src/main.rs"));
    assert!(text.contains("src/lib.rs"));
    assert!(!text.contains("Cargo.toml"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_glob_with_gitignore_e2e() {
    let dir = std::env::temp_dir().join("pi-test-glob-git-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join(".gitignore"), "*.log\ntarget/\n").unwrap();

    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "").unwrap();
    std::fs::write(dir.join("app.log"), "").unwrap();
    std::fs::create_dir_all(dir.join("target")).unwrap();
    std::fs::write(dir.join("target/debug.bin"), "").unwrap();

    let call = make_call(
        "glob",
        serde_json::json!({
            "paths": ["**/*"]
        }),
    );
    let result = execute(&call, &dir).unwrap();
    let text = extract_text(result);

    assert!(text.contains("src/main.rs"));
    assert!(text.contains("src/lib.rs"));
    assert!(!text.contains("app.log"), "should ignore *.log files");
    assert!(!text.contains("target/"), "should ignore target/");

    std::fs::remove_dir_all(&dir).ok();
}
