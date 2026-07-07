use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;

// ---------------------------------------------------------------------------
// Kill ring (Emacs-style kill/yank)
// ---------------------------------------------------------------------------

/// Ring buffer for Emacs-style kill/yank operations.
///
/// Tracks killed (deleted) text entries. Consecutive kills accumulate
/// into a single entry. Supports yank (paste most recent) and yank-pop
/// (cycle through older entries).
pub(crate) struct KillRing {
    ring: Vec<String>,
}

impl KillRing {
    pub(crate) fn new() -> Self {
        Self { ring: Vec::new() }
    }

    /// Add text to the kill ring.
    ///
    /// - `prepend`: if accumulating, prepend (backward deletion) or append (forward deletion)
    /// - `accumulate`: merge with the most recent entry instead of creating a new one
    pub(crate) fn push(&mut self, text: String, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate && !self.ring.is_empty() {
            if let Some(last) = self.ring.pop() {
                self.ring.push(if prepend {
                    format!("{text}{last}")
                } else {
                    format!("{last}{text}")
                });
            }
        } else {
            self.ring.push(text);
        }
    }

    /// Get most recent entry without modifying the ring.
    pub(crate) fn peek(&self) -> Option<&str> {
        self.ring.last().map(|s| s.as_str())
    }

    /// Move last entry to front (for yank-pop cycling).
    pub(crate) fn rotate(&mut self) {
        if self.ring.len() > 1 {
            if let Some(last) = self.ring.pop() {
                self.ring.insert(0, last);
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.ring.len()
    }
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

pub(crate) const COMMANDS: &[&str] = &[
    "/clear",
    "/help",
    "/model",
    "/quit",
    "/session list",
    "/session load",
    "/session new",
    "/tokens",
    "/undo",
    "/config",
];

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

pub(crate) struct Editor {
    pub input: String,
    pub cursor_pos: usize,

    // Kill ring
    pub kill_ring: KillRing,
    last_kill_action: bool,
    last_yank: Option<(usize, String)>,

    // History
    history: Vec<String>,
    history_index: Option<usize>,
    original_input: String,

    // Suggestions
    pub suggestions: Vec<String>,
    pub show_suggestions: bool,
    pub suggestion_state: ListState,
}

impl Editor {
    pub(crate) fn new() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            kill_ring: KillRing::new(),
            last_kill_action: false,
            last_yank: None,
            history: Vec::new(),
            history_index: None,
            original_input: String::new(),
            suggestions: Vec::new(),
            show_suggestions: false,
            suggestion_state: ListState::default(),
        }
    }

    // -----------------------------------------------------------------------
    // Public key handlers
    // -----------------------------------------------------------------------

    /// Insert a character at the cursor position.
    pub(crate) fn push_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        self.reset_kill_accumulation();
        if self.show_suggestions && !self.input.starts_with('/') {
            self.show_suggestions = false;
        } else if self.input.starts_with('/') {
            self.update_suggestions();
        }
    }

    /// Handle Backspace key.
    pub(crate) fn handle_backspace(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.input[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_pos -= prev;
            self.input.remove(self.cursor_pos);
        }
        self.reset_kill_accumulation();
        if self.input.is_empty() || !self.input.starts_with('/') {
            self.show_suggestions = false;
        } else {
            self.update_suggestions();
        }
    }

    /// Move cursor left by one character.
    pub(crate) fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos = self
                .input
                .char_indices()
                .take_while(|(i, _)| *i < self.cursor_pos)
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
        self.reset_kill_accumulation();
    }

    /// Move cursor right by one character.
    pub(crate) fn move_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos = self.input[self.cursor_pos..]
                .chars()
                .next()
                .map(|c| self.cursor_pos + c.len_utf8())
                .unwrap_or(self.input.len());
        }
        self.reset_kill_accumulation();
    }

    /// Move cursor to line start.
    pub(crate) fn move_home(&mut self) {
        self.cursor_pos = 0;
        self.reset_kill_accumulation();
    }

    /// Move cursor to line end.
    pub(crate) fn move_end(&mut self) {
        self.cursor_pos = self.input.len();
        self.reset_kill_accumulation();
    }

    /// Handle Enter key. Returns `true` if the key was consumed.
    /// Returns `true` for Shift+Enter (newline inserted) and for plain Enter
    /// when running/input empty.
    /// Returns `false` for plain Enter when ready to submit (non-empty, not running).
    pub(crate) fn handle_enter(&mut self, is_shift: bool) -> bool {
        if is_shift {
            self.input.insert(self.cursor_pos, '\n');
            self.cursor_pos += 1;
            self.reset_kill_accumulation();
            return true;
        }
        if self.show_suggestions {
            // Accept selected suggestion (with trailing space) and submit
            self.apply_selected_suggestion();
            self.show_suggestions = false;
            // Fall through — return false so caller submits
        }
        // Return false to signal caller should submit
        false
    }

    /// Handle Tab key — show or cycle through suggestions.
    pub(crate) fn handle_tab(&mut self) {
        if self.show_suggestions {
            // Cycle with wrapping: don't go past the last suggestion
            let count = self.suggestions.len();
            if let Some(idx) = self.suggestion_state.selected() {
                if idx + 1 < count {
                    self.suggestion_state.select(Some(idx + 1));
                } else if count > 1 {
                    self.suggestion_state.select(Some(0));
                }
                // else: single suggestion, stay at 0
            }
        } else if self.input.starts_with('/') {
            self.update_suggestions();
        }
        self.apply_selected_suggestion();
    }

    /// Handle Up key — navigate suggestions or history.
    pub(crate) fn handle_up(&mut self) {
        if self.show_suggestions {
            let count = self.suggestions.len();
            if let Some(idx) = self.suggestion_state.selected() {
                if idx > 0 {
                    self.suggestion_state.select(Some(idx - 1));
                } else if count > 1 {
                    self.suggestion_state.select(Some(count - 1));
                }
            }
            self.apply_selected_suggestion();
        } else {
            self.history_recall_previous();
        }
    }

    /// Handle Down key — navigate suggestions or history.
    pub(crate) fn handle_down(&mut self) {
        if self.show_suggestions {
            let count = self.suggestions.len();
            if let Some(idx) = self.suggestion_state.selected() {
                if idx + 1 < count {
                    self.suggestion_state.select(Some(idx + 1));
                } else if count > 1 {
                    self.suggestion_state.select(Some(0));
                }
            }
            self.apply_selected_suggestion();
        } else {
            self.history_recall_next();
        }
    }

    /// Apply the currently selected suggestion to the input, trailing space included.
    fn apply_selected_suggestion(&mut self) {
        if let Some(idx) = self.suggestion_state.selected() {
            if let Some(cmd) = self.suggestions.get(idx).cloned() {
                self.input = format!("{} ", cmd);
                self.cursor_pos = self.input.len();
            }
        }
    }

    /// Dismiss suggestion popup. Returns `true` if suggestions were shown and dismissed.
    pub(crate) fn dismiss_suggestions(&mut self) -> bool {
        if self.show_suggestions {
            self.show_suggestions = false;
            true
        } else {
            false
        }
    }

    /// Handle Ctrl+letter keys.
    pub(crate) fn handle_ctrl_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            // Cursor movement
            KeyCode::Char('a') => {
                // Ctrl+A: move to line start
                self.cursor_pos = 0;
                self.reset_kill_accumulation();
                true
            }
            KeyCode::Char('e') => {
                // Ctrl+E: move to line end
                self.cursor_pos = self.input.len();
                self.reset_kill_accumulation();
                true
            }
            KeyCode::Char('b') => {
                // Ctrl+B: move cursor left one grapheme
                if self.cursor_pos > 0 {
                    let before = &self.input[..self.cursor_pos];
                    let last_char_len = before.chars().last().map(|c| c.len_utf8()).unwrap_or(0);
                    self.cursor_pos -= last_char_len;
                }
                self.reset_kill_accumulation();
                true
            }
            KeyCode::Char('f') => {
                // Ctrl+F: move cursor right one grapheme
                if self.cursor_pos < self.input.len() {
                    let after = &self.input[self.cursor_pos..];
                    let first_char_len = after.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                    self.cursor_pos += first_char_len;
                }
                self.reset_kill_accumulation();
                true
            }

            // Deletion
            KeyCode::Char('w') => {
                // Ctrl+W: delete word backward
                self.delete_word_backward();
                true
            }
            KeyCode::Char('k') => {
                // Ctrl+K: delete to line end
                self.delete_to_line_end();
                true
            }
            KeyCode::Char('u') => {
                // Ctrl+U: delete to line start
                self.delete_to_line_start();
                true
            }
            KeyCode::Char('d') => {
                // Ctrl+D: delete char forward
                self.delete_char_forward();
                true
            }

            // Yank
            KeyCode::Char('y') => {
                // Ctrl+Y: yank from kill ring
                self.yank();
                self.reset_kill_accumulation();
                true
            }

            // Word navigation
            KeyCode::Left => {
                // Ctrl+Left: word left
                self.move_word_backward();
                self.reset_kill_accumulation();
                true
            }
            KeyCode::Right => {
                // Ctrl+Right: word right
                self.move_word_forward();
                self.reset_kill_accumulation();
                true
            }

            _ => false,
        }
    }

    /// Handle Alt+key editing keys.
    pub(crate) fn handle_alt_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            // Word navigation
            KeyCode::Left => {
                self.move_word_backward();
                self.reset_kill_accumulation();
                true
            }
            KeyCode::Right => {
                self.move_word_forward();
                self.reset_kill_accumulation();
                true
            }

            // Word deletion
            KeyCode::Backspace => {
                // Alt+Backspace: delete word backward
                self.delete_word_backward();
                true
            }
            KeyCode::Delete => {
                // Alt+Delete: delete word forward
                self.delete_word_forward();
                true
            }

            // Yank-pop
            KeyCode::Char('y') => {
                // Alt+Y: yank-pop
                self.yank_pop();
                self.reset_kill_accumulation();
                true
            }

            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // Cursor movement helpers
    // -----------------------------------------------------------------------

    /// Move cursor to previous word boundary.
    fn move_word_backward(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let mut byte_idx = self.cursor_pos;

        // Skip whitespace backward; if only whitespace before cursor, go to start.
        let mut found_non_ws = false;
        for (i, c) in self.input.char_indices().rev() {
            if i >= byte_idx {
                continue;
            }
            if !c.is_whitespace() {
                byte_idx = i;
                found_non_ws = true;
                break;
            }
        }
        if !found_non_ws {
            self.cursor_pos = 0;
            return;
        }
        // Skip word characters backward
        for (i, c) in self.input.char_indices().rev() {
            if i >= byte_idx {
                continue;
            }
            if c.is_whitespace() {
                break;
            }
            byte_idx = i;
        }
        self.cursor_pos = byte_idx;
    }

    /// Move cursor to next word boundary.
    fn move_word_forward(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        let mut byte_idx = self.cursor_pos;

        // Skip whitespace forward
        for (i, c) in self.input.char_indices() {
            if i < byte_idx {
                continue;
            }
            if !c.is_whitespace() {
                break;
            }
            byte_idx = i + c.len_utf8();
        }
        // Skip word characters forward
        for (i, c) in self.input.char_indices() {
            if i < byte_idx {
                continue;
            }
            if c.is_whitespace() {
                break;
            }
            byte_idx = i + c.len_utf8();
        }
        self.cursor_pos = byte_idx;
    }

    // -----------------------------------------------------------------------
    // Deletion helpers (with kill ring)
    // -----------------------------------------------------------------------

    /// Push deleted text onto the kill ring and mark this as a kill action.
    fn push_kill(&mut self, text: String, backward: bool) {
        if text.is_empty() {
            return;
        }
        self.kill_ring.push(text, backward, self.last_kill_action);
        self.last_kill_action = true;
    }

    /// Reset the kill-accumulation flag after a non-kill action.
    fn reset_kill_accumulation(&mut self) {
        self.last_kill_action = false;
    }

    /// Delete character at cursor (forward).
    fn delete_char_forward(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        let deleted: String = self.input[self.cursor_pos..]
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        if !deleted.is_empty() {
            self.input
                .drain(self.cursor_pos..self.cursor_pos + deleted.len());
            self.push_kill(deleted, false /* forward */);
        }
    }

    /// Delete from cursor to line end.
    fn delete_to_line_end(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        let deleted: String = self.input[self.cursor_pos..].chars().collect();
        self.input.truncate(self.cursor_pos);
        self.push_kill(deleted, false /* forward */);
    }

    /// Delete from line start to cursor.
    fn delete_to_line_start(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let deleted = self.input[..self.cursor_pos].to_string();
        self.input.drain(..self.cursor_pos);
        self.cursor_pos = 0;
        self.push_kill(deleted, true /* backward */);
    }

    /// Delete word backward from cursor.
    fn delete_word_backward(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let old_pos = self.cursor_pos;
        self.move_word_backward();
        let deleted = self.input[self.cursor_pos..old_pos].to_string();
        self.input.drain(self.cursor_pos..old_pos);
        self.push_kill(deleted, true /* backward */);
    }

    /// Delete word forward from cursor.
    fn delete_word_forward(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        let old_pos = self.cursor_pos;
        self.move_word_forward();
        let deleted = self.input[old_pos..self.cursor_pos].to_string();
        self.input.drain(old_pos..self.cursor_pos);
        self.cursor_pos = old_pos;
        self.push_kill(deleted, false /* forward */);
    }

    // -----------------------------------------------------------------------
    // Yank helpers
    // -----------------------------------------------------------------------

    /// Yank (paste) the most recent kill ring entry at cursor.
    fn yank(&mut self) {
        if let Some(text) = self.kill_ring.peek() {
            let text = text.to_string();
            self.last_yank = Some((self.cursor_pos, text.clone()));
            self.input.insert_str(self.cursor_pos, &text);
            self.cursor_pos += text.len();
        }
    }

    /// Yank-pop: rotate kill ring and re-yank.
    ///
    /// Removes the text from the last yank, rotates the ring, and yanks the new entry.
    /// Only valid immediately after yank (cursor must still be at yank end).
    fn yank_pop(&mut self) {
        // Must have yanked something first
        let (yank_start, old_text) = match &self.last_yank {
            Some(pair) => pair,
            None => return,
        };

        // Guard: cursor must still be at the yank end position.
        // If the user typed or moved since yank, the byte offset is stale
        // and draining would corrupt the buffer.
        if self.cursor_pos != *yank_start + old_text.len() {
            return;
        }

        // Need at least 2 entries to rotate meaningfully
        if self.kill_ring.len() < 2 {
            return;
        }

        // Remove the previously yanked text
        self.input.drain(*yank_start..*yank_start + old_text.len());
        self.cursor_pos = *yank_start;

        // Rotate the ring and yank the new entry
        self.kill_ring.rotate();
        self.yank();
    }

    // -----------------------------------------------------------------------
    // History recall
    // -----------------------------------------------------------------------

    fn history_recall_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            self.original_input = self.input.clone();
            self.history_index = Some(self.history.len().saturating_sub(1));
        } else {
            self.history_index = Some(self.history_index.unwrap().saturating_sub(1));
        }
        if let Some(idx) = self.history_index {
            self.input = self.history[idx].clone();
            self.cursor_pos = self.input.len();
        }
    }

    fn history_recall_next(&mut self) {
        match self.history_index {
            None => {}
            Some(idx) => {
                if idx + 1 < self.history.len() {
                    self.history_index = Some(idx + 1);
                    self.input = self.history[idx + 1].clone();
                    self.cursor_pos = self.input.len();
                } else {
                    self.input = self.original_input.clone();
                    self.cursor_pos = self.input.len();
                    self.history_index = None;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Suggestions
    // -----------------------------------------------------------------------

    fn update_suggestions(&mut self) {
        if !self.input.starts_with('/') {
            self.show_suggestions = false;
            self.suggestions.clear();
            self.suggestion_state.select(None);
            return;
        }
        let filtered: Vec<String> = COMMANDS
            .iter()
            .filter(|c| c.starts_with(&self.input))
            .cloned()
            .map(|s| s.to_string())
            .collect();
        self.show_suggestions = !filtered.is_empty();
        self.suggestions = filtered;
        self.suggestion_state
            .select(self.show_suggestions.then_some(0));
    }

    // -----------------------------------------------------------------------
    // History management
    // -----------------------------------------------------------------------

    /// Push text to history.
    pub(crate) fn push_to_history(&mut self, text: &str) {
        self.history.push(text.to_string());
        self.history_index = None;
        self.original_input.clear();
    }

    /// Clear all editor state.
    pub(crate) fn clear_input(&mut self) {
        self.input.clear();
        self.cursor_pos = 0;
        self.suggestions.clear();
        self.show_suggestions = false;
        self.suggestion_state.select(None);
        self.history_index = None;
        self.original_input.clear();
        self.reset_kill_accumulation();
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
