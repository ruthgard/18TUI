//! Save-game persistence scaffolding.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::models::GameInfo;

/// Root directory under `~/.config` used for save files.
pub const DEFAULT_SAVE_DIR: &str = "18tui/saves";

/// Metadata describing a persisted session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveEntry {
    /// Absolute path to the save file on disk.
    pub path: PathBuf,
    /// Identifier of the game associated with the save.
    pub game_id: String,
    /// Human readable save name.
    pub name: String,
    /// Timestamp when the save was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Serialized representation of a save file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavePayload {
    game_id: String,
    name: String,
    saved_at: DateTime<Utc>,
    #[serde(default)]
    state: Value,
    #[serde(default)]
    history: Vec<Value>,
    #[serde(default)]
    history_index: usize,
}

impl SavePayload {
    fn new(game: &GameInfo, name: Option<&str>, state: Value) -> Self {
        let saved_at = Utc::now();
        let display_name = name
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| game.title.clone());
        let mut payload = Self {
            game_id: game.id.clone(),
            name: display_name,
            saved_at,
            state,
            history: Vec::new(),
            history_index: 0,
        };
        payload.normalize_history();
        payload
    }

    /// Consume the payload and return the stored game state.
    pub fn into_state(self) -> Value {
        self.state
    }

    /// Borrow the stored game state without consuming the payload.
    pub fn state(&self) -> &Value {
        &self.state
    }

    /// Return the total number of recorded history snapshots stored for this save.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Index of the currently active history snapshot.
    pub fn history_index(&self) -> usize {
        self.history_index
    }

    fn normalize_history(&mut self) {
        if self.history.is_empty() {
            self.history.push(self.state.clone());
            self.history_index = self.history.len().saturating_sub(1);
        } else if self.history_index >= self.history.len() {
            self.history_index = self.history.len().saturating_sub(1);
        }
        if let Some(current) = self.history.get(self.history_index).cloned() {
            self.state = current;
        }
    }

    fn push_state(&mut self, state: Value) {
        self.normalize_history();
        if self.history_index + 1 < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        if self
            .history
            .last()
            .map(|value| value == &state)
            .unwrap_or(false)
        {
            return;
        }
        self.history.push(state.clone());
        self.history_index = self.history.len() - 1;
        self.state = state;
        self.saved_at = Utc::now();
    }

    fn set_history_index(&mut self, index: usize) -> Result<()> {
        self.normalize_history();
        if index >= self.history.len() {
            return Err(anyhow!("history index {index} out of range"));
        }
        self.history_index = index;
        if let Some(current) = self.history.get(index).cloned() {
            self.state = current;
        }
        self.saved_at = Utc::now();
        Ok(())
    }
}

/// Manager responsible for loading and writing save files.
pub struct SaveManager {
    root: PathBuf,
}

impl SaveManager {
    /// Create a new manager rooted at the provided directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Default location under the user's config directory.
    pub fn default_root() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(DEFAULT_SAVE_DIR)
    }

    /// Return all known saves sorted by timestamp (most recent first).
    pub fn entries(&self) -> Result<Vec<SaveEntry>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root).context("failed to read save directory")? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            match self.read_payload(entry.path()) {
                Ok(payload) => entries.push(SaveEntry {
                    path: entry.path(),
                    game_id: payload.game_id,
                    name: payload.name,
                    updated_at: payload.saved_at,
                }),
                Err(err) => {
                    warn!("Failed to read save {:?}: {err}", entry.path());
                }
            }
        }

        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(entries)
    }

    /// Save the current game selection to disk and return the resulting entry.
    pub fn create_save(
        &self,
        game: &GameInfo,
        name: Option<&str>,
        state: Value,
    ) -> Result<SaveEntry> {
        if let Some(parent) = self.root.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;

        let payload = SavePayload::new(game, name, state);
        let file_name = format!(
            "{}_{}.json",
            sanitize_component(&payload.game_id),
            payload.saved_at.format("%Y%m%d%H%M%S")
        );
        let path = self.root.join(file_name);
        self.write_payload(&path, &payload)?;

        Ok(SaveEntry {
            path,
            game_id: payload.game_id,
            name: payload.name,
            updated_at: payload.saved_at,
        })
    }

    /// Persist the current selection without any additional state payload.
    pub fn save_selection(&self, game: &GameInfo, name: Option<&str>) -> Result<SaveEntry> {
        self.create_save(game, name, Value::Null)
    }

    /// Overwrite an existing save with updated state while refreshing metadata.
    pub fn update_save(&self, entry: &SaveEntry, state: Value) -> Result<SaveEntry> {
        let mut payload = self.read_payload(&entry.path)?;
        payload.push_state(state);
        self.write_payload(&entry.path, &payload)?;
        Ok(SaveEntry {
            path: entry.path.clone(),
            game_id: payload.game_id,
            name: payload.name,
            updated_at: payload.saved_at,
        })
    }

    /// Load payload for the provided entry.
    pub fn load(&self, entry: &SaveEntry) -> Result<SavePayload> {
        let mut payload = self.read_payload(&entry.path)?;
        payload.normalize_history();
        Ok(payload)
    }

    /// Load most recent save entry, if any.
    pub fn latest(&self) -> Result<Option<SaveEntry>> {
        let entries = self.entries()?;
        Ok(entries.into_iter().next())
    }

    /// Persist an updated payload back to disk.
    pub fn persist_payload(&self, entry: &SaveEntry, payload: &SavePayload) -> Result<SaveEntry> {
        self.write_payload(&entry.path, payload)?;
        Ok(SaveEntry {
            path: entry.path.clone(),
            game_id: payload.game_id.clone(),
            name: payload.name.clone(),
            updated_at: payload.saved_at,
        })
    }

    /// Adjust the active history index without mutating the history contents.
    pub fn set_history_index(
        &self,
        entry: &SaveEntry,
        index: usize,
    ) -> Result<(SaveEntry, SavePayload)> {
        let mut payload = self.read_payload(&entry.path)?;
        payload.normalize_history();
        payload
            .set_history_index(index)
            .with_context(|| format!("failed to set history index to {index}"))?;
        self.write_payload(&entry.path, &payload)?;
        let updated_entry = SaveEntry {
            path: entry.path.clone(),
            game_id: payload.game_id.clone(),
            name: payload.name.clone(),
            updated_at: payload.saved_at,
        };
        Ok((updated_entry, payload))
    }

    fn write_payload(&self, path: &Path, payload: &SavePayload) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let serialised = serde_json::to_vec_pretty(payload)?;
        fs::write(path, serialised).with_context(|| format!("failed to write {}", path.display()))
    }

    fn read_payload(&self, path: impl AsRef<Path>) -> Result<SavePayload> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let payload = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(payload)
    }
}

fn sanitize_component(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            result.push(ch);
        }
    }
    if result.is_empty() {
        "save".to_string()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn sample_game() -> GameInfo {
        GameInfo {
            id: "sample".to_string(),
            title: "Sample".to_string(),
            subtitle: Some("Test".to_string()),
            folder: "g_sample".to_string(),
            designer: Some("Designer".to_string()),
            location: Some("Somewhere".to_string()),
            rules_url: Some("https://example.com".to_string()),
            commit: Some("abc1234".to_string()),
            updated_at: Some(Utc::now()),
        }
    }

    #[test]
    fn save_round_trip() -> Result<()> {
        let dir = tempdir()?;
        let manager = SaveManager::new(dir.path());
        let game = sample_game();

        let entry = manager.create_save(&game, Some("First Save"), json!({"state": "initial"}))?;
        assert!(entry.path.exists());

        let entries = manager.entries()?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].game_id, "sample");
        assert_eq!(entries[0].name, "First Save".to_string());

        let payload = manager.load(&entries[0])?;
        assert_eq!(payload.game_id, "sample");
        assert_eq!(payload.name, "First Save");
        assert_eq!(payload.state()["state"], json!("initial"));
        assert_eq!(payload.history_len(), 1);
        assert_eq!(payload.history_index(), 0);

        let updated = manager.update_save(&entries[0], json!({"state": "updated"}))?;
        assert!(updated.updated_at > entries[0].updated_at);
        let payload = manager.load(&updated)?;
        assert_eq!(payload.state()["state"], json!("updated"));
        assert_eq!(payload.history_len(), 2);
        assert_eq!(payload.history_index(), 1);

        let (_, reverted) = manager.set_history_index(&updated, 0)?;
        assert_eq!(reverted.state()["state"], json!("initial"));

        let latest = manager.latest()?.expect("expected latest entry");
        assert_eq!(latest.game_id, "sample");

        Ok(())
    }

    #[test]
    fn sanitize_creates_safe_filenames() {
        let name = sanitize_component("Hello World!* 18??");
        assert_eq!(name, "HelloWorld18");
    }
}
