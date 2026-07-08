//! Replay-only E2E tests — fully offline, no API key, deterministic.
//!
//! These tests use pre-recorded HTTP cassettes committed to
//! `tests/fixtures/deepseek_http.cassette.json`. The pi-record-server
//! replays the cassette, and pio runs against it in a PTY.
//!
//! No network, no API key, no #[ignore]. Runs in `cargo test`.

use anyhow::{Context, Result};
use libc::c_int;
use std::ffi::CString;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Binary paths
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

fn fixture_path(name: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
    PathBuf::from(&manifest_dir)
        .join("tests")
        .join("fixtures")
        .join(name)
}

// ---------------------------------------------------------------------------
// PTY helpers
// ---------------------------------------------------------------------------

fn open_pty() -> Result<(RawFd, RawFd)> {
    let winsize = nix::pty::Winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = nix::pty::openpty(Some(&winsize), None).context("openpty")?;
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
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let poll_timeout_ms = remaining.as_millis().max(1) as libc::c_int;
        let mut pfd = libc::pollfd {
            fd: fd as libc::c_int,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pfd, 1, poll_timeout_ms) } <= 0 {
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
    (unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) }) <= 0
}

fn kill_child(pid: u32) {
    unsafe { libc::kill(pid as c_int, libc::SIGINT) };
    thread::sleep(Duration::from_millis(500));
    unsafe { libc::kill(pid as c_int, libc::SIGKILL) };
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
// Replay server manager
// ---------------------------------------------------------------------------

struct ReplayServer {
    child: Child,
}

impl ReplayServer {
    fn start(port: u16, cassette: &Path) -> Result<Self> {
        let binary = record_server_binary();
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

        // Wait for ready
        let url = format!("http://localhost:{port}/v1/models");
        let client = reqwest::blocking::Client::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                kill_child(child.id());
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
        kill_child(self.child.id());
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
    fn start(port: u16) -> Result<Self> {
        let binary = pio_binary();
        assert!(binary.exists(), "pio not found at {}", binary.display());

        let (master_fd, slave_fd) = open_pty()?;

        let env_strings: Vec<String> = std::env::vars_os()
            .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
            .filter(|kv| !kv.starts_with("CARGO_") && !kv.starts_with("RUST_TEST_"))
            .collect();

        let pid = unsafe { libc::fork() };
        match pid {
            -1 => anyhow::bail!("fork failed"),
            0 => {
                unsafe {
                    libc::close(master_fd);
                }
                unsafe {
                    libc::signal(libc::SIGINT, libc::SIG_IGN);
                }
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

                for kv in &[
                    format!("PI_BASE_URL=http://localhost:{port}"),
                    "PI_PROVIDER=deepseek".into(),
                    "PI_MODEL=deepseek-chat".into(),
                    "PI_API_KEY=test-key-not-real".into(),
                ] {
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
                    libc::write(libc::STDERR_FILENO, msg.as_ptr() as *const _, msg.len());
                    libc::_exit(1);
                }
            }
            child_pid => {
                unsafe {
                    libc::close(slave_fd);
                }
                Ok(Self {
                    master_fd,
                    child_pid,
                })
            }
        }
    }

    fn wait_ready(&mut self) -> String {
        thread::sleep(Duration::from_millis(600));
        strip_ansi(&read_pty_timeout(self.master_fd, 300))
    }

    /// Type prompt, press Enter, wait for stream to complete.
    fn submit_prompt(&mut self, text: &str, timeout_secs: u64) -> String {
        type_string(self.master_fd, text);
        thread::sleep(Duration::from_millis(100));
        send_enter(self.master_fd);

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
                let final_chunk = read_pty_timeout(self.master_fd, 300);
                all_output.extend_from_slice(&final_chunk);
                break;
            }
        }
        strip_ansi(&all_output)
    }

    fn quit(&mut self) {
        type_string(self.master_fd, "/quit");
        send_enter(self.master_fd);
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
        unsafe {
            libc::close(self.master_fd);
        }
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
    let cassette = fixture_path("deepseek_http.cassette.json");
    assert!(
        cassette.exists(),
        "fixture not found: {}",
        cassette.display()
    );

    let port = next_port();
    let _server = ReplayServer::start(port, &cassette)?;
    let mut tui = TuiRunner::start(port)?;
    let initial = tui.wait_ready();
    assert!(initial.contains("Ready"), "TUI should show Ready");

    // Turn 1: "Say hello in one word" → cassette entry 0 returns "hello"
    let out1 = tui.submit_prompt("Say hello in one word", 10);
    eprintln!("=== TURN 1 ===\n{}", out1);
    assert!(
        out1.contains("hello"),
        "Turn 1 should contain 'hello'; got:\n{}",
        out1
    );

    // Turn 2: "What is 7*8? One word." → cassette entry 1 returns "Fifty-six"
    let out2 = tui.submit_prompt("What is 7*8? One word.", 10);
    eprintln!("=== TURN 2 ===\n{}", out2);
    assert!(
        out2.contains("ifty-six"),
        "Turn 2 should contain 'ifty-six'; got:\n{}",
        out2
    );

    // Turn 3: cassette exhausted → pio shows "Done" silently (known behavior)
    let out3 = tui.submit_prompt("anything", 5);
    eprintln!("=== TURN 3 (exhausted) ===\n{}", out3);
    // Pio shows Done but no assistant text when cassette is exhausted

    tui.quit();
    drop(tui);
    drop(_server);

    Ok(())
}

/// Verify the replay server exhausts and returns 503.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn replay_cassette_exhausts() -> Result<()> {
    let cassette = fixture_path("deepseek_http.cassette.json");
    let port = next_port();
    let _server = ReplayServer::start(port, &cassette)?;

    // Consume all 3 entries
    for _ in 0..2 {
        let resp = reqwest::blocking::Client::new()
            .post(format!("http://localhost:{port}/v1/chat/completions"))
            .header("content-type", "application/json")
            .body(r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"x"}],"stream":true}"#)
            .send()?;
        assert!(resp.status().is_success(), "entry should be served");
        // Drain the body
        let _ = resp.bytes()?;
    }

    // 3rd request should get 503
    let resp = reqwest::blocking::Client::new()
        .post(format!("http://localhost:{port}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(
            r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"x"}],"stream":true}"#,
        )
        .send()?;
    assert_eq!(resp.status().as_u16(), 503, "4th request should exhaust");
    assert!(resp.text()?.contains("exhausted"));

    Ok(())
}
