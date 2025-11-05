//! Resource manifest stored alongside the cloned engine.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata describing the currently active engine snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceMetadata {
    /// Commit hash of the engine repo.
    pub commit: Option<String>,
    /// ISO8601 timestamp for the snapshot.
    pub updated_at: Option<DateTime<Utc>>,
}

impl ResourceMetadata {
    /// Load metadata from the given path, returning `None` if it does not exist.
    pub fn load(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest {}", path.display()))?;
        let metadata = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse manifest {}", path.display()))?;
        Ok(metadata)
    }

    /// Persist metadata to the given file, creating parent directories if needed.
    pub fn persist(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create manifest directory {}", parent.display())
            })?;
        }

        let serialized =
            serde_json::to_string_pretty(self).context("failed to serialize resource metadata")?;
        fs::write(path, serialized)
            .with_context(|| format!("failed to write manifest {}", path.display()))
    }
}

/// Helper to compute the default manifest path inside a repo directory.
pub fn manifest_path(repo_path: impl AsRef<Path>) -> PathBuf {
    repo_path.as_ref().join(".18tui-manifest.json")
}
