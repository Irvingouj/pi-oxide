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

#[cfg(all(test, not(feature = "replay")))]
mod tests {
    use super::*;

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
        let key = make_key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn derive_scroll_ctrl_f_not_scroll() {
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
        assert!(!auto || off == 50);
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
    fn apply_scroll_shift_up_disengages_auto() {
        let key = make_key(KeyCode::Up, KeyModifiers::SHIFT);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (off, auto) = apply_scroll(intent, 100, 10, 0, true);
        assert!(!auto, "auto_scroll should be disengaged");
        assert_eq!(off, 89, "should scroll up one row from bottom");
    }

    #[test]
    fn apply_scroll_home_jumps_to_top() {
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (off, auto) = apply_scroll(intent, 100, 10, 50, false);
        assert_eq!(off, 0);
        assert!(!auto);
    }

    #[test]
    fn apply_scroll_end_rearms_auto() {
        let key = make_key(KeyCode::End, KeyModifiers::NONE);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (_off, auto) = apply_scroll(intent, 100, 10, 0, false);
        assert!(auto, "auto_scroll should be re-armed");
    }

    #[test]
    fn apply_scroll_plain_up_not_consumed() {
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }
}
