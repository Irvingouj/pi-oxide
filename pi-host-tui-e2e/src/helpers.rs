//! E2E test helpers — shared PTY primitives for pi-host-tui E2E tests.
//!
//! These tests spawn the `pio` binary in a real pseudo-terminal (PTY),
//! send keystrokes via the PTY master, and read rendered output.
//!
//! Requires: Unix (macOS/Linux).

#![allow(dead_code)]

use anyhow::{Context, Result};
use libc::c_int;
use nix::pty::openpty;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Binary path resolution
// ---------------------------------------------------------------------------

/// Build the path to the `pio` binary. If not found via CARGO_BIN_EXE_pio,
/// tries workspace target/debug.
pub fn pio_binary() -> PathBuf {
    static BINARY: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut bin = BINARY.lock().unwrap();
    if let Some(ref p) = *bin {
        return p.clone();
    }
    let p = resolve_binary("pio");
    *bin = Some(p.clone());
    p
}

/// Build the path to the `pi-record-server` binary.
pub fn record_server_binary() -> PathBuf {
    static BINARY: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut bin = BINARY.lock().unwrap();
    if let Some(ref p) = *bin {
        return p.clone();
    }
    let p = resolve_binary("pi-record-server");
    *bin = Some(p.clone());
    p
}

fn resolve_binary(name: &str) -> PathBuf {
    let env_key = format!("CARGO_BIN_EXE_{}", name.replace('-', "_").to_uppercase());
    if let Ok(path) = std::env::var(&env_key) {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }

    // Fallback: look in workspace target/debug
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
    let workspace_root = PathBuf::from(&manifest_dir).parent().unwrap().to_path_buf();

    let candidate = workspace_root.join("target/debug").join(name);
    if candidate.exists() {
        return candidate;
    }

    // Try to build the binary
    eprintln!(
        "Binary '{}' not found at {}, attempting cargo build...",
        name,
        candidate.display()
    );
    let status = std::process::Command::new("cargo")
        .arg("build")
        .args(["-p", &format!("pi-{name}")])
        .current_dir(&workspace_root)
        .status();

    match status {
        Ok(s) if s.success() => {
            if candidate.exists() {
                candidate
            } else {
                panic!(
                    "cargo build -p pi-{name} succeeded but binary not found at {}",
                    candidate.display()
                );
            }
        }
        Ok(s) => panic!("cargo build -p pi-{name} failed with status {}", s),
        Err(e) => panic!("failed to run cargo build -p pi-{name}: {}", e),
    }
}

/// Find fixture files relative to workspace root.
pub fn fixture_path(name: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
    PathBuf::from(&manifest_dir)
        .join("..")
        .join("pi-host-tui")
        .join("tests")
        .join("fixtures")
        .join(name)
}

// ---------------------------------------------------------------------------
// PTY helpers
// ---------------------------------------------------------------------------

/// Open a PTY and return (master_fd, slave_fd).
pub fn open_pty_pair() -> Result<(RawFd, RawFd)> {
    let winsize = nix::pty::Winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = openpty(Some(&winsize), None).context("openpty")?;
    Ok((pty.master.into_raw_fd(), pty.slave.into_raw_fd()))
}

/// Send raw bytes to the PTY master fd (simulates keystrokes).
pub fn send_raw(fd: RawFd, data: &[u8]) {
    let res = unsafe {
        libc::write(
            fd,
            data.as_ptr() as *const libc::c_void,
            data.len() as libc::size_t,
        )
    };
    assert!(res >= 0, "write to pty failed");
}

/// Send a Ctrl+letter keystroke.
pub fn send_ctrl(fd: RawFd, letter: char) {
    let code = (letter.to_lowercase().next().unwrap() as u8).wrapping_sub(b'a') + 1;
    send_raw(fd, &[code]);
}

/// Send Enter (CR).
pub fn send_enter(fd: RawFd) {
    send_raw(fd, b"\r");
}

/// Send Escape.
pub fn send_escape(fd: RawFd) {
    send_raw(fd, b"\x1b");
}

/// Send Tab.
pub fn send_tab(fd: RawFd) {
    send_raw(fd, b"\t");
}

/// Send Backspace.
pub fn send_backspace(fd: RawFd) {
    send_raw(fd, b"\x7f");
}

/// Send a printable character.
pub fn send_char(fd: RawFd, c: char) {
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    send_raw(fd, s.as_bytes());
}

/// Send Shift+Enter (ESC + CR).
pub fn send_shift_enter(fd: RawFd) {
    send_raw(fd, b"\x1b\r");
}

/// Type a string character by character with small delays.
pub fn type_string(fd: RawFd, s: &str) {
    for c in s.chars() {
        send_char(fd, c);
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(50));
}

/// Read available output from PTY fd with a timeout using poll(2).
pub fn read_pty_timeout(fd: RawFd, timeout_ms: i32) -> Vec<u8> {
    let mut buf = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

    loop {
        let now = Instant::now();
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

/// Strip ANSI escape sequences for readable assertions.
pub fn strip_ansi(input: &[u8]) -> String {
    let s = String::from_utf8_lossy(input);
    let re = regex::Regex::new(
        r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b\([A-HP-Z]|\x1b\[7m|\x1b\[27m|\x1b\[\??\d+[hl]",
    )
    .unwrap();
    re.replace_all(&s, "").to_string()
}

// ---------------------------------------------------------------------------
// Process management
// ---------------------------------------------------------------------------

/// Check if a process is alive (not terminated and not a zombie).
pub fn is_alive(pid: c_int) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    let mut status: c_int = 0;
    let ret = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if ret > 0 {
        false
    } else {
        ret == 0
    }
}

/// Kill a child process gracefully then forcefully, then reap.
pub fn kill_child(pid: u32) {
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
