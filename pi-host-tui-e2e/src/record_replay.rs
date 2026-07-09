//! Integration test: record real LLM session via pi-record-server proxy,
//! replay it, and verify the TUI handles both runs identically.
//!
//! This test exercises the full pipeline:
//!   1. pi-record-server in record mode proxies to real DeepSeek
//!   2. pio (TUI) runs in a PTY, points at the proxy
//!   3. Multi-turn prompts are sent, responses captured
//!   4. Cassette is saved on shutdown
//!   5. pi-record-server in replay mode serves the cassette
//!   6. pio runs again, same prompts, responses compared

use anyhow::{Context, Result};
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::helpers;

// ---------------------------------------------------------------------------
// Record server manager
// ---------------------------------------------------------------------------

struct RecordServer {
    child: Child,
    port: u16,
}

impl RecordServer {
    fn start(port: u16, cassette_path: PathBuf, mode: &str) -> Result<Self> {
        let binary = helpers::record_server_binary();
        assert!(
            binary.exists(),
            "pi-record-server binary not found at {}",
            binary.display()
        );

        let mut cmd = Command::new(&binary);
        cmd.arg(mode)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        match mode {
            "record" => {
                cmd.arg("--output")
                    .arg(cassette_path.to_string_lossy().to_string());
            }
            "replay" => {
                cmd.arg("--cassette")
                    .arg(cassette_path.to_string_lossy().to_string());
            }
            _ => unreachable!(),
        }

        let child = cmd.spawn().context("failed to start pi-record-server")?;

        // Wait for the server to be ready
        let url = format!("http://localhost:{port}/v1/models");
        let client = reqwest::blocking::Client::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                helpers::kill_child(child.id());
                anyhow::bail!("pi-record-server did not start within 5s");
            }
            if client.get(&url).send().is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        Ok(Self { child, port })
    }
}

impl Drop for RecordServer {
    fn drop(&mut self) {
        helpers::kill_child(self.child.id());
    }
}

// ---------------------------------------------------------------------------
// PTY TUI runner
// ---------------------------------------------------------------------------

struct TuiRunner {
    master_fd: RawFd,
    child_pid: libc::c_int,
}

impl TuiRunner {
    /// Spawn pio in a PTY, pointing at the given base_url.
    fn start(port: u16) -> Result<Self> {
        let binary = helpers::pio_binary();
        assert!(
            binary.exists(),
            "pio binary not found at {}",
            binary.display()
        );

        let (master_fd, slave_fd) = helpers::open_pty_pair()?;

        // Collect env before fork
        let env_strings: Vec<String> = std::env::vars_os()
            .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
            .filter(|kv| !kv.starts_with("CARGO_") && !kv.starts_with("RUST_TEST_"))
            .collect();

        let pid = unsafe { libc::fork() };
        match pid {
            -1 => anyhow::bail!("fork failed"),
            0 => {
                unsafe { libc::close(master_fd) };
                unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };

                unsafe {
                    libc::dup2(slave_fd, libc::STDIN_FILENO);
                    libc::dup2(slave_fd, libc::STDOUT_FILENO);
                    libc::dup2(slave_fd, libc::STDERR_FILENO);
                    if slave_fd > 2 {
                        libc::close(slave_fd);
                    }
                }

                let mut env_ptrs: Vec<*const libc::c_char> = env_strings
                    .into_iter()
                    .filter_map(|s| std::ffi::CString::new(s).ok())
                    .map(|s| s.into_raw() as *const libc::c_char)
                    .collect();

                let overrides = vec![
                    format!("PI_BASE_URL=http://localhost:{port}"),
                    "PI_PROVIDER=deepseek".to_string(),
                    "PI_MODEL=deepseek-chat".to_string(),
                    format!(
                        "PI_API_KEY={}",
                        std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()
                    ),
                ];
                for kv in &overrides {
                    if let Ok(c) = std::ffi::CString::new(kv.as_str()) {
                        env_ptrs.push(c.into_raw() as *const libc::c_char);
                    }
                }
                env_ptrs.push(std::ptr::null());

                let prog = std::ffi::CString::new(binary.to_string_lossy().to_string()).unwrap();
                let skip = std::ffi::CString::new("--skip-onboarding").unwrap();

                let argv: [*const libc::c_char; 3] =
                    [prog.as_ptr(), skip.as_ptr(), std::ptr::null()];

                unsafe {
                    libc::execve(argv[0], argv.as_ptr(), env_ptrs.as_ptr());
                    let msg = format!("execve failed: {}\n", std::io::Error::last_os_error());
                    let _ = libc::write(libc::STDERR_FILENO, msg.as_ptr() as *const _, msg.len());
                    libc::_exit(1);
                }
            }
            child_pid => {
                unsafe { libc::close(slave_fd) };
                Ok(Self {
                    master_fd,
                    child_pid,
                })
            }
        }
    }

    fn wait_ready(&mut self) -> String {
        thread::sleep(Duration::from_millis(600));
        let output = helpers::read_pty_timeout(self.master_fd, 300);
        helpers::strip_ansi(&output)
    }

    fn submit_prompt(&mut self, text: &str, timeout_secs: u64) -> String {
        helpers::type_string(self.master_fd, text);
        thread::sleep(Duration::from_millis(100));
        helpers::send_enter(self.master_fd);

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut all_output = Vec::new();
        let mut last_read = Instant::now();

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let chunk = helpers::read_pty_timeout(self.master_fd, 500);
            if chunk.is_empty() {
                if last_read.elapsed() > Duration::from_secs(2) {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            last_read = Instant::now();
            all_output.extend_from_slice(&chunk);

            let s = String::from_utf8_lossy(&all_output);
            if s.contains("Done") || s.contains("[DONE]") {
                thread::sleep(Duration::from_millis(300));
                let final_chunk = helpers::read_pty_timeout(self.master_fd, 300);
                all_output.extend_from_slice(&final_chunk);
                break;
            }
        }

        helpers::strip_ansi(&all_output)
    }

    fn quit(&mut self) {
        helpers::type_string(self.master_fd, "/quit");
        helpers::send_enter(self.master_fd);
        thread::sleep(Duration::from_millis(500));
        for _ in 0..10 {
            if !helpers::is_alive(self.child_pid) {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for TuiRunner {
    fn drop(&mut self) {
        if helpers::is_alive(self.child_pid) {
            helpers::kill_child(self.child_pid as u32);
        }
        unsafe { libc::close(self.master_fd) };
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Extract assistant text from stripped TUI output.
fn extract_assistant_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|line| line.contains('\u{258c}'))
        .map(|line| line.replacen("\u{258c} ", "", 1).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
#[ignore = "requires DEEPSEEK_API_KEY and network — run with `cargo test -- --ignored`"]
fn record_and_replay_single_turn() -> Result<()> {
    let cassette = std::env::temp_dir().join("pi_test_record_replay.cassette.json");
    let _ = std::fs::remove_file(&cassette);
    let port: u16 = 19998;

    if std::env::var("DEEPSEEK_API_KEY")
        .unwrap_or_default()
        .is_empty()
    {
        eprintln!("Skipping test: DEEPSEEK_API_KEY not set");
        return Ok(());
    }

    // Phase 1: Record
    let _record_server = RecordServer::start(port, cassette.clone(), "record")?;
    let mut tui = TuiRunner::start(port)?;
    let initial = tui.wait_ready();
    assert!(
        initial.contains("Ready"),
        "TUI should show Ready; got: {:.200}",
        initial
    );

    let prompt = "What is 2+2? Answer in exactly 3 words.";
    let record_output = tui.submit_prompt(prompt, 30);
    let record_lines = extract_assistant_lines(&record_output);
    assert!(!record_lines.is_empty(), "Should have assistant response");

    tui.quit();
    drop(tui);
    drop(_record_server);
    thread::sleep(Duration::from_secs(1));
    assert!(cassette.exists());

    let cassette_json = std::fs::read_to_string(&cassette)?;
    let cassette_data: serde_json::Value = serde_json::from_str(&cassette_json)?;
    assert_eq!(cassette_data["version"], 2);

    // Phase 2: Replay
    let _replay_server = RecordServer::start(port, cassette.clone(), "replay")?;
    let mut tui2 = TuiRunner::start(port)?;
    tui2.wait_ready();

    let replay_output = tui2.submit_prompt(prompt, 5);
    let replay_lines = extract_assistant_lines(&replay_output);
    assert!(
        !replay_lines.is_empty(),
        "Replay should have assistant response"
    );

    // Phase 3: Compare
    let record_meaningful: Vec<&str> = record_lines
        .iter()
        .map(|s| s.as_str())
        .filter(|s| s.len() > 3)
        .collect();
    let replay_meaningful: Vec<&str> = replay_lines
        .iter()
        .map(|s| s.as_str())
        .filter(|s| s.len() > 3)
        .collect();

    assert!(!record_meaningful.is_empty());
    assert!(!replay_meaningful.is_empty());

    for line in &record_meaningful {
        let found = replay_meaningful
            .iter()
            .any(|r| r.contains(line) || line.contains(r));
        assert!(found, "Record line not found in replay: '{}'", line);
    }

    tui2.quit();
    drop(tui2);
    drop(_replay_server);
    let _ = std::fs::remove_file(&cassette);

    Ok(())
}

#[test]
#[ignore = "requires DEEPSEEK_API_KEY and network — run with `cargo test -- --ignored`"]
fn record_and_replay_multi_turn() -> Result<()> {
    let cassette = std::env::temp_dir().join("pi_test_multi_turn.cassette.json");
    let _ = std::fs::remove_file(&cassette);
    let port: u16 = 19997;

    if std::env::var("DEEPSEEK_API_KEY")
        .unwrap_or_default()
        .is_empty()
    {
        eprintln!("Skipping test: DEEPSEEK_API_KEY not set");
        return Ok(());
    }

    // Phase 1: Record
    let _record_server = RecordServer::start(port, cassette.clone(), "record")?;
    let mut tui = TuiRunner::start(port)?;
    tui.wait_ready();

    let prompt1 = "What is pi-core? Answer in one sentence.";
    let out1 = tui.submit_prompt(prompt1, 30);
    let lines1 = extract_assistant_lines(&out1);
    assert!(!lines1.is_empty());

    let prompt2 = "And what about pi-llm? One sentence.";
    let out2 = tui.submit_prompt(prompt2, 30);
    let lines2 = extract_assistant_lines(&out2);
    assert!(!lines2.is_empty());

    tui.quit();
    drop(tui);
    drop(_record_server);
    thread::sleep(Duration::from_secs(1));
    assert!(cassette.exists());

    // Phase 2: Replay
    let _replay_server = RecordServer::start(port, cassette.clone(), "replay")?;
    let mut tui2 = TuiRunner::start(port)?;
    tui2.wait_ready();

    let r_out1 = tui2.submit_prompt(prompt1, 10);
    let r_lines1 = extract_assistant_lines(&r_out1);
    let r_out2 = tui2.submit_prompt(prompt2, 10);
    let r_lines2 = extract_assistant_lines(&r_out2);

    assert!(!r_lines1.is_empty());
    assert!(!r_lines2.is_empty());

    // Phase 3: Compare
    for (turn_name, record_lines, replay_lines) in [
        ("turn 1", &lines1, &r_lines1),
        ("turn 2", &lines2, &r_lines2),
    ] {
        let record_meaningful: Vec<&str> = record_lines
            .iter()
            .map(|s| s.as_str())
            .filter(|s| s.len() > 3)
            .collect();
        let replay_meaningful: Vec<&str> = replay_lines
            .iter()
            .map(|s| s.as_str())
            .filter(|s| s.len() > 3)
            .collect();
        assert!(
            !record_meaningful.is_empty(),
            "{turn_name}: should have meaningful record output"
        );
        assert!(
            !replay_meaningful.is_empty(),
            "{turn_name}: should have meaningful replay output"
        );
        for line in &record_meaningful {
            let found = replay_meaningful
                .iter()
                .any(|r| r.contains(line) || line.contains(r));
            assert!(
                found,
                "{turn_name}: record line not found in replay: '{line}'"
            );
        }
    }

    tui2.quit();
    drop(tui2);
    drop(_replay_server);
    let _ = std::fs::remove_file(&cassette);

    Ok(())
}
