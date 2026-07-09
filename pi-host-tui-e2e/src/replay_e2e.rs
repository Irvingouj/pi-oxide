//! Replay-only E2E tests — fully offline, no API key, deterministic.
//!
//! These tests use pre-recorded HTTP cassettes committed to
//! pi-host-tui/tests/fixtures/. The pi-record-server replays the cassette,
//! and pio runs against it in a PTY.
//!
//! No network, no API key, no #[ignore]. Runs in `cargo test`.

use anyhow::{Context, Result};
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::helpers;

// ---------------------------------------------------------------------------
// Replay server manager
// ---------------------------------------------------------------------------

struct ReplayServer {
    child: Child,
}

impl ReplayServer {
    fn start(port: u16, cassette: &Path) -> Result<Self> {
        let binary = helpers::record_server_binary();
        assert!(
            binary.exists(),
            "pi-record-server not found at {}",
            binary.display()
        );

        let child = Command::new(&binary)
            .arg("replay")
            .arg("--port")
            .arg(port.to_string())
            .arg("--cassette")
            .arg(cassette.to_string_lossy().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start pi-record-server")?;

        let url = format!("http://localhost:{port}/v1/models");
        let client = reqwest::blocking::Client::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                helpers::kill_child(child.id());
                anyhow::bail!("replay server did not start within 5s");
            }
            if client.get(&url).send().is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(Self { child })
    }
}

impl Drop for ReplayServer {
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
    fn start(port: u16) -> Result<Self> {
        let binary = helpers::pio_binary();
        assert!(binary.exists(), "pio not found at {}", binary.display());

        let (master_fd, slave_fd) = helpers::open_pty_pair()?;

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

                for kv in &[
                    format!("PI_BASE_URL=http://localhost:{port}"),
                    "PI_PROVIDER=deepseek".into(),
                    "PI_MODEL=deepseek-chat".into(),
                    "PI_API_KEY=test-key-not-real".into(),
                ] {
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
                    libc::write(libc::STDERR_FILENO, msg.as_ptr() as *const _, msg.len());
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
        helpers::strip_ansi(&helpers::read_pty_timeout(self.master_fd, 300))
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

static NEXT_PORT: AtomicU16 = AtomicU16::new(20000);

fn next_port() -> u16 {
    NEXT_PORT.fetch_add(1, Ordering::Relaxed)
}

/// Replay a pre-recorded cassette through pio and verify the TUI renders
/// the expected text. Fully offline, no API key needed.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn replay_cassette_multi_turn_offline() -> Result<()> {
    let cassette = helpers::fixture_path("deepseek_http.cassette.json");
    assert!(
        cassette.exists(),
        "fixture not found: {}",
        cassette.display()
    );

    let port = next_port();
    let _server = ReplayServer::start(port, &cassette)?;
    let mut tui = TuiRunner::start(port)?;
    let initial = tui.wait_ready();
    assert!(initial.contains("Ready"));

    // Turn 1: "Say hello in one word" → cassette entry 0 returns "hello"
    let out1 = tui.submit_prompt("Say hello in one word", 10);
    assert!(out1.contains("hello"), "Turn 1 should contain 'hello'");

    // Turn 2: "What is 7*8? One word." → cassette entry 1 returns "Fifty-six"
    let out2 = tui.submit_prompt("What is 7*8? One word.", 10);
    assert!(
        out2.contains("ifty-six"),
        "Turn 2 should contain 'ifty-six'"
    );

    // Turn 3: cassette exhausted → pio shows "Done" silently
    let _out3 = tui.submit_prompt("anything", 5);

    tui.quit();
    drop(tui);
    drop(_server);

    Ok(())
}

/// Verify the replay server exhausts and returns 503.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn replay_cassette_exhausts() -> Result<()> {
    let cassette = helpers::fixture_path("deepseek_http.cassette.json");
    let port = next_port();
    let _server = ReplayServer::start(port, &cassette)?;

    // Consume all entries
    for _ in 0..2 {
        let resp = reqwest::blocking::Client::new()
            .post(format!("http://localhost:{port}/v1/chat/completions"))
            .header("content-type", "application/json")
            .body(r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"x"}],"stream":true}"#)
            .send()?;
        assert!(resp.status().is_success());
        let _ = resp.bytes()?;
    }

    let resp = reqwest::blocking::Client::new()
        .post(format!("http://localhost:{port}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(
            r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"x"}],"stream":true}"#,
        )
        .send()?;
    assert_eq!(resp.status().as_u16(), 503);
    assert!(resp.text()?.contains("exhausted"));

    Ok(())
}
