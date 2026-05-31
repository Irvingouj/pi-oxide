use crate::app::App;

impl App {
    pub(crate) fn save_session(&self) {
        if let Some(ref id) = self.session_id {
            if let Some(ref host_state) = self.host_state {
                let data = host_state.get_persist_data(
                    &self.transcript,
                    &self.artifacts,
                    self.turn_number,
                    &self.budget,
                );
                if let Err(e) = self.session_backend.save(id, &data) {
                    tracing::warn!(session_id = id.as_str(), error = ?e, "failed to save session");
                }
            }
        }
    }
}
