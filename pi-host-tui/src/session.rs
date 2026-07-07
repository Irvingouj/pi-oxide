//! Filesystem-based session persistence for the terminal agent.
//!
//! Stores PersistData as pretty-printed JSON in ~/.pi-oxide/sessions/{id}.json.

use std::path::PathBuf;

use crate::host_state::PersistData;

pub struct FileSystemSessionBackend {
    dir: PathBuf,
}

impl FileSystemSessionBackend {
    pub fn new() -> Self {
        let dir = crate::config::home_dir().join(".pi-oxide").join("sessions");
        std::fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    /// Load a persisted host state from disk.
    pub fn load(&self, session_id: &str) -> Option<PersistData> {
        let path = self.path_for(session_id);
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Save a persisted host state to disk.
    pub fn save(
        &self,
        session_id: &str,
        state: &PersistData,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.path_for(session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(state)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// List all saved session IDs.
    pub fn list(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(id) = name.strip_suffix(".json") {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        ids.sort();
        ids
    }

    pub fn path_for(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", session_id))
    }
}
