# TUI Improvements — Emacs-Style Input Editing

## Status: ✅ Complete (198 tests green, incl. 13 real-terminal E2E)

## What Changed

### New Files
- `src/kill_ring.rs` — Emacs-style kill/yank ring buffer
- `src/input_tests.rs` — 29 integration tests for input editing

### Modified Files
- `src/app.rs` — Key handling, new editing methods
- `src/main.rs` — Module declarations

### Key Bindings Added

| Key | Action | Status |
|-----|--------|--------|
| `Ctrl+A` | Move to line start | ✅ |
| `Ctrl+E` | Move to line end | ✅ |
| `Ctrl+B` | Cursor left (was: scroll page-up) | ✅ |
| `Ctrl+F` | Cursor right (was: scroll page-down) | ✅ |
| `Ctrl+W` | Delete word backward (kill ring) | ✅ |
| `Ctrl+K` | Delete to line end (kill ring) | ✅ |
| `Ctrl+U` | Delete to line start (kill ring) | ✅ |
| `Ctrl+D` | Delete char forward (kill ring) | ✅ |
| `Ctrl+Y` | Yank from kill ring | ✅ |
| `Alt+Y` | Yank-pop (rotate kill ring) | ✅ |
| `Alt+Backspace` | Delete word backward | ✅ |
| `Alt+Delete` | Delete word forward | ✅ |
| `Ctrl+Left` / `Alt+Left` | Word left | ✅ |
| `Ctrl+Right` / `Alt+Right` | Word right | ✅ |
| `Shift+Enter` | Insert newline (multi-line) | ✅ |
| `Esc` (running) | Interrupt (was: quit) | ✅ |
| `Esc` (idle) | Quit (unchanged) | ✅ |
| `Tab` | Cycle suggestions | ✅ |

### Scroll Keys (Still Work)
- `PageUp` / `PageDown` — Page scroll
- `Home` / `End` — Jump to top/bottom
- `Shift+Up` / `Shift+Down` — Line scroll

### Behavior Changes
1. **Ctrl+B/F repurposed** — Now cursor movement (Emacs standard), not scroll
2. **Esc when running** — Interrupts/cancels instead of quitting
3. **Kill ring accumulation** — Consecutive kills merge into one entry
4. **Tab cycles** — Tab now cycles through suggestions, not just shows them

## Test Coverage

29 new integration tests in `input_tests.rs`:
- Ctrl+A/E: 4 tests (start/end/noop variants)
- Ctrl+B/F: 4 tests (left/right/noop variants)
- Ctrl+W: 3 tests (delete/accumulate/noop)
- Ctrl+K: 2 tests (delete/noop)
- Ctrl+U: 2 tests (delete/noop)
- Ctrl+D: 2 tests (delete/noop)
- Ctrl+Y: 2 tests (yank/empty noop)
- Alt+Y: 1 test (yank-pop)
- Alt+Backspace/Delete: 2 tests
- Ctrl+Left/Right: 2 tests
- Shift+Enter: 1 test
- Esc: 3 tests (running/idle/suggestions)
- Tab: 1 test

All 198 tests pass (156 existing + 29 input + 13 e2e).

### E2E Tests (Real Terminal PTY)

13 E2E tests in `e2e_tests.rs` that spawn the `pio` binary in a real PTY:
- `e2e_tui_starts_and_shows_prompt` — binary launches and renders
- `e2e_type_text_and_delete` — type text, Ctrl+U clears it
- `e2e_ctrl_a_and_ctrl_e_cursor_movement` — Ctrl+A moves to start
- `e2e_ctrl_w_deletes_word` — Ctrl+W deletes word backward
- `e2e_ctrl_k_deletes_to_end` — Ctrl+K deletes to line end
- `e2e_ctrl_u_deletes_to_start` — Ctrl+U deletes to line start
- `e2e_ctrl_y_yanks_text` — Ctrl+Y yanks killed text
- `e2e_ctrl_d_deletes_forward` — Ctrl+D deletes char forward
- `e2e_backspace_deletes_char` — Backspace deletes character
- `e2e_shift_enter_inserts_newline` — multi-line input
- `e2e_esc_quits` — Escape quits the TUI
- `e2e_slash_shows_commands` — / + Tab shows autocomplete
- `e2e_ctrl_e_moves_to_end` — Ctrl+E moves to line end

E2E harness uses `nix::pty::openpty` + `libc::fork`/`execve` + `libc::poll`
for reliable cross-platform PTY I/O.
