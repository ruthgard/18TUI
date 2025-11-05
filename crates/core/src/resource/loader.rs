use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use regex::Regex;
use tracing::warn;

use crate::{manifest::ResourceMetadata, models::GameInfo};

/// Thread-safe resource loader that discovers games from an engine checkout.
pub struct ResourceLoader {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    root_path: PathBuf,
    metadata: ResourceMetadata,
    cache: Vec<GameInfo>,
}

impl ResourceLoader {
    /// Build a new loader rooted at the given engine directory.
    pub fn new(root_path: impl Into<PathBuf>, metadata: ResourceMetadata) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                root_path: root_path.into(),
                metadata,
                cache: Vec::new(),
            })),
        }
    }

    /// Current manifest metadata for the loaded resources.
    pub fn metadata(&self) -> ResourceMetadata {
        self.inner.read().metadata.clone()
    }

    /// Root path of the engine resources.
    pub fn root_path(&self) -> PathBuf {
        self.inner.read().root_path.clone()
    }

    /// Update the loader to point at a new engine directory with updated metadata.
    pub fn refresh(&self, root_path: impl Into<PathBuf>, metadata: ResourceMetadata) {
        let mut inner = self.inner.write();
        inner.root_path = root_path.into();
        inner.metadata = metadata;
        inner.cache.clear();
    }

    /// Return all known games, populating the cache on first use.
    pub fn games(&self) -> Result<Vec<GameInfo>> {
        let mut inner = self.inner.write();
        if inner.cache.is_empty() {
            inner.cache = discover_games(&inner.root_path, &inner.metadata)?;
        }
        Ok(inner.cache.clone())
    }

    /// Filter games using a case-insensitive substring search.
    pub fn games_matching(&self, query: &str) -> Result<Vec<GameInfo>> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return self.games();
        }

        let games = self.games()?;
        Ok(games
            .into_iter()
            .filter(|game| {
                game.id.to_lowercase().contains(&needle)
                    || game.title.to_lowercase().contains(&needle)
                    || game
                        .subtitle
                        .as_ref()
                        .map(|value| value.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                    || game
                        .designer
                        .as_ref()
                        .map(|value| value.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                    || game
                        .location
                        .as_ref()
                        .map(|value| value.to_lowercase().contains(&needle))
                        .unwrap_or(false)
            })
            .collect())
    }
}

/// Public helper used by tests and future tooling.
pub struct GameDiscovery;

impl GameDiscovery {
    /// Enumerate games beneath `root_path`. Currently returns an empty list.
    ///
    pub fn discover(
        root_path: impl Into<PathBuf>,
        metadata: &ResourceMetadata,
    ) -> Result<Vec<GameInfo>> {
        discover_games(&root_path.into(), metadata)
    }
}

fn discover_games(root: &PathBuf, metadata: &ResourceMetadata) -> Result<Vec<GameInfo>> {
    let game_root = root.join("lib").join("engine").join("game");
    if !game_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut games = Vec::new();
    let mut folders: Vec<_> = fs::read_dir(&game_root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .collect();

    folders.sort_by_key(|entry| entry.file_name());

    for entry in folders {
        let folder_name = entry.file_name().to_string_lossy().to_string();
        if !folder_name.starts_with("g_") {
            continue;
        }

        let meta_path = entry.path().join("meta.rb");
        if !meta_path.is_file() {
            warn!("Skipping {} – missing meta.rb", folder_name);
            continue;
        }

        match build_game(&meta_path, metadata) {
            Ok(Some(game)) => games.push(game),
            Ok(None) => continue,
            Err(err) => warn!("Skipping {}: {}", folder_name, err),
        }
    }

    Ok(games)
}

fn build_game(meta_path: &Path, metadata: &ResourceMetadata) -> Result<Option<GameInfo>> {
    let folder = meta_path
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("failed to determine folder for {:?}", meta_path))?
        .to_string();

    let compatibility = ensure_game_compatible(meta_path)?;
    if let Err(reason) = compatibility {
        warn!("Skipping {}: {}", folder, reason);
        return Ok(None);
    }

    let content = fs::read_to_string(meta_path)
        .with_context(|| format!("failed to read {}", meta_path.display()))?;
    let id = folder.trim_start_matches("g_").to_string();

    let title = extract_constant(&content, "GAME_TITLE").unwrap_or_else(|| id.to_uppercase());
    let subtitle = extract_constant(&content, "GAME_SUBTITLE");
    let designer = extract_constant(&content, "GAME_DESIGNER");
    let location = extract_constant(&content, "GAME_LOCATION");
    let rules_url = extract_constant(&content, "GAME_RULES_URL");

    Ok(Some(GameInfo {
        id,
        title: title.trim().to_string(),
        subtitle: subtitle
            .map(|value| value.trim().to_string())
            .filter(|s| !s.is_empty()),
        folder,
        designer: designer
            .map(|value| value.trim().to_string())
            .filter(|s| !s.is_empty()),
        location: location
            .map(|value| value.trim().to_string())
            .filter(|s| !s.is_empty()),
        rules_url: rules_url
            .map(|value| value.trim().to_string())
            .filter(|s| !s.is_empty()),
        commit: metadata.commit.clone(),
        updated_at: metadata.updated_at.clone(),
    }))
}

fn extract_constant(content: &str, name: &str) -> Option<String> {
    let quoted_pattern = Regex::new(&format!(
        r#"(?ms)^\s*{}\s*=\s*(?:"([^"]+)"|'([^']+)')"#,
        regex::escape(name)
    ))
    .ok()?;
    if let Some(caps) = quoted_pattern.captures(content) {
        return caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str().to_string());
    }

    let heredoc_pattern = Regex::new(&format!(
        r#"(?m)^\s*{}\s*=\s*<<-?(\w+)\s*$"#,
        regex::escape(name)
    ))
    .ok()?;
    if let Some(caps) = heredoc_pattern.captures(content) {
        let tag = caps.get(1)?.as_str();
        let start = caps.get(0)?.end();
        let after = &content[start..];
        let mut collected = Vec::new();
        for line in after.lines() {
            if line.trim() == tag {
                return Some(collected.join("\n"));
            }
            collected.push(line.to_string());
        }
    }

    None
}

pub(crate) fn ensure_game_compatible(meta_path: &Path) -> Result<Result<(), String>> {
    let folder = meta_path.parent().map(Path::to_path_buf).ok_or_else(|| {
        anyhow::anyhow!(
            "failed to determine parent directory for {}",
            meta_path.display()
        )
    })?;
    let entities = folder.join("entities.rb");
    let game_file = folder.join("game.rb");

    if !entities.is_file() {
        return Ok(Err(format!(
            "missing entities.rb at {}",
            entities.display()
        )));
    }
    if !game_file.is_file() {
        return Ok(Err(format!("missing game.rb at {}", game_file.display())));
    }

    let entities_content = fs::read_to_string(&entities)
        .with_context(|| format!("failed to read {}", entities.display()))?;
    let game_content = fs::read_to_string(&game_file)
        .with_context(|| format!("failed to read {}", game_file.display()))?;

    let module_name = extract_module_name(&entities_content).ok_or_else(|| {
        anyhow::anyhow!(
            "unable to determine module name from {}",
            entities.display()
        )
    })?;

    if !entities_content.contains("module Entities") {
        return Ok(Err("Entities module missing".to_string()));
    }
    if !entities_content.contains("CORPORATIONS") {
        return Ok(Err("corporation data missing".to_string()));
    }
    if !game_content.contains("module Game") {
        return Ok(Err("Game module missing".to_string()));
    }
    if !game_content.contains("MARKET") {
        return Ok(Err("MARKET data missing".to_string()));
    }
    if !game_content.contains("TRAINS") {
        return Ok(Err("TRAINS data missing".to_string()));
    }

    // Basic check to ensure module reference is consistent between entities and game.
    if !game_content.contains(&module_name) {
        return Ok(Err(format!(
            "module {} not referenced in game.rb",
            module_name
        )));
    }

    Ok(Ok(()))
}

pub(crate) fn extract_module_name(content: &str) -> Option<String> {
    static MODULE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"module\s+(G\d[0-9A-Za-z_]*)").expect("invalid module regex"));

    MODULE_RE
        .captures_iter(content)
        .last()
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn discovers_valid_games() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path();
        let game_dir = root.join("lib/engine/game/g_sample");
        fs::create_dir_all(&game_dir)?;

        fs::write(
            game_dir.join("meta.rb"),
            r#"GAME_TITLE = "Sample"
GAME_SUBTITLE = "Test"
GAME_DESIGNER = "Designer"
GAME_LOCATION = "Somewhere"
GAME_RULES_URL = "https://example.com"
"#,
        )?;

        fs::write(
            game_dir.join("entities.rb"),
            r#"
module G18Sample
  module Entities
    CORPORATIONS = [{ sym: 'A', name: 'Alpha' }]
  end
end
"#,
        )?;

        fs::write(
            game_dir.join("game.rb"),
            r#"
module G18Sample
  module Game
    MARKET = [["100"]]
    TRAINS = [{ name: '2', distance: 2 }]
    PHASES = [{ name: 'I' }]
  end
end
"#,
        )?;

        // Add an incompatible game to ensure it is skipped
        let invalid_dir = root.join("lib/engine/game/g_invalid");
        fs::create_dir_all(&invalid_dir)?;
        fs::write(invalid_dir.join("meta.rb"), "GAME_TITLE = \"Broken\"")?;
        fs::write(
            invalid_dir.join("entities.rb"),
            "module GInvalid\n  module Entities\n  end\nend\n",
        )?;

        let meta_path = game_dir.join("meta.rb");
        let compatibility = ensure_game_compatible(&meta_path)?;
        assert!(compatibility.is_ok(), "compatibility check failed");
        let meta_content = fs::read_to_string(&meta_path)?;
        assert_eq!(
            extract_constant(&meta_content, "GAME_TITLE").as_deref(),
            Some("Sample")
        );

        let metadata = ResourceMetadata {
            commit: Some("abc1234".to_string()),
            updated_at: Some(Utc::now()),
        };

        let games = GameDiscovery::discover(root, &metadata)?;
        assert_eq!(games.len(), 1);
        let game = &games[0];
        assert_eq!(game.id, "sample");
        assert_eq!(game.title, "Sample");
        assert_eq!(game.subtitle.as_deref(), Some("Test"));
        assert_eq!(game.designer.as_deref(), Some("Designer"));
        assert_eq!(game.location.as_deref(), Some("Somewhere"));
        assert_eq!(game.rules_url.as_deref(), Some("https://example.com"));
        assert_eq!(game.commit.as_deref(), Some("abc1234"));

        Ok(())
    }
}
