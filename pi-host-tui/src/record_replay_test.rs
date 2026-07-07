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
use libc::c_int;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

// Reuse the PTY helpers from e2e_tests (same crate, same module scope).
// They're public within the crate since e2e_tests uses `#![allow(dead_code)]`.
// We redeclare the ones we need inline to avoid circular module deps.

// ---------------------------------------------------------------------------
// External binary paths
// ---------------------------------------------------------------------------

fn pio_binary() -> PathBuf {
    static PIO: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut bin = PIO.lock().unwrap();
    if let Some(ref p) = *bin {
        return p.clone();
    }
    let p = if let Ok(path) = std::env::var("CARGO_BIN_EXE_pio") {
        PathBuf::from(path)
    } else {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
        let root = PathBuf::from(&manifest_dir).parent().unwrap().to_path_buf();
        root.join("target/debug/pio")
    };
    *bin = Some(p.clone());
    p
}

fn record_server_binary() -> PathBuf {
    static RS: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut bin = RS.lock().unwrap();
    if let Some(ref p) = *bin {
        return p.clone();
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
    let root = PathBuf::from(&manifest_dir).parent().unwrap().to_path_buf();
    let p = root.join("target/debug/pi-record-server");
    *bin = Some(p.clone());
    p
}

// ---------------------------------------------------------------------------
// PTY helpers (same as e2e_tests, duplicated for module independence)
// ---------------------------------------------------------------------------

use nix::pty::openpty;
use std::ffi::CString;
use std::os::unix::io::IntoRawFd;

fn open_pty() -> Result<(RawFd, RawFd)> {
    let winsize = nix::pty::Winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = openpty(Some(&winsize), None).context("openpty")?;
    Ok((pty.master.into_raw_fd(), pty.slave.into_raw_fd()))
}

fn send_raw(fd: RawFd, data: &[u8]) {
    let res = unsafe {
        libc::write(
            fd,
            data.as_ptr() as *const libc::c_void,
            data.len() as libc::size_t,
        )
    };
    assert!(res >= 0, "write to pty failed");
}

fn send_enter(fd: RawFd) {
    send_raw(fd, b"\r");
}

fn send_char(fd: RawFd, c: char) {
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    send_raw(fd, s.as_bytes());
}

fn type_string(fd: RawFd, s: &str) {
    for c in s.chars() {
        send_char(fd, c);
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(50));
}

fn read_pty_timeout(fd: RawFd, timeout_ms: i32) -> Vec<u8> {
    let mut buf = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

    loop {
        let now = Instant::now();
        let remaining = deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            break;
        }
        let poll_timeout_ms = remaining.as_millis().max(1) as libc::c_int;

        let mut pfd = libc::pollfd {
            fd: fd as libc::c_int,
            events: libc::POLLIN,
            revents: 0,
        };

        let ret = unsafe { libc::poll(&mut pfd, 1, poll_timeout_ms) };
        if ret <= 0 {
            break;
        }
        if pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
            break;
        }

        let mut chunk = [0u8; 8192];
        let res = unsafe {
            libc::read(
                fd,
                chunk.as_mut_ptr() as *mut libc::c_void,
                chunk.len() as libc::size_t,
            )
        };
        if res <= 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..res as usize]);
    }

    buf
}

fn strip_ansi(input: &[u8]) -> String {
    let s = String::from_utf8_lossy(input);
    let re = regex::Regex::new(
        r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b\([A-HP-Z]|\x1b\[7m|\x1b\[27m|\x1b\[\??\d+[hl]",
    )
    .unwrap();
    re.replace_all(&s, "").to_string()
}

fn is_alive(pid: c_int) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    let mut status: c_int = 0;
    let ret = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    ret <= 0
}

// ---------------------------------------------------------------------------
// Record server manager
// ---------------------------------------------------------------------------

struct RecordServer {
    child: Child,
    #[allow(dead_code)]
    port: u16,
    #[allow(dead_code)]
    cassette_path: PathBuf,
}

impl RecordServer {
    fn start(port: u16, cassette_path: PathBuf, mode: &str) -> Result<Self> {
        let binary = record_server_binary();
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
                kill_child(child.id());
                anyhow::bail!("pi-record-server did not start within 5s");
            }
            if client.get(&url).send().is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        Ok(Self {
            child,
            port,
            cassette_path,
        })
    }
}

impl Drop for RecordServer {
    fn drop(&mut self) {
        kill_child(self.child.id());
    }
}

fn kill_child(pid: u32) {
    unsafe { libc::kill(pid as c_int, libc::SIGINT) };
    // Give it time to shut down gracefully
    thread::sleep(Duration::from_millis(500));
    // Force kill if still alive
    unsafe { libc::kill(pid as c_int, libc::SIGKILL) };
    // Reap the child
    let mut status: c_int = 0;
    for _ in 0..10 {
        let ret = unsafe { libc::waitpid(pid as c_int, &mut status, libc::WNOHANG) };
        if ret != 0 {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

// ---------------------------------------------------------------------------
// PTY TUI runner
// ---------------------------------------------------------------------------

struct TuiRunner {
    master_fd: RawFd,
    child_pid: c_int,
}

impl TuiRunner {
    /// Spawn pio in a PTY, pointing at the given base_url.
    fn start(port: u16) -> Result<Self> {
        let binary = pio_binary();
        assert!(
            binary.exists(),
            "pio binary not found at {}",
            binary.display()
        );

        let (master_fd, slave_fd) = open_pty()?;

        // Collect env before fork
        let env_strings: Vec<String> = std::env::vars_os()
            .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
            .filter(|kv| {
                // Filter out test runner env vars that could confuse pio
                !kv.starts_with("CARGO_") && !kv.starts_with("RUST_TEST_")
            })
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
                    .filter_map(|s| CString::new(s).ok())
                    .map(|s| s.into_raw() as *const libc::c_char)
                    .collect();

                // TUI overrides
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
                    if let Ok(c) = CString::new(kv.as_str()) {
                        env_ptrs.push(c.into_raw() as *const libc::c_char);
                    }
                }
                env_ptrs.push(std::ptr::null());

                let prog = CString::new(binary.to_string_lossy().to_string()).unwrap();
                let skip = CString::new("--skip-onboarding").unwrap();

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

    /// Wait for initial render, then drain output.
    fn wait_ready(&mut self) -> String {
        thread::sleep(Duration::from_millis(600));
        let output = read_pty_timeout(self.master_fd, 300);
        strip_ansi(&output)
    }

    /// Type a prompt and press Enter, then wait for LLM response.
    /// Polls the PTY output until the response completes or timeout.
    fn submit_prompt(&mut self, text: &str, timeout_secs: u64) -> String {
        type_string(self.master_fd, text);
        thread::sleep(Duration::from_millis(100));
        send_enter(self.master_fd);

        // Poll for output until we see the Done marker or timeout
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut all_output = Vec::new();
        let mut last_read = Instant::now();

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let chunk = read_pty_timeout(self.master_fd, 500);
            if chunk.is_empty() {
                // No new data for 2 seconds, assume response is done
                if last_read.elapsed() > Duration::from_secs(2) {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            last_read = Instant::now();
            all_output.extend_from_slice(&chunk);

            // Check if we see the "Done" marker (stream completed)
            let s = String::from_utf8_lossy(&all_output);
            if s.contains("Done") || s.contains("[DONE]") {
                // Give it a bit more time for final re-render
                thread::sleep(Duration::from_millis(300));
                let final_chunk = read_pty_timeout(self.master_fd, 300);
                all_output.extend_from_slice(&final_chunk);
                break;
            }
        }

        strip_ansi(&all_output)
    }

    /// Wait for additional output.
    #[allow(dead_code)]
    fn drain_output(&mut self, timeout_ms: i32) -> String {
        thread::sleep(Duration::from_millis(timeout_ms as u64));
        let output = read_pty_timeout(self.master_fd, 500);
        strip_ansi(&output)
    }

    /// Send /quit to exit cleanly.
    fn quit(&mut self) {
        type_string(self.master_fd, "/quit");
        send_enter(self.master_fd);
        thread::sleep(Duration::from_millis(500));
        // If still alive, kill
        for _ in 0..10 {
            if !is_alive(self.child_pid) {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for TuiRunner {
    fn drop(&mut self) {
        if is_alive(self.child_pid) {
            kill_child(self.child_pid as u32);
        }
        unsafe { libc::close(self.master_fd) };
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Extract assistant text from stripped TUI output.
/// The TUI prefixes assistant content with "▌ ".
fn extract_assistant_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|line| line.contains('▌'))
        .map(|line| {
            // Remove the "▌ " prefix and any trailing whitespace
            line.replacen("▌ ", "", 1).trim().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
#[ignore = "requires DEEPSEEK_API_KEY and network — run with `cargo test -- --ignored`"]
fn record_and_replay_single_turn() -> Result<()> {
    let cassette = std::env::temp_dir().join("pi_test_record_replay.cassette.json");
    let _ = std::fs::remove_file(&cassette);
    let port: u16 = 19998;

    // Check DEEPSEEK_API_KEY
    if std::env::var("DEEPSEEK_API_KEY")
        .unwrap_or_default()
        .is_empty()
    {
        eprintln!("Skipping test: DEEPSEEK_API_KEY not set");
        return Ok(());
    }

    // ── Phase 1: Record ──────────────────────────────────────────────
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

    // Verify we got a response with content
    let record_lines = extract_assistant_lines(&record_output);
    assert!(
        !record_lines.is_empty(),
        "Should have assistant response; output: {:.500}",
        record_output
    );

    eprintln!("=== RECORD ASSISTANT LINES ===");
    for line in &record_lines {
        eprintln!("  > {}", line);
    }

    tui.quit();
    drop(tui);
    drop(_record_server);

    // Give the record server time to flush the cassette
    thread::sleep(Duration::from_secs(1));
    assert!(
        cassette.exists(),
        "Cassette should exist at {}",
        cassette.display()
    );

    // Validate cassette content
    let cassette_json =
        std::fs::read_to_string(&cassette).expect("should read cassette");
    let cassette_data: serde_json::Value =
        serde_json::from_str(&cassette_json).expect("cassette should be valid JSON");
    assert_eq!(cassette_data["version"], 2);
    assert!(
        !cassette_data["entries"].as_array().unwrap().is_empty(),
        "cassette should have at least one entry"
    );

    // ── Phase 2: Replay ─────────────────────────────────────────────
    let _replay_server = RecordServer::start(port, cassette.clone(), "replay")?;
    let mut tui2 = TuiRunner::start(port)?;
    let initial2 = tui2.wait_ready();
    assert!(
        initial2.contains("Ready"),
        "TUI should show Ready on replay"
    );

    let replay_output = tui2.submit_prompt(prompt, 5);
    let replay_lines = extract_assistant_lines(&replay_output);

    eprintln!("=== REPLAY ASSISTANT LINES ===");
    for line in &replay_lines {
        eprintln!("  > {}", line);
    }

    assert!(
        !replay_lines.is_empty(),
        "Replay should have assistant response; output: {:.500}",
        replay_output
    );

    // ── Phase 3: Compare ────────────────────────────────────────────
    // Replay must produce at least the same non-trivial content.
    // We filter out very short lines (likely PTY noise) and check
    // that meaningful record lines appear in replay.
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
        "Should have meaningful record output"
    );
    assert!(
        !replay_meaningful.is_empty(),
        "Should have meaningful replay output"
    );

    for line in &record_meaningful {
        let found = replay_meaningful
            .iter()
            .any(|r| r.contains(line) || line.contains(r));
        assert!(
            found,
            "Record line not found in replay: '{}'",
            line
        );
    }

    tui2.quit();
    drop(tui2);
    drop(_replay_server);

    // Clean up cassette
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

    // ── Phase 1: Record multi-turn ─────────────────────────────────
    let _record_server = RecordServer::start(port, cassette.clone(), "record")?;
    let mut tui = TuiRunner::start(port)?;
    let initial = tui.wait_ready();
    assert!(initial.contains("Ready"), "TUI should show Ready");

    // Turn 1
    let prompt1 = "What is pi-core? Answer in one sentence.";
    let out1 = tui.submit_prompt(prompt1, 30);
    let lines1 = extract_assistant_lines(&out1);
    eprintln!("=== RECORD TURN 1 ===");
    for line in &lines1 {
        eprintln!("  > {}", line);
    }
    assert!(!lines1.is_empty(), "Turn 1 should have response");

    // Turn 2
    let prompt2 = "And what about pi-llm? One sentence.";
    let out2 = tui.submit_prompt(prompt2, 30);
    let lines2 = extract_assistant_lines(&out2);
    eprintln!("=== RECORD TURN 2 ===");
    for line in &lines2 {
        eprintln!("  > {}", line);
    }
    assert!(!lines2.is_empty(), "Turn 2 should have response");

    tui.quit();
    drop(tui);
    drop(_record_server);
    thread::sleep(Duration::from_secs(1));
    assert!(cassette.exists(), "Cassette should exist");

    // Validate cassette content
    let cassette_json =
        std::fs::read_to_string(&cassette).expect("should read cassette");
    let cassette_data: serde_json::Value =
        serde_json::from_str(&cassette_json).expect("cassette should be valid JSON");
    assert_eq!(cassette_data["version"], 2);
    assert!(
        cassette_data["entries"].as_array().unwrap().len() >= 2,
        "multi-turn cassette should have at least 2 entries"
    );

    // ── Phase 2: Replay multi-turn ────────────────────────────────
    let _replay_server = RecordServer::start(port, cassette.clone(), "replay")?;
    let mut tui2 = TuiRunner::start(port)?;
    tui2.wait_ready();

    let r_out1 = tui2.submit_prompt(prompt1, 10);
    let r_lines1 = extract_assistant_lines(&r_out1);
    eprintln!("=== REPLAY TURN 1 ===");
    for line in &r_lines1 {
        eprintln!("  > {}", line);
    }

    let r_out2 = tui2.submit_prompt(prompt2, 10);
    let r_lines2 = extract_assistant_lines(&r_out2);
    eprintln!("=== REPLAY TURN 2 ===");
    for line in &r_lines2 {
        eprintln!("  > {}", line);
    }

    assert!(!r_lines1.is_empty(), "Replay turn 1 should have response");
    assert!(!r_lines2.is_empty(), "Replay turn 2 should have response");

    // ── Phase 3: Compare ──────────────────────────────────────────
    // Both turns should produce the same assistant content
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
