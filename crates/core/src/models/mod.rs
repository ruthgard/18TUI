//! Shared domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata extracted from a game's `meta.rb`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    /// Short identifier (e.g. `1889`).
    pub id: String,
    /// Human-readable game title.
    pub title: String,
    /// Optional subtitle/tagline.
    pub subtitle: Option<String>,
    /// Directory name under the engine `game` folder.
    pub folder: String,
    /// Game designer credit.
    pub designer: Option<String>,
    /// Geographic or thematic location.
    pub location: Option<String>,
    /// Link to the rulebook, if available.
    pub rules_url: Option<String>,
    /// Commit hash of the engine snapshot where metadata was read.
    pub commit: Option<String>,
    /// Timestamp of last update for the snapshot.
    pub updated_at: Option<DateTime<Utc>>,
}

impl GameInfo {
    /// Returns a user-facing label combining title and subtitle.
    pub fn display_name(&self) -> String {
        match self.subtitle.as_deref() {
            Some(subtitle) if !subtitle.is_empty() => format!("{} · {}", self.title, subtitle),
            _ => self.title.clone(),
        }
    }
}
