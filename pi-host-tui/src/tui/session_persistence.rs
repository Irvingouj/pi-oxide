use crate::app::App;

impl App {
    pub(crate) fn save_session(&self) {
        if let Some(ref id) = self.session_id {
            let data = self.host_state.as_ref().unwrap().get_persist_data();
            if let Err(e) = self.session_backend.save(id, &data) {
                tracing::warn!(session_id = id.as_str(), error = ?e, "failed to save session");
            }
        }
    }
}
