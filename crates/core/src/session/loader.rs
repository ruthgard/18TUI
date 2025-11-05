#![allow(missing_docs)]

use std::{collections::HashMap, fs, path::PathBuf, process::Stdio};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

use crate::{
    models::GameInfo,
    resource::loader::{ensure_game_compatible, extract_module_name},
};

use super::models::{Corporation, GameSession, MarketCell, TrainPoolEntry, TrainType};

const RUBY_SESSION_SCRIPT: &str = r#"
require 'json'

root = ENV.fetch('ENGINE_ROOT')
folder = ENV.fetch('GAME_FOLDER')
module_name = ENV.fetch('GAME_MODULE')

$LOAD_PATH.unshift(File.join(root, 'lib'))
module Kernel
  unless private_method_defined?(:__tui18_original_require)
    alias_method :__tui18_original_require, :require

    def require(path)
      return false if path == 'require_all'
      __tui18_original_require(path)
    rescue LoadError => e
      raise e unless path == 'require_all'
      false
    end
  end
end

module Kernel
  unless method_defined?(:require_all)
    def require_all(path)
      Dir[File.join(path, '**/*.rb')].sort.each { |file| require file }
    end

    def require_rel(path)
      base = caller_locations(1, 1)[0].absolute_path
      dir = File.dirname(base)
      require_all(File.expand_path(File.join(dir, path)))
    end
  end
end
begin
  require 'require_all'
rescue LoadError
  # swallow; shim above handles functionality
end
require File.join(root, 'lib', 'engine.rb')

base_path = File.expand_path(File.join(root, 'lib', 'engine', 'game', 'base.rb'))
require base_path if File.exist?(base_path)

entities_path = File.expand_path(File.join(root, 'lib', 'engine', 'game', folder, 'entities.rb'))
game_path = File.expand_path(File.join(root, 'lib', 'engine', 'game', folder, 'game.rb'))

unless Engine::Game.const_defined?(module_name.to_sym)
  load entities_path if File.exist?(entities_path)
  load game_path if File.exist?(game_path)
end

def convert(obj)
  case obj
  when Hash
    obj.each_with_object({}) do |(key, value), acc|
      acc[key.to_s] = convert(value)
    end
  when Array
    obj.map { |value| convert(value) }
  else
    obj
  end
end

game_module = Engine::Game.const_get(module_name)

data = {
  'corporations' => convert(game_module::Entities::CORPORATIONS),
  'market' => convert(game_module::Game::MARKET),
  'trains' => convert(game_module::Game::TRAINS),
  'phases' => convert(game_module::Game::PHASES)
}

puts JSON.dump(data)
"#;

/// Loads fully-detailed game sessions by delegating metadata extraction to Ruby.
#[derive(Debug, Clone)]
pub struct SessionLoader {
    root_path: PathBuf,
}

impl SessionLoader {
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
        }
    }

    pub fn with_root(&mut self, root_path: impl Into<PathBuf>) {
        self.root_path = root_path.into();
    }

    pub async fn load(&self, info: &GameInfo) -> Result<GameSession> {
        let base_path = self
            .root_path
            .join("lib")
            .join("engine")
            .join("game")
            .join(&info.folder);
        if !base_path.exists() {
            return Err(anyhow!("game directory missing: {}", base_path.display()));
        }

        let meta_path = base_path.join("meta.rb");
        match ensure_game_compatible(&meta_path)? {
            Ok(_) => {}
            Err(reason) => return Err(anyhow!("{}", reason)),
        }

        let entities_path = base_path.join("entities.rb");
        let entities_content = fs::read_to_string(&entities_path)
            .with_context(|| format!("failed to read {}", entities_path.display()))?;
        let module_name = extract_module_name(&entities_content)
            .ok_or_else(|| anyhow!("unable to determine module name for {}", info.folder))?;

        let raw = self.fetch_raw_session(&info.folder, &module_name).await?;
        let session = self.build_session(info.clone(), &raw)?;
        Ok(session)
    }

    async fn fetch_raw_session(&self, folder: &str, module: &str) -> Result<RawSession> {
        let mut command = Command::new("ruby");
        command.arg("-e").arg(RUBY_SESSION_SCRIPT);
        command
            .env("ENGINE_ROOT", &self.root_path)
            .env("GAME_FOLDER", folder)
            .env("GAME_MODULE", module)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&self.root_path);

        let output = command
            .output()
            .await
            .context("failed to execute ruby session extractor")?;

        if !output.status.success() {
            return Err(anyhow!(
                "ruby session extractor failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let decoded: RawSession = serde_json::from_slice(&output.stdout)
            .context("failed to parse session payload from ruby")?;
        Ok(decoded)
    }

    fn build_session(&self, info: GameInfo, raw: &RawSession) -> Result<GameSession> {
        let corporations = raw
            .corporations
            .iter()
            .map(|corp| {
                Corporation::new(
                    corp.sym.clone().unwrap_or_else(|| "?".to_string()),
                    corp.name
                        .clone()
                        .unwrap_or_else(|| corp.sym.clone().unwrap_or_else(|| "?".to_string())),
                    normalize_color(corp.color.clone()),
                    normalize_color(corp.text_color.clone()),
                )
            })
            .collect::<Vec<_>>();

        let market = normalize_market(&raw.market);
        let market_cells = collect_market_cells(&market);
        let market_index = market_cells
            .iter()
            .cloned()
            .map(|cell| ((cell.row, cell.col), cell))
            .collect::<HashMap<_, _>>();
        let par_cells = available_par_cells_from(&market_cells);

        let train_types = raw
            .trains
            .iter()
            .map(|train| TrainType {
                name: train.name.clone().unwrap_or_else(|| "?".to_string()),
                distance: train.distance.clone().unwrap_or(Value::Null),
                price: train.price,
                total: train.num.unwrap_or(0),
                rusts_on: train.rusts_on.clone().unwrap_or(Value::Null),
                obsolete_on: train.obsolete_on.clone().unwrap_or(Value::Null),
            })
            .collect::<Vec<_>>();

        let train_pool = train_types
            .iter()
            .map(|train| TrainPoolEntry {
                name: train.name.clone(),
                remaining: train.total,
            })
            .collect();

        let phases = raw.phases.clone();

        Ok(GameSession {
            info,
            corporations,
            market,
            market_cells,
            market_index,
            par_cells,
            train_types,
            train_pool,
            phases,
            loaded_at: Utc::now(),
        })
    }
}

fn normalize_color(input: Option<String>) -> Option<String> {
    input
        .map(|value| value.trim().trim_start_matches(':').to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_market(raw: &[Vec<Value>]) -> Vec<Vec<String>> {
    raw.iter()
        .map(|row| row.iter().map(value_to_string).collect())
        .collect()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(num) => num.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(value_to_string)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "*".to_string(),
    }
}

fn collect_market_cells(rows: &[Vec<String>]) -> Vec<MarketCell> {
    let mut cells = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (col_index, raw) in row.iter().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let numeric = RAW_NUMBER_RE
                .captures(raw)
                .and_then(|cap| cap.get(1))
                .and_then(|m| m.as_str().parse::<i32>().ok());
            let is_par = raw.contains('p') || raw.contains('P');
            cells.push(MarketCell {
                row: row_index,
                col: col_index,
                value: numeric,
                raw: raw.clone(),
                is_par,
            });
        }
    }
    cells
}

fn available_par_cells_from(cells: &[MarketCell]) -> Vec<MarketCell> {
    let par_cells: Vec<MarketCell> = cells.iter().cloned().filter(|cell| cell.is_par).collect();
    if par_cells.is_empty() {
        cells.to_vec()
    } else {
        par_cells
    }
}

static RAW_NUMBER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\d+)").expect("failed to compile market numeric regex"));

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn loads_session_from_ruby() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path();
        let game_dir = root.join("lib/engine/game/g_sample");
        fs::create_dir_all(&game_dir)?;

        fs::create_dir_all(root.join("lib"))?;
        // Minimal engine stub
        fs::write(
            root.join("lib/engine.rb"),
            r#"
module Engine
  module Game
  end
end
"#,
        )?;

        fs::write(
            game_dir.join("entities.rb"),
            r#"
module Engine
  module Game
    module G18Sample
      module Entities
        CORPORATIONS = [
          { sym: 'A', name: 'Alpha', color: 'red', text_color: 'white' }
        ]
      end
    end
  end
end
"#,
        )?;

        fs::write(
            game_dir.join("game.rb"),
            r#"
module Engine
  module Game
    module G18Sample
      module Game
        MARKET = [['100', '110p']]
        TRAINS = [
          { name: '2', distance: 2, price: 100, num: 3, rusts_on: '4', obsolete_on: '5' }
        ]
        PHASES = [
          { name: '2', train_limit: 4 }
        ]
      end
    end
  end
end
"#,
        )?;

        fs::write(
            game_dir.join("meta.rb"),
            r#"
GAME_TITLE = "Sample"
GAME_SUBTITLE = "Test"
GAME_DESIGNER = "Designer"
GAME_LOCATION = "Somewhere"
GAME_RULES_URL = "https://example.com"
"#,
        )?;

        let loader = SessionLoader::new(root);
        let info = GameInfo {
            id: "sample".to_string(),
            title: "Sample".to_string(),
            subtitle: Some("Test".to_string()),
            folder: "g_sample".to_string(),
            designer: Some("Designer".to_string()),
            location: Some("Somewhere".to_string()),
            rules_url: Some("https://example.com".to_string()),
            commit: None,
            updated_at: None,
        };

        let session = loader.load(&info).await?;
        assert_eq!(session.corporations.len(), 1);
        assert_eq!(session.market.len(), 1);
        assert_eq!(session.train_types.len(), 1);
        assert_eq!(session.par_cells.len(), 1);
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct RawSession {
    #[serde(default)]
    corporations: Vec<RawCorporation>,
    #[serde(default)]
    market: Vec<Vec<Value>>,
    #[serde(default)]
    trains: Vec<RawTrain>,
    #[serde(default)]
    phases: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawCorporation {
    sym: Option<String>,
    name: Option<String>,
    color: Option<String>,
    text_color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTrain {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    distance: Option<Value>,
    #[serde(default)]
    price: Option<i64>,
    #[serde(alias = "num", default)]
    num: Option<i64>,
    #[serde(default)]
    rusts_on: Option<Value>,
    #[serde(default)]
    obsolete_on: Option<Value>,
}
