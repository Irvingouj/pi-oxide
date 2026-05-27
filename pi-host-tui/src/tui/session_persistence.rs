use crate::app::App;

impl App {
    pub(crate) fn save_session(&self) {
        if let Some(ref id) = self.session_id {
            let state = self.agent().session_state();
            if let Err(e) = self.session_backend.save(id, state) {
                tracing::warn!(session_id = id.as_str(), error = ?e, "failed to save session");
            }
        }
    }
}
