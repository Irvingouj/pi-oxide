#[cfg(test)]
use crossterm::event::KeyEventKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ---------------------------------------------------------------------------
// Scroll key handling
// ---------------------------------------------------------------------------

/// Scroll intent derived from a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollIntent {
    Up,       // one row
    Down,     // one row
    PageUp,   // one viewport
    PageDown, // one viewport
    Top,      // to start
    Bottom,   // to end / re-arm auto_scroll
}

/// Map a `KeyEvent` to a `ScrollIntent`. Returns `None` when the key is not
/// a scroll key.
pub(crate) fn derive_scroll_intent(key: &KeyEvent) -> Option<ScrollIntent> {
    match key.code {
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => Some(ScrollIntent::Up),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => Some(ScrollIntent::Down),
        KeyCode::PageUp => Some(ScrollIntent::PageUp),
        KeyCode::PageDown => Some(ScrollIntent::PageDown),
        KeyCode::Home => Some(ScrollIntent::Top),
        KeyCode::End => Some(ScrollIntent::Bottom),
        // Note: Ctrl+B/F are now cursor movement (Emacs), not scroll
        _ => None,
    }
}

/// Pure state transition. Returns the new `(scroll_offset, auto_scroll)`.
pub(crate) fn apply_scroll(
    intent: ScrollIntent,
    total_lines: u16,
    visible: u16,
    scroll_offset: u16,
    auto_scroll: bool,
) -> (u16, bool) {
    // Everything fits — no scrolling needed
    if total_lines <= visible {
        let (off, auto) = match intent {
            ScrollIntent::Bottom => (scroll_offset, true),
            _ => (scroll_offset, auto_scroll),
        };
        tracing::debug!(
            ?intent,
            total_lines,
            visible,
            scroll_offset,
            auto_scroll_before = auto_scroll,
            offset = off,
            auto_scroll = auto,
            "apply_scroll: fits"
        );
        return (off, auto);
    }

    let max_offset = total_lines - visible;

    let (off, auto) = match intent {
        ScrollIntent::Up => {
            if auto_scroll {
                (max_offset.saturating_sub(1), false)
            } else {
                (scroll_offset.saturating_sub(1), false)
            }
        }
        ScrollIntent::Down => {
            if auto_scroll {
                (scroll_offset, true)
            } else {
                let new_offset = (scroll_offset + 1).min(max_offset);
                if new_offset >= max_offset {
                    (max_offset, true)
                } else {
                    (new_offset, false)
                }
            }
        }
        ScrollIntent::PageUp => {
            if auto_scroll {
                (max_offset.saturating_sub(visible), false)
            } else {
                (scroll_offset.saturating_sub(visible), false)
            }
        }
        ScrollIntent::PageDown => {
            if auto_scroll {
                (scroll_offset, true)
            } else {
                let new_offset = (scroll_offset + visible).min(max_offset);
                if new_offset >= max_offset {
                    (max_offset, true)
                } else {
                    (new_offset, false)
                }
            }
        }
        ScrollIntent::Top => (0, false),
        ScrollIntent::Bottom => (scroll_offset, true),
    };

    tracing::debug!(
        ?intent,
        total_lines,
        visible,
        scroll_offset,
        auto_scroll_before = auto_scroll,
        offset = off,
        auto_scroll = auto,
        "apply_scroll"
    );
    (off, auto)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, ChatEntry};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn derive_scroll_shift_up() {
        let key = make_key(KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Up));
    }

    #[test]
    fn derive_scroll_shift_down() {
        let key = make_key(KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Down));
    }

    #[test]
    fn derive_scroll_page_up() {
        let key = make_key(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::PageUp));
    }

    #[test]
    fn derive_scroll_page_down() {
        let key = make_key(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::PageDown));
    }

    #[test]
    fn derive_scroll_home() {
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Top));
    }

    #[test]
    fn derive_scroll_end() {
        let key = make_key(KeyCode::End, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Bottom));
    }

    #[test]
    fn derive_scroll_ctrl_b_not_scroll() {
        // Ctrl+B is now cursor left (Emacs), not scroll
        let key = make_key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn derive_scroll_ctrl_f_not_scroll() {
        // Ctrl+F is now cursor right (Emacs), not scroll
        let key = make_key(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn derive_scroll_plain_up_not_scroll() {
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn derive_scroll_plain_down_not_scroll() {
        let key = make_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn apply_scroll_top() {
        let (off, auto) = apply_scroll(ScrollIntent::Top, 100, 10, 50, false);
        assert_eq!(off, 0);
        assert!(!auto);
    }

    #[test]
    fn apply_scroll_bottom() {
        let (off, auto) = apply_scroll(ScrollIntent::Bottom, 100, 10, 50, false);
        assert!(!auto || off == 50); // offset unchanged, auto=true
        assert!(auto);
    }

    #[test]
    fn apply_scroll_up_from_auto() {
        let (off, auto) = apply_scroll(ScrollIntent::Up, 100, 10, 90, true);
        assert_eq!(off, 89);
        assert!(!auto);
    }

    #[test]
    fn apply_scroll_down_from_auto_noop() {
        let (off, auto) = apply_scroll(ScrollIntent::Down, 100, 10, 90, true);
        assert_eq!(off, 90);
        assert!(auto);
    }

    #[test]
    fn apply_scroll_down_reaches_bottom() {
        let (off, auto) = apply_scroll(ScrollIntent::Down, 100, 10, 89, false);
        assert_eq!(off, 90);
        assert!(auto);
    }

    #[test]
    fn apply_scroll_fits_all_noop() {
        let (off, auto) = apply_scroll(ScrollIntent::Up, 5, 10, 0, true);
        assert_eq!(off, 0);
        assert!(auto);
    }

    #[test]
    fn handle_key_scroll_shift_up_disengages_auto() {
        // Simulate what handle_key does for scroll keys:
        // 1. derive_scroll_intent maps Shift+Up -> ScrollIntent::Up
        // 2. apply_scroll computes new state
        // 3. handle_key writes back to self
        let key = make_key(KeyCode::Up, KeyModifiers::SHIFT);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        // 100 lines, 10 visible, auto_scroll=true, offset=0 (at bottom = 90)
        let (off, auto) = apply_scroll(intent, 100, 10, 0, true);
        assert!(!auto, "auto_scroll should be disengaged");
        assert_eq!(off, 89, "should scroll up one row from bottom");
    }

    #[test]
    fn handle_key_scroll_home_jumps_to_top() {
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (off, auto) = apply_scroll(intent, 100, 10, 50, false);
        assert_eq!(off, 0);
        assert!(!auto);
    }

    #[test]
    fn handle_key_scroll_end_rearms_auto() {
        let key = make_key(KeyCode::End, KeyModifiers::NONE);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (_off, auto) = apply_scroll(intent, 100, 10, 0, false);
        assert!(auto, "auto_scroll should be re-armed");
    }

    #[test]
    fn handle_key_plain_up_not_consumed_as_scroll() {
        // Plain Up (no modifier) should NOT be treated as a scroll key
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    // -----------------------------------------------------------------------
    // E2E: render -> scroll key -> re-render -> assert buffer content changed
    // -----------------------------------------------------------------------

    fn build_scroll_entries() -> Vec<ChatEntry> {
        (0..30)
            .map(|i| ChatEntry::System(format!("Line {i:02}")))
            .collect()
    }

    fn get_backend_render(app: &mut App, terminal: &mut Terminal<TestBackend>) -> String {
        terminal.draw(|f| app.render(f)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn e2e_home_end_scroll() {
        let entries = build_scroll_entries();
        let mut app = App::with_entries_for_test(entries);
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // Initial render — auto-scroll should show bottom
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "auto-scroll should show bottom; got: {rendered}"
        );
        assert!(
            !rendered.contains("Line 00"),
            "top should not be visible; got: {rendered}"
        );

        // Press Home -> jump to top
        let consumed = app.handle_key(make_key(KeyCode::Home, KeyModifiers::NONE));
        assert!(consumed, "Home should be consumed as scroll key");
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 00"),
            "after Home, top should be visible; got: {rendered}"
        );
        assert!(
            !rendered.contains("Line 29"),
            "after Home, bottom should not be visible; got: {rendered}"
        );

        // Press End -> jump to bottom
        let consumed = app.handle_key(make_key(KeyCode::End, KeyModifiers::NONE));
        assert!(consumed, "End should be consumed as scroll key");
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "after End, bottom should be visible; got: {rendered}"
        );
        assert!(
            !rendered.contains("Line 00"),
            "after End, top should not be visible; got: {rendered}"
        );
    }

    #[test]
    fn e2e_page_scroll() {
        let entries = build_scroll_entries();
        let mut app = App::with_entries_for_test(entries);
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // Initial render at bottom
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "start at bottom; got: {rendered}"
        );

        // PageUp -> shift up one viewport
        app.handle_key(make_key(KeyCode::PageUp, KeyModifiers::NONE));
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            !rendered.contains("Line 29"),
            "after PageUp, bottom should not be visible; got: {rendered}"
        );
        assert!(!app.auto_scroll, "PageUp should disengage auto_scroll");

        // PageDown twice -> back to bottom
        app.handle_key(make_key(KeyCode::PageDown, KeyModifiers::NONE));
        app.handle_key(make_key(KeyCode::PageDown, KeyModifiers::NONE));
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "after 2x PageDown, bottom should be visible; got: {rendered}"
        );
    }

    #[test]
    fn e2e_partial_entry_overlap() {
        // Build entries where one entry is very long (wraps to many lines)
        // so it may only partially overlap the visible range.
        // Short entries first to fill up, then one long entry at the end.
        let mut entries: Vec<ChatEntry> = (0..10)
            .map(|i| ChatEntry::System(format!("Short {i:02}")))
            .collect();
        // One long user message that wraps to ~15 lines at width 40
        let long_text = (0..15)
            .map(|i| format!("Long line {i:02} of the big message"))
            .collect::<Vec<_>>()
            .join("\n");
        entries.push(ChatEntry::User(long_text));

        let mut app = App::with_entries_for_test(entries);
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // Initial render at bottom — should show end of long message
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Long line 14"),
            "auto-scroll should show end of long message; got: {rendered}"
        );
        assert!(
            !rendered.contains("Short 00"),
            "top short entries should not be visible; got: {rendered}"
        );

        // Scroll to top
        app.handle_key(make_key(KeyCode::Home, KeyModifiers::NONE));
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Short 00"),
            "after Home, first short entry should be visible; got: {rendered}"
        );

        // Scroll line by line down until we hit the long message
        for _ in 0..20 {
            app.handle_key(make_key(KeyCode::Down, KeyModifiers::SHIFT));
        }
        let rendered = get_backend_render(&mut app, &mut terminal);
        // We should be somewhere in the long message now
        assert!(
            rendered.contains("Long line"),
            "after scrolling down, long message should be visible; got: {rendered}"
        );
    }
}
