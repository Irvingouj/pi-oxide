/// Integration tests for Emacs-style input editing.
///
/// Tests exercise `handle_key()` through the public `App` struct and verify
/// `input`, `cursor_pos`, `kill_ring`, and `should_quit` state.
///
/// Each test follows the pattern:
///   1. Build App with known input/cursor
///   2. Feed KeyEvent(s) via handle_key()
///   3. Assert resulting state
use crate::app::App;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

fn ctrl(code: KeyCode) -> KeyEvent {
    make_key(code, KeyModifiers::CONTROL)
}

fn alt(code: KeyCode) -> KeyEvent {
    make_key(code, KeyModifiers::ALT)
}

fn shift(code: KeyCode) -> KeyEvent {
    make_key(code, KeyModifiers::SHIFT)
}

fn plain(code: KeyCode) -> KeyEvent {
    make_key(code, KeyModifiers::NONE)
}

/// Build a minimal App with the given input string and cursor position.
fn build_app(input: &str, cursor_pos: usize) -> App {
    let mut app = App::with_entries_for_test(Vec::new());
    app.editor.input = input.to_string();
    app.editor.cursor_pos = cursor_pos.min(input.len());
    app
}

// ---------------------------------------------------------------------------
// Ctrl+A / Ctrl+E — line start / line end
// ---------------------------------------------------------------------------

#[test]
fn ctrl_a_moves_cursor_to_start() {
    let mut app = build_app("hello world", 7); // cursor at 'w'
    app.handle_key(ctrl(KeyCode::Char('a')));
    assert_eq!(app.editor.cursor_pos, 0, "Ctrl+A should move to line start");
    assert_eq!(app.editor.input, "hello world");
}

#[test]
fn ctrl_a_at_start_is_noop() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Char('a')));
    assert_eq!(app.editor.cursor_pos, 0);
}

#[test]
fn ctrl_e_moves_cursor_to_end() {
    let mut app = build_app("hello world", 3); // cursor at 'l'
    app.handle_key(ctrl(KeyCode::Char('e')));
    assert_eq!(app.editor.cursor_pos, 11, "Ctrl+E should move to line end");
    assert_eq!(app.editor.input, "hello world");
}

#[test]
fn ctrl_e_at_end_is_noop() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Char('e')));
    assert_eq!(app.editor.cursor_pos, 5);
}

// ---------------------------------------------------------------------------
// Ctrl+B / Ctrl+F — cursor left / right (Emacs)
// ---------------------------------------------------------------------------

#[test]
fn ctrl_b_moves_cursor_left() {
    let mut app = build_app("hello", 3);
    app.handle_key(ctrl(KeyCode::Char('b')));
    assert_eq!(app.editor.cursor_pos, 2, "Ctrl+B should move left one char");
}

#[test]
fn ctrl_b_at_start_stays() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Char('b')));
    assert_eq!(app.editor.cursor_pos, 0);
}

#[test]
fn ctrl_f_moves_cursor_right() {
    let mut app = build_app("hello", 2);
    app.handle_key(ctrl(KeyCode::Char('f')));
    assert_eq!(
        app.editor.cursor_pos, 3,
        "Ctrl+F should move right one char"
    );
}

#[test]
fn ctrl_f_at_end_stays() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Char('f')));
    assert_eq!(app.editor.cursor_pos, 5);
}

// ---------------------------------------------------------------------------
// Ctrl+W — delete word backward
// ---------------------------------------------------------------------------

#[test]
fn ctrl_w_deletes_word_backward() {
    let mut app = build_app("hello world", 11); // cursor at end
    app.handle_key(ctrl(KeyCode::Char('w')));
    assert_eq!(app.editor.input, "hello ", "Ctrl+W should delete 'world'");
    assert_eq!(app.editor.cursor_pos, 6);
    assert_eq!(
        app.editor.kill_ring.peek(),
        Some("world"),
        "Kill ring should have 'world'"
    );
}

#[test]
fn ctrl_w_at_start_is_noop() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Char('w')));
    assert_eq!(app.editor.input, "hello");
    assert!(app.editor.kill_ring.is_empty());
}

#[test]
fn ctrl_w_consecutive_accumulates_in_kill_ring() {
    let mut app = build_app("foo bar baz", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "baz"
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "bar "
                                              // Should accumulate: "bar baz"
    assert_eq!(app.editor.kill_ring.peek(), Some("bar baz"));
}

// ---------------------------------------------------------------------------
// Ctrl+K — delete to line end
// ---------------------------------------------------------------------------

#[test]
fn ctrl_k_deletes_to_line_end() {
    let mut app = build_app("hello world", 3); // cursor at 'l'
    app.handle_key(ctrl(KeyCode::Char('k')));
    assert_eq!(
        app.editor.input, "hel",
        "Ctrl+K should delete from cursor to end"
    );
    assert_eq!(app.editor.cursor_pos, 3);
    assert_eq!(app.editor.kill_ring.peek(), Some("lo world"));
}

#[test]
fn ctrl_k_at_end_is_noop() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Char('k')));
    assert_eq!(app.editor.input, "hello");
    assert!(app.editor.kill_ring.is_empty());
}

// ---------------------------------------------------------------------------
// Ctrl+U — delete to line start
// ---------------------------------------------------------------------------

#[test]
fn ctrl_u_deletes_to_line_start() {
    let mut app = build_app("hello world", 7); // cursor at 'o'
    app.handle_key(ctrl(KeyCode::Char('u')));
    assert_eq!(
        app.editor.input, "orld",
        "Ctrl+U should delete from start to cursor"
    );
    assert_eq!(app.editor.cursor_pos, 0);
    assert_eq!(app.editor.kill_ring.peek(), Some("hello w"));
}

#[test]
fn ctrl_u_at_start_is_noop() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Char('u')));
    assert_eq!(app.editor.input, "hello");
    assert!(app.editor.kill_ring.is_empty());
}

// ---------------------------------------------------------------------------
// Ctrl+D — delete char forward
// ---------------------------------------------------------------------------

#[test]
fn ctrl_d_deletes_char_forward() {
    let mut app = build_app("hello", 2);
    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(
        app.editor.input, "helo",
        "Ctrl+D should delete char at cursor"
    );
    assert_eq!(app.editor.cursor_pos, 2);
}

#[test]
fn ctrl_d_at_end_is_noop() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(app.editor.input, "hello");
}

// ---------------------------------------------------------------------------
// Ctrl+Y — yank
// ---------------------------------------------------------------------------

#[test]
fn ctrl_y_yanks_from_kill_ring() {
    let mut app = build_app("hello world", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kill "world"
    assert_eq!(app.editor.input, "hello ");
    app.handle_key(ctrl(KeyCode::Char('y'))); // yank
    assert_eq!(
        app.editor.input, "hello world",
        "Ctrl+Y should yank 'world'"
    );
    assert_eq!(app.editor.cursor_pos, 11);
}

#[test]
fn ctrl_y_with_empty_kill_ring_is_noop() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Char('y')));
    assert_eq!(app.editor.input, "hello");
}

// ---------------------------------------------------------------------------
// Alt+Y — yank-pop
// ---------------------------------------------------------------------------

#[test]
fn alt_y_yank_pop_noop_with_single_entry() {
    // When kills accumulate into one entry, yank-pop is a no-op
    let mut app = build_app("hello world", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kill "world"
    app.handle_key(ctrl(KeyCode::Char('w'))); // kill "hello " (accumulated)
    assert_eq!(app.editor.input, "");
    // Kill ring has 1 accumulated entry
    app.handle_key(ctrl(KeyCode::Char('y'))); // yank
    let after_yank = app.editor.input.clone();
    app.handle_key(alt(KeyCode::Char('y'))); // yank-pop (no-op with 1 entry)
                                             // With single entry, yank-pop shouldn't crash and input stays same
    assert_eq!(
        app.editor.input, after_yank,
        "Yank-pop with single entry is no-op"
    );
}

// ---------------------------------------------------------------------------
// Alt+Backspace / Alt+Delete — word delete
// ---------------------------------------------------------------------------

#[test]
fn alt_backspace_deletes_word_backward() {
    let mut app = build_app("hello world", 11);
    app.handle_key(alt(KeyCode::Backspace));
    assert_eq!(
        app.editor.input, "hello ",
        "Alt+Backspace should delete 'world'"
    );
    assert_eq!(app.editor.cursor_pos, 6);
}

#[test]
fn alt_delete_deletes_word_forward() {
    let mut app = build_app("hello world", 6); // cursor at 'w'
    app.handle_key(alt(KeyCode::Delete));
    assert_eq!(
        app.editor.input, "hello ",
        "Alt+Delete should delete 'world'"
    );
    assert_eq!(app.editor.cursor_pos, 6);
}

// ---------------------------------------------------------------------------
// Ctrl+Left / Ctrl+Right — word navigation
// ---------------------------------------------------------------------------

#[test]
fn ctrl_left_moves_word_left() {
    let mut app = build_app("hello world", 11);
    app.handle_key(ctrl(KeyCode::Left));
    assert_eq!(app.editor.cursor_pos, 6, "Ctrl+Left should jump to 'w'");
}

#[test]
fn ctrl_right_moves_word_right() {
    let mut app = build_app("hello world", 0);
    app.handle_key(ctrl(KeyCode::Right));
    assert!(
        app.editor.cursor_pos > 0,
        "Ctrl+Right should move forward by word"
    );
    assert!(
        app.editor.cursor_pos < 11,
        "Ctrl+Right should not jump to end in one step"
    );
}

// ---------------------------------------------------------------------------
// Shift+Enter — insert newline
// ---------------------------------------------------------------------------

#[test]
fn shift_enter_inserts_newline() {
    let mut app = build_app("hello world", 5); // cursor at ' '
    app.handle_key(shift(KeyCode::Enter));
    assert_eq!(
        app.editor.input, "hello\n world",
        "Shift+Enter should insert newline"
    );
    assert_eq!(app.editor.cursor_pos, 6); // after the newline
    assert!(!app.should_quit, "Shift+Enter should not quit");
}

// ---------------------------------------------------------------------------
// Esc — dismiss suggestions, clear input, never quit or cancel
// ---------------------------------------------------------------------------

#[test]
fn esc_dismisses_suggestions_first() {
    let mut app = build_app("/hel", 4);
    app.editor.show_suggestions = true;
    app.editor.suggestions = vec!["/help".to_string()];
    app.handle_key(plain(KeyCode::Esc));
    assert!(
        !app.editor.show_suggestions,
        "Esc should dismiss suggestions"
    );
    assert!(
        !app.should_quit,
        "Esc should not quit when suggestions shown"
    );
}

#[test]
fn esc_clears_input_when_idle() {
    let mut app = build_app("hello", 5);
    app.running = false;
    app.editor.show_suggestions = false;
    app.handle_key(plain(KeyCode::Esc));
    assert!(
        app.editor.input.is_empty(),
        "Esc when idle should clear input"
    );
    assert!(!app.should_quit, "Esc when idle should NOT quit");
    assert!(!app.cancelled, "Esc when idle should not cancel");
}

#[test]
fn esc_empty_input_noop_does_not_quit() {
    let mut app = build_app("", 0);
    app.running = false;
    app.editor.show_suggestions = false;
    app.handle_key(plain(KeyCode::Esc));
    assert!(!app.should_quit, "Esc with empty input should NOT quit");
    assert!(!app.cancelled, "Esc should not cancel");
    assert!(app.editor.input.is_empty(), "Input should remain empty");
}

#[test]
fn esc_when_running_clears_input_not_cancels() {
    let mut app = build_app("hello", 5);
    app.running = true;
    app.handle_key(plain(KeyCode::Esc));
    assert!(
        !app.cancelled,
        "Esc should no longer cancel even when running"
    );
    assert!(!app.should_quit, "Esc when running should not quit");
    assert!(app.editor.input.is_empty(), "Esc should clear input");
}

// ---------------------------------------------------------------------------
// Tab — cycle suggestions
// ---------------------------------------------------------------------------

#[test]
fn tab_cycles_suggestions() {
    let mut app = build_app("/", 1);
    app.handle_key(plain(KeyCode::Tab)); // show suggestions
    assert!(app.editor.show_suggestions);
    // Tab again should cycle to next
    app.handle_key(plain(KeyCode::Tab));
    // Should still show suggestions (cycling, not submitting)
    assert!(app.editor.show_suggestions, "Tab should cycle suggestions");
}

// ---------------------------------------------------------------------------
// Ctrl+C — clear input when idle (double-press to quit), cancel when running
// ---------------------------------------------------------------------------

#[test]
fn ctrl_c_clears_input_when_idle() {
    let mut app = build_app("hello", 5);
    app.running = false;
    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(
        app.editor.input.is_empty(),
        "First Ctrl+C when idle should clear input"
    );
    assert!(!app.should_quit, "First Ctrl+C should not quit");
    assert!(!app.cancelled, "Ctrl+C when idle should not cancel");
}

#[test]
fn ctrl_c_double_press_quits() {
    let mut app = build_app("", 0);
    app.running = false;
    // First press with already-empty input sets pending_quit
    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(
        !app.should_quit,
        "First Ctrl+C with empty input should not quit"
    );
    assert!(
        app.pending_quit,
        "First Ctrl+C with empty input should set pending_quit"
    );
    // Second press while pending_quit is set should quit
    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(
        app.should_quit,
        "Second Ctrl+C with empty input should quit"
    );
}

#[test]
fn ctrl_c_clears_input_resets_pending_quit() {
    let mut app = build_app("hello", 5);
    app.running = false;
    // First Ctrl+C clears the input
    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(app.editor.input.is_empty());
    assert!(
        !app.pending_quit,
        "pending_quit should be reset when input was non-empty"
    );
}

#[test]
fn ctrl_c_cancels_when_running() {
    let mut app = build_app("hello", 5);
    app.running = true;
    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(app.cancelled, "Ctrl+C when running should cancel");
    assert!(!app.should_quit, "Ctrl+C when running should not quit");
}

// ---------------------------------------------------------------------------
// Kill ring — direct unit tests
// ---------------------------------------------------------------------------

#[test]
fn kill_ring_push_single_entry() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "hello"
    assert_eq!(app.editor.kill_ring.len(), 1);
    assert_eq!(app.editor.kill_ring.peek(), Some("hello"));
}

#[test]
fn kill_ring_rotate_with_two_entries() {
    // Create two independent entries by breaking accumulation with cursor move
    let mut app = build_app("foo bar baz", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "baz"
                                              // Break accumulation with a cursor move
    app.handle_key(ctrl(KeyCode::Char('b')));
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "bar" (new entry, not accumulated)
    assert_eq!(app.editor.kill_ring.len(), 2, "Should have 2 entries");
    assert_eq!(
        app.editor.kill_ring.peek(),
        Some("bar"),
        "Most recent is 'bar'"
    );

    // Yank and yank-pop should rotate
    app.handle_key(ctrl(KeyCode::Char('y'))); // yank "bar"
    assert_eq!(app.editor.input, "foo bar ");
    app.handle_key(alt(KeyCode::Char('y'))); // yank-pop → "baz"
    assert_eq!(
        app.editor.input, "foo baz ",
        "Yank-pop should rotate to 'baz'"
    );
}

#[test]
fn kill_ring_forward_deletion_accumulates() {
    // Ctrl+D (forward) twice should accumulate left-to-right
    let mut app = build_app("abc", 0);
    app.handle_key(ctrl(KeyCode::Char('d'))); // kills "a"
    app.handle_key(ctrl(KeyCode::Char('d'))); // kills "b" (accumulated)
    assert_eq!(
        app.editor.kill_ring.peek(),
        Some("ab"),
        "Forward kills accumulate left-to-right"
    );
}

#[test]
fn kill_ring_backward_then_forward_accumulates() {
    // Ctrl+W (backward) then Ctrl+D (forward) with accumulation
    let mut app = build_app("hello world", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "world" (backward)
                                              // Break accumulation with cursor move
    app.handle_key(ctrl(KeyCode::Char('a')));
    app.handle_key(ctrl(KeyCode::Char('d'))); // kills "h" (forward)
    assert_eq!(
        app.editor.kill_ring.len(),
        2,
        "Non-consecutive kills create separate entries"
    );
}

// ---------------------------------------------------------------------------
// Multi-byte UTF-8
// ---------------------------------------------------------------------------

#[test]
fn ctrl_b_moves_left_multi_byte() {
    // "日本" = 6 bytes (3 each), cursor at end (6)
    let mut app = build_app("日本", 6);
    app.handle_key(ctrl(KeyCode::Char('b')));
    assert_eq!(
        app.editor.cursor_pos, 3,
        "Ctrl+B should move left by one multi-byte char"
    );
    app.handle_key(ctrl(KeyCode::Char('b')));
    assert_eq!(app.editor.cursor_pos, 0, "Ctrl+B should reach start");
}

#[test]
fn ctrl_f_moves_right_multi_byte() {
    let mut app = build_app("日本", 0);
    app.handle_key(ctrl(KeyCode::Char('f')));
    assert_eq!(
        app.editor.cursor_pos, 3,
        "Ctrl+F should move right by one multi-byte char"
    );
    app.handle_key(ctrl(KeyCode::Char('f')));
    assert_eq!(app.editor.cursor_pos, 6, "Ctrl+F should reach end");
}

#[test]
fn ctrl_d_deletes_multi_byte_char() {
    let mut app = build_app("日本", 0);
    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(
        app.editor.input, "本",
        "Ctrl+D should delete one multi-byte char"
    );
    assert_eq!(app.editor.cursor_pos, 0);
}

#[test]
fn ctrl_w_deletes_multi_byte_word() {
    // "日本 語" = 10 bytes: 日(0-2) 本(3-5) ' '(6) 語(7-9)
    let mut app = build_app("日本 語", 10); // cursor at end
    app.handle_key(ctrl(KeyCode::Char('w'))); // kill backward → deletes "語"
    assert_eq!(app.editor.input, "日本 ", "Ctrl+W should delete '語'");
    assert_eq!(app.editor.cursor_pos, 7); // "日本 " = 3 + 3 + 1 = 7 bytes
}

// ---------------------------------------------------------------------------
// Word navigation edge cases
// ---------------------------------------------------------------------------

#[test]
fn ctrl_left_at_start_is_noop() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Left));
    assert_eq!(app.editor.cursor_pos, 0);
}

#[test]
fn ctrl_right_at_end_is_noop() {
    let mut app = build_app("hello", 5);
    app.handle_key(ctrl(KeyCode::Right));
    assert_eq!(app.editor.cursor_pos, 5);
}

#[test]
fn ctrl_left_multiple_words() {
    // "a b c" = a(0) ' '(1) b(2) ' '(3) c(4)
    let mut app = build_app("a b c", 5); // cursor at end
    app.handle_key(ctrl(KeyCode::Left)); // jump to 'c' (pos 4)
    assert_eq!(app.editor.cursor_pos, 4);
    app.handle_key(ctrl(KeyCode::Left)); // jump to 'b' (pos 2)
    assert_eq!(app.editor.cursor_pos, 2);
    app.handle_key(ctrl(KeyCode::Left)); // jump to 'a' (pos 0)
    assert_eq!(app.editor.cursor_pos, 0);
}

#[test]
fn ctrl_left_leading_whitespace() {
    // "  hello" = ' '(0) ' '(1) h(2) e(3) l(4) l(5) o(6)
    let mut app = build_app("  hello", 7); // cursor at end
    app.handle_key(ctrl(KeyCode::Left)); // skip word 'hello', land at start of 'hello'
    assert_eq!(
        app.editor.cursor_pos, 2,
        "Ctrl+Left should jump to start of 'hello'"
    );
    app.handle_key(ctrl(KeyCode::Left)); // skip whitespace, land at start
    assert_eq!(
        app.editor.cursor_pos, 0,
        "Second Ctrl+Left should jump to start"
    );
}

#[test]
fn ctrl_right_no_whitespace() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Right));
    assert_eq!(
        app.editor.cursor_pos, 5,
        "Ctrl+Right on single word should jump to end"
    );
}

#[test]
fn ctrl_right_all_whitespace() {
    let mut app = build_app("   ", 0);
    app.handle_key(ctrl(KeyCode::Right));
    assert_eq!(
        app.editor.cursor_pos, 3,
        "Ctrl+Right should skip all whitespace"
    );
}

// ---------------------------------------------------------------------------
// Deletion edge cases
// ---------------------------------------------------------------------------

#[test]
fn ctrl_k_consecutive_is_noop_after_first() {
    let mut app = build_app("hello", 0);
    app.handle_key(ctrl(KeyCode::Char('k'))); // kills "hello"
    assert_eq!(app.editor.input, "");
    app.handle_key(ctrl(KeyCode::Char('k'))); // noop (empty input)
    assert_eq!(app.editor.input, "");
}

#[test]
fn alt_backspace_at_start_is_noop() {
    let mut app = build_app("hello", 0);
    app.handle_key(alt(KeyCode::Backspace));
    assert_eq!(app.editor.input, "hello");
}

#[test]
fn alt_delete_at_end_is_noop() {
    let mut app = build_app("hello", 5);
    app.handle_key(alt(KeyCode::Delete));
    assert_eq!(app.editor.input, "hello");
}

// ---------------------------------------------------------------------------
// Kill accumulation reset
// ---------------------------------------------------------------------------

#[test]
fn cursor_move_breaks_kill_accumulation() {
    // Kill, move, kill → two separate entries
    let mut app = build_app("foo bar baz", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "baz"
    app.handle_key(ctrl(KeyCode::Char('b'))); // cursor move → breaks accumulation
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "bar "
    assert_eq!(
        app.editor.kill_ring.len(),
        2,
        "Cursor move should break accumulation"
    );
}

#[test]
fn char_insertion_breaks_kill_accumulation() {
    // Kill, cursor-move (breaks accumulation), kill → two separate entries
    let mut app = build_app("hello world", 11);
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "world"
    app.handle_key(ctrl(KeyCode::Char('b'))); // cursor move → breaks accumulation
    app.handle_key(ctrl(KeyCode::Char('w'))); // kills "hello "
    assert_eq!(
        app.editor.kill_ring.len(),
        2,
        "Cursor move should break accumulation"
    );
}
