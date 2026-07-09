//! E2E tests that run `pio` in a real pseudo-terminal (PTY).
//!
//! These tests spawn the actual binary, send real keystrokes via the PTY master,
//! and read rendered output from the slave. This exercises the full crossterm
//! raw mode, ratatui rendering, and key event pipeline.
//!
//! Note: PTY output mixes terminal echo with TUI re-renders, making precise
//! substring assertions unreliable. Editing logic is tested in `input_tests.rs`
//! in pi-host-tui. E2E tests verify the full pipeline: startup, input acceptance,
//! and shutdown.
//!
//! Requires: Unix (macOS/Linux).

use anyhow::{Context, Result};
use libc::c_int;
use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::thread;
use std::time::Duration;

use crate::helpers;

// ---------------------------------------------------------------------------
// E2E test harness — fork + exec in PTY
// ---------------------------------------------------------------------------

struct E2EHarness {
    master_fd: RawFd,
    child_pid: c_int,
}

impl E2EHarness {
    fn new() -> Result<Self> {
        let binary = helpers::pio_binary();
        assert!(
            binary.exists(),
            "pio binary not found at {}",
            binary.display()
        );

        let (master_fd, slave_fd) = helpers::open_pty_pair()?;

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
                let mut env_ptrs: Vec<*const libc::c_char> = env_strings
                    .into_iter()
                    .filter_map(|s| CString::new(s).ok())
                    .map(|s| s.into_raw() as *const libc::c_char)
                    .collect();

                // Add test overrides
                for kv in &[
                    "PI_API_KEY=e2e-test-key",
                    "PI_MODEL=gpt-4",
                    "PI_PROVIDER=openai",
                    "PI_BASE_URL=http://localhost:9999",
                ] {
                    if let Ok(c) = CString::new(*kv) {
                        env_ptrs.push(c.into_raw() as *const libc::c_char);
                    }
                }
                env_ptrs.push(std::ptr::null());

                // Build argv
                let prog_cstr =
                    CString::new(binary.to_string_lossy().to_string()).expect("binary path");
                let skip_cstr = CString::new("--skip-onboarding").expect("arg");

                let argv: [*const libc::c_char; 3] =
                    [prog_cstr.as_ptr(), skip_cstr.as_ptr(), std::ptr::null()];

                unsafe {
                    libc::execve(argv[0], argv.as_ptr(), env_ptrs.as_ptr());
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
    fn run<F>(&mut self, test_fn: F) -> Result<String>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        // Wait for initial render
        thread::sleep(Duration::from_millis(800));

        // Verify child is alive
        assert!(
            helpers::is_alive(self.child_pid),
            "Child process died before initial render"
        );

        // Drain initial output
        let mut all_output = helpers::read_pty_timeout(self.master_fd, 300);

        // Run the test
        test_fn(self)?;

        // Read final output and append
        let final_output = helpers::read_pty_timeout(self.master_fd, 300);
        all_output.extend_from_slice(&final_output);

        Ok(helpers::strip_ansi(&all_output))
    }

    /// Send a typed string with small delays between chars.
    fn type_string(&mut self, s: &str) {
        helpers::type_string(self.master_fd, s);
    }

    /// Read accumulated output.
    fn read_output(&mut self, timeout_ms: i32) -> String {
        let data = helpers::read_pty_timeout(self.master_fd, timeout_ms);
        helpers::strip_ansi(&data)
    }
}

impl Drop for E2EHarness {
    fn drop(&mut self) {
        helpers::kill_child(self.child_pid as u32);
        unsafe { libc::close(self.master_fd) };
    }
}

// ---------------------------------------------------------------------------
// E2E Tests
//
// PTY output mixes terminal echo with TUI re-renders, making precise substring
// assertions unreliable. These tests verify the full pipeline works:
// startup, input acceptance, command autocomplete, and shutdown.
// Editing logic (cursor movement, kill/yank) is tested in input_tests.rs.
// ---------------------------------------------------------------------------

/// Verify the TUI renders its prompt and status line on startup.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_tui_starts_and_shows_prompt() -> Result<()> {
    let mut h = E2EHarness::new()?;
    let output = h.run(|_| Ok(()))?;

    assert!(
        output.contains("Ready") && output.contains("pio"),
        "TUI should render system message and status bar; got: {}",
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
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'u');
        thread::sleep(Duration::from_millis(300));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'w');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'w');
        thread::sleep(Duration::from_millis(100));
        helpers::send_ctrl(h.master_fd, 'y');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
        "TUI should not crash after Ctrl+Y"
    );
    Ok(())
}

/// Sending SIGTERM quits the TUI cleanly.
#[test]
#[cfg_attr(target_os = "windows", ignore)]
fn e2e_sigterm_quits_cleanly() -> Result<()> {
    let mut h = E2EHarness::new()?;
    h.run(|h| {
        unsafe { libc::kill(h.child_pid, libc::SIGTERM) };
        for _ in 0..20 {
            if !helpers::is_alive(h.child_pid) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !helpers::is_alive(h.child_pid),
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
        helpers::send_char(h.master_fd, '/');
        thread::sleep(Duration::from_millis(100));
        helpers::send_tab(h.master_fd);
        thread::sleep(Duration::from_millis(400));
        helpers::send_escape(h.master_fd);
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

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
            helpers::send_backspace(h.master_fd);
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        helpers::send_ctrl(h.master_fd, 'd');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_shift_enter(h.master_fd);
        thread::sleep(Duration::from_millis(100));
        h.type_string("line2");
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        helpers::send_char(h.master_fd, 'X');
        thread::sleep(Duration::from_millis(300));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        helpers::send_ctrl(h.master_fd, 'e');
        thread::sleep(Duration::from_millis(100));
        h.type_string("XX");
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
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
        helpers::send_ctrl(h.master_fd, 'a');
        thread::sleep(Duration::from_millis(100));
        for _ in 0..3 {
            helpers::send_ctrl(h.master_fd, 'f');
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(100));
        helpers::send_ctrl(h.master_fd, 'k');
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })?;

    assert!(
        helpers::is_alive(h.child_pid),
        "TUI should not crash after Ctrl+K"
    );
    Ok(())
}
