//! Model picker — filters and selects from a list of available models.

pub struct ModelPicker {
    models: Vec<String>,
    current_model: String,
    filter: String,
    selected_index: usize,
}

impl ModelPicker {
    pub fn new(models: Vec<String>, current_model: String) -> Self {
        Self {
            models,
            current_model,
            filter: String::new(),
            selected_index: 0,
        }
    }

    /// Return models matching the current filter.
    pub fn filtered(&self) -> Vec<&str> {
        self.models
            .iter()
            .map(|s| s.as_str())
            .filter(|m| {
                self.filter.is_empty() || m.to_lowercase().contains(&self.filter.to_lowercase())
            })
            .collect()
    }

    /// Return the currently selected model, if any.
    pub fn selected(&self) -> Option<&str> {
        let filtered = self.filtered();
        filtered.get(self.selected_index).copied()
    }

    /// Move selection down. Wraps to top.
    pub fn select_next(&mut self) {
        let count = self.filtered().len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    /// Move selection up. Wraps to bottom.
    pub fn select_previous(&mut self) {
        let count = self.filtered().len();
        if count > 1 {
            self.selected_index = if self.selected_index == 0 {
                count - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    /// Append a character to the filter.
    pub fn append_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected_index = 0; // reset to top on filter change
    }

    /// Remove last character from filter.
    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected_index = 0; // reset to top on filter change
    }

    /// Confirm selection. Returns the selected model ID, or None if empty.
    pub fn confirm(&mut self) -> Option<String> {
        self.selected().map(|s| s.to_string())
    }

    /// Return the current filter text.
    pub fn filter_text(&self) -> &str {
        &self.filter
    }

    /// Return the total number of models.
    pub fn total_count(&self) -> usize {
        self.models.len()
    }

    /// Return the number of filtered results.
    #[allow(dead_code)]
    pub fn filtered_count(&self) -> usize {
        self.filtered().len()
    }

    /// Return the current model ID (for display).
    pub fn current_model(&self) -> &str {
        &self.current_model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_picker() -> ModelPicker {
        ModelPicker::new(
            vec![
                "deepseek-v4-pro".into(),
                "deepseek-chat".into(),
                "deepseek-reasoner".into(),
                "gpt-5.5".into(),
            ],
            "deepseek-v4-pro".into(),
        )
    }

    #[test]
    fn new_shows_all_models_when_no_filter() {
        let picker = make_picker();
        assert_eq!(picker.filtered_count(), 4);
        assert_eq!(
            picker.filtered(),
            vec![
                "deepseek-v4-pro",
                "deepseek-chat",
                "deepseek-reasoner",
                "gpt-5.5"
            ]
        );
    }

    #[test]
    fn filter_narrows_to_matching_models() {
        let mut picker = make_picker();
        picker.append_char('c');
        picker.append_char('h');
        picker.append_char('a');
        picker.append_char('t');
        assert_eq!(picker.filtered(), vec!["deepseek-chat"]);
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut picker = make_picker();
        picker.append_char('D');
        picker.append_char('E');
        picker.append_char('E');
        assert_eq!(picker.filtered_count(), 3); // all deepseek
    }

    #[test]
    fn filter_with_no_match_returns_empty() {
        let mut picker = make_picker();
        picker.append_char('x');
        picker.append_char('y');
        picker.append_char('z');
        assert!(picker.filtered().is_empty());
        assert_eq!(picker.selected(), None);
    }

    #[test]
    fn select_next_cycles_through_filtered() {
        let mut picker = make_picker();
        assert_eq!(picker.selected(), Some("deepseek-v4-pro"));
        picker.select_next();
        assert_eq!(picker.selected(), Some("deepseek-chat"));
        picker.select_next();
        assert_eq!(picker.selected(), Some("deepseek-reasoner"));
        picker.select_next();
        assert_eq!(picker.selected(), Some("gpt-5.5"));
        // wraps to top
        picker.select_next();
        assert_eq!(picker.selected(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn select_previous_wraps_to_bottom() {
        let mut picker = make_picker();
        picker.select_previous();
        assert_eq!(picker.selected(), Some("gpt-5.5"));
        picker.select_previous();
        assert_eq!(picker.selected(), Some("deepseek-reasoner"));
    }

    #[test]
    fn backspace_clears_filter() {
        let mut picker = make_picker();
        picker.append_char('g');
        picker.append_char('p');
        assert_eq!(picker.filtered_count(), 1); // gpt-5.5
        picker.backspace();
        assert_eq!(picker.filtered_count(), 1); // "g" still only gpt-5.5
        picker.backspace();
        assert_eq!(picker.filtered_count(), 4);
    }

    #[test]
    fn confirm_returns_selected_model() {
        let mut picker = make_picker();
        picker.select_next();
        let model = picker.confirm();
        assert_eq!(model, Some("deepseek-chat".into()));
    }

    #[test]
    fn confirm_returns_none_when_no_match() {
        let mut picker = make_picker();
        picker.append_char('z');
        assert_eq!(picker.confirm(), None);
    }

    #[test]
    fn filter_resets_selection_to_top() {
        let mut picker = make_picker();
        picker.select_next();
        picker.select_next();
        assert_eq!(picker.selected(), Some("deepseek-reasoner"));
        picker.append_char('v'); // filters to deepseek-v4-pro
        assert_eq!(picker.selected(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn empty_model_list() {
        let mut picker = ModelPicker::new(vec![], "unknown".into());
        assert!(picker.filtered().is_empty());
        assert_eq!(picker.selected(), None);
        assert_eq!(picker.confirm(), None);
    }
}
