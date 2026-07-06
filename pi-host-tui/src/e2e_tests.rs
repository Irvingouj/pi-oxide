//! E2E tests that run the `pio` binary in a real pseudo-terminal (PTY).
//!
//! These tests spawn the actual binary, send real keystrokes via the PTY master,
//! and read rendered output from the slave. This exercises the full crossterm
//! raw mode, ratatui rendering, and key event pipeline.
//!
//! Note: PTY output mixes terminal echo with TUI re-renders, making precise
//! substring assertions unreliable. Editing logic is tested in `input_tests.rs`.
//! E2E tests verify the full pipeline: startup, input acceptance, and shutdown.
//!
//! Requires: Unix (macOS/Linux).
#![allow(dead_code)]

use anyhow::{Context, Result};
use libc::{c_char, c_int};
use nix::pty::openpty;
use std::ffi::CString;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

// ---------------------------------------------------------------------------
// PTY helpers — raw libc to avoid nix API churn
// ---------------------------------------------------------------------------

/// Build the path to the `pio` binary for the current test run.
fn pio_binary() -> PathBuf {
    static BINARY: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut bin = BINARY.lock().unwrap();
    if let Some(ref p) = *bin {
        return p.clone();
    }
    let p = if let Ok(path) = std::env::var("CARGO_BIN_EXE_pio") {
        PathBuf::from(path)
    } else {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
        let workspace_root = PathBuf::from(&manifest_dir).parent().unwrap().to_path_buf();
        let candidate = workspace_root.join("target/debug/pio");
        if candidate.exists() {
            candidate
        } else {
            let status = std::process::Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("pi-host-tui")
                .current_dir(&workspace_root)
                .status()
                .unwrap();
            if !status.success() {
                panic!("cargo build failed");
            }
            workspace_root.join("target/debug/pio")
        }
    };
    *bin = Some(p.clone());
    p
}

/// Open a PTY and return (master_fd, slave_fd).
fn open_pty() -> Result<(RawFd, RawFd)> {
    let winsize = nix::pty::Winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = openpty(Some(&winsize), None).context("openpty")?;
    Ok((pty.master.into_raw_fd(), pty.slave.into_raw_fd()))
}

/// Send raw bytes to the PTY master fd (simulates keystrokes).
fn send_raw(fd: RawFd, data: &[u8]) {
    let res = unsafe {
        libc::write(
            fd,
            data.as_ptr() as *const libc::c_void,
            data.len() as libc::size_t,
        )
    };
    assert!(res >= 0, "write to pty failed: {}", res);
}

/// Send a Ctrl+letter keystroke.
fn send_ctrl(fd: RawFd, letter: char) {
    let code = (letter.to_lowercase().next().unwrap() as u8).wrapping_sub(b'a') + 1;
    send_raw(fd, &[code]);
}

/// Send Enter (CR).
fn send_enter(fd: RawFd) {
    send_raw(fd, b"\r");
}

/// Send Escape.
fn send_escape(fd: RawFd) {
    send_raw(fd, b"\x1b");
}

/// Send Tab.
fn send_tab(fd: RawFd) {
    send_raw(fd, b"\t");
}

/// Send Backspace.
fn send_backspace(fd: RawFd) {
    send_raw(fd, b"\x7f");
}

/// Send a printable character.
fn send_char(fd: RawFd, c: char) {
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    send_raw(fd, s.as_bytes());
}

/// Send Shift+Enter (legacy: ESC + CR).
fn send_shift_enter(fd: RawFd) {
    send_raw(fd, b"\x1b\r");
}

/// Read available output from PTY fd with a timeout using poll(2).
fn read_pty_timeout(fd: RawFd, timeout_ms: i32) -> Vec<u8> {
    let mut buf = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms as u64);

    loop {
        let now = std::time::Instant::now();
        let remaining = deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            break;
        }
        let poll_timeout_ms = remaining.as_millis().max(1) as c_int;

        let mut pfd = libc::pollfd {
            fd: fd as c_int,
            events: libc::POLLIN,
            revents: 0,
        };

        let ret = unsafe { libc::poll(&mut pfd, 1, poll_timeout_ms) };
        if ret <= 0 {
            break; // timeout or error
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

/// Strip ANSI escape sequences for readable assertions.
fn strip_ansi(input: &[u8]) -> String {
    let s = String::from_utf8_lossy(input);
    let re = regex::Regex::new(
        r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b\([A-HP-Z]|\x1b\[7m|\x1b\[27m|\x1b\[\??\d+[hl]",
    )
    .unwrap();
    re.replace_all(&s, "").to_string()
}

/// Check if a process is alive (not terminated and not a zombie).
fn is_alive(pid: c_int) -> bool {
    // First check if the process ID still exists
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false; // Process does not exist
    }
    // Process exists — check if it's a zombie by trying non-blocking waitpid.
    // If waitpid returns immediately with WNOHANG, the process has terminated (zombie).
    let mut status: c_int = 0;
    let ret = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if ret > 0 {
        // Process has terminated (we reaped it)
        false
    } else if ret == 0 {
        // Process is still running
        true
    } else {
        // Error (e.g., ECHILD) — process doesn't exist
        false
    }
}

// ---------------------------------------------------------------------------
// E2E test harness — fork + exec in PTY
// ---------------------------------------------------------------------------

struct E2EHarness {
    master_fd: RawFd,
    child_pid: c_int,
}

impl E2EHarness {
    fn new() -> Result<Self> {
        let binary = pio_binary();
        if !binary.exists() {
            anyhow::bail!("pio binary not found at {}", binary.display());
        }

        let (master_fd, slave_fd) = open_pty().context("open_pty")?;

        // Collect env BEFORE fork to avoid std::env mutex issues
        let env_strings: Vec<String> = std::env::vars_os()
            .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
            .collect();

        // Fork
        let pid = unsafe { libc::fork() };
        match pid {
            -1 => anyhow::bail!("fork failed"),
            0 => {
                // Child process
                unsafe { libc::close(master_fd) };

                // Ignore SIGINT so Ctrl+C is delivered as a raw byte to crossterm
                // rather than terminating the process.
                unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };

                // Dup slave to stdin/stdout/stderr
                unsafe {
                    libc::dup2(slave_fd, libc::STDIN_FILENO);
                    libc::dup2(slave_fd, libc::STDOUT_FILENO);
                    libc::dup2(slave_fd, libc::STDERR_FILENO);
                    if slave_fd > 2 {
                        libc::close(slave_fd);
                    }
                }

                // Build envp from pre-collected strings + overrides
                let mut env_ptrs: Vec<*const c_char> = env_strings
                    .into_iter()
                    .filter_map(|s| CString::new(s).ok())
                    .map(|s| s.into_raw() as *const c_char)
                    .collect();

                // Add test overrides
                for kv in &[
                    "PI_API_KEY=e2e-test-key",
                    "PI_MODEL=gpt-4",
                    "PI_PROVIDER=openai",
                    "PI_BASE_URL=http://localhost:9999",
                ] {
                    if let Ok(c) = CString::new(*kv) {
                        env_ptrs.push(c.into_raw() as *const c_char);
                    }
                }
                env_ptrs.push(std::ptr::null());

                // Build argv
                let prog_cstr =
                    CString::new(binary.to_string_lossy().to_string()).expect("binary path");
                let skip_cstr = CString::new("--skip-onboarding").expect("arg");

                let argv: [*const c_char; 3] =
                    [prog_cstr.as_ptr(), skip_cstr.as_ptr(), std::ptr::null()];

                unsafe {
                    libc::execve(argv[0], argv.as_ptr(), env_ptrs.as_ptr());
                    // If execve returns, it failed
                    let err_msg = format!("execve failed: {}\n", std::io::Error::last_os_error());
                    let _ = libc::write(
                        libc::STDERR_FILENO,
                        err_msg.as_ptr() as *const _,
                        err_msg.len(),
                    );
                    libc::_exit(1);
                }
            }
            child_pid => {
                // Parent
                unsafe { libc::close(slave_fd) };
                Ok(Self {
                    master_fd,
                    child_pid,
                })
            }
        }
    }

    /// Wait for initial render, then run test closure.
    /// Returns the combined output (initial render + test interaction).
    fn run<F>(&mut self, test_fn: F) -> Result<String>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        // Wait for initial render
        thread::sleep(Duration::from_millis(800));

        // Verify child is alive
        assert!(
            is_alive(self.child_pid),
            "Child process died before initial render"
        );

        // Drain initial output
        let mut all_output = read_pty_timeout(self.master_fd, 300);

        // Run the test
        test_fn(self)?;

        // Read final output and append
        let final_output = read_pty_timeout(self.master_fd, 300);
        all_output.extend_from_slice(&final_output);

        Ok(strip_ansi(&all_output))
    }

    /// Send a typed string with small delays between chars.
    fn type_string(&mut self, s: &str) {
        for c in s.chars() {
            send_char(self.master_fd, c);
            thread::sleep(Duration::from_millis(30));
        }
        thread::sleep(Duration::from_millis(100));
    }

    /// Read accumulated output.
    fn read_output(&mut self, timeout_ms: i32) -> String {
        let data = read_pty_timeout(self.master_fd, timeout_ms);
        strip_ansi(&data)
    }
}

impl Drop for E2EHarness {
    fn drop(&mut self) {
        // Kill the child process
        unsafe { libc::kill(self.child_pid, libc::SIGKILL) };
        // Wait for child to avoid zombie
        loop {
            let mut status: c_int = 0;
            match unsafe { libc::waitpid(self.child_pid, &mut status, 0) } {
                -1 => {
                    let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if err == libc::ECHILD || err == libc::EINTR {
                        break;
                    }
                    break;
                }
                _ => break,
            }
        }
        // Close master fd
        unsafe { libc::close(self.master_fd) };
    }
}

// ---------------------------------------------------------------------------
// E2E Tests
//
// PTY output mixes terminal echo with TUI re-renders, making precise substring
// assertions unreliable. These tests verify the full pipeline works:
// startup, input acceptance, command autocomplete, and shutdown.
// Editing logic (cursor movement, kill/yank) is tested in `input_tests.rs`.
// ---------------------------------------------------------------------------

/// Verify the TUI renders its prompt and status line on startup.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_tui_starts_and_shows_prompt() -> Result<()> {
    let mut h = E2EHarness::new()?;
    let output = h.run(|_| Ok(()))?;

    assert!(
        output.contains("Ready") && output.contains(">"),
        "TUI should render header and prompt; got: {}",
        output.chars().take(500).collect::<String>()
    );
    Ok(())
}

/// Verify the TUI accepts typed input without crashing.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_type_text_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    let output = h.run(|h| {
        h.type_string("hello world");
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    // Child should still be alive (no crash from input)
    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after typing input"
    );
    // Output should contain fragments of what was typed
    assert!(
        output.contains("hello") || output.contains("world"),
        "Typed text should appear in output; got: {}",
        output.chars().take(500).collect::<String>()
    );
    Ok(())
}

/// Verify Ctrl+U (kill line) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_u_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello");
        thread::sleep(Duration::from_millis(200));
        send_ctrl(h.master_fd, 'u');
        thread::sleep(Duration::from_millis(300));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+U"
    );
    Ok(())
}

/// Verify Ctrl+W (kill word) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_w_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello world");
        send_ctrl(h.master_fd, 'w');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+W"
    );
    Ok(())
}

/// Verify Ctrl+Y (yank) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_y_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello world");
        send_ctrl(h.master_fd, 'w');
        thread::sleep(Duration::from_millis(100));
        send_ctrl(h.master_fd, 'y');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+Y"
    );
    Ok(())
}

/// Sending SIGTERM quits the TUI cleanly.
///
/// We use SIGTERM rather than a PTY keystroke because crossterm's event
/// pipeline in a PTY is unreliable for control characters (escape timeouts,
/// terminal driver interception). SIGTERM verifies the process exits cleanly
/// and the harness cleanup works.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_sigterm_quits_cleanly() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        // Send SIGTERM to the child
        unsafe { libc::kill(h.child_pid, libc::SIGTERM) };
        // Give the process time to exit
        for _ in 0..20 {
            if !is_alive(h.child_pid) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !is_alive(h.child_pid),
            "SIGTERM should quit the TUI; child should be dead"
        );
        Ok(())
    })?;

    Ok(())
}

/// / + Tab shows the command autocomplete list.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_slash_shows_commands() -> Result<()> {
    let mut h = E2EHarness::new()?;
    let output = h.run(|h| {
        send_char(h.master_fd, '/');
        thread::sleep(Duration::from_millis(100));
        send_tab(h.master_fd);
        thread::sleep(Duration::from_millis(400));
        send_escape(h.master_fd);
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    // The command list should include known commands.
    assert!(
        output.contains("/clear") || output.contains("/help") || output.contains("/quit"),
        "Tab after '/' should show command list; got: {}",
        output.chars().take(500).collect::<String>()
    );
    Ok(())
}

/// Backspace doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_backspace_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello");
        for _ in 0..3 {
            send_backspace(h.master_fd);
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after backspace"
    );
    Ok(())
}

/// Ctrl+D (delete forward) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_d_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello");
        send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        send_ctrl(h.master_fd, 'd');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+D"
    );
    Ok(())
}

/// Shift+Enter (multi-line) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_shift_enter_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("line1");
        send_shift_enter(h.master_fd);
        thread::sleep(Duration::from_millis(100));
        h.type_string("line2");
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Shift+Enter"
    );
    Ok(())
}

/// Ctrl+A (move to start) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_a_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello");
        send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        send_char(h.master_fd, 'X');
        thread::sleep(Duration::from_millis(300));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+A"
    );
    Ok(())
}

/// Ctrl+E (move to end) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_e_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello");
        send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        send_ctrl(h.master_fd, 'e');
        thread::sleep(Duration::from_millis(100));
        h.type_string("XX");
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+E"
    );
    Ok(())
}

/// Ctrl+K (kill to end) doesn't crash the TUI.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_ctrl_k_no_crash() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        h.type_string("hello world");
        send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        for _ in 0..3 {
            send_ctrl(h.master_fd, 'f');
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(100));
        send_ctrl(h.master_fd, 'k');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        is_alive(h.child_pid),
        "TUI should not crash after Ctrl+K"
    );
    Ok(())
}
