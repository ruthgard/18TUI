#![allow(missing_docs)]

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::de;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::models::GameInfo;

/// Corporation data mirrored from the Ruby UI with runtime state fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Corporation {
    pub sym: String,
    pub name: String,
    pub color: Option<String>,
    pub text_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub par_value: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_position: Option<MarketPosition>,
    #[serde(default)]
    pub trains: Vec<CorporationTrain>,
    #[serde(default)]
    pub last_revenue: i32,
}

impl Corporation {
    pub fn new(
        sym: String,
        name: String,
        color: Option<String>,
        text_color: Option<String>,
    ) -> Self {
        Self {
            sym,
            name,
            color,
            text_color,
            par_value: None,
            market_position: None,
            trains: Vec::new(),
            last_revenue: 0,
        }
    }
}

/// Train instance assigned to a corporation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorporationTrain {
    pub name: String,
    pub distance: serde_json::Value,
    pub price: Option<i64>,
    #[serde(default)]
    pub revenue_stops: Vec<i32>,
    #[serde(default)]
    pub last_revenue: i32,
}

/// Market position within the stock grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPosition {
    pub row: usize,
    pub col: usize,
    pub value: Option<i32>,
    pub raw: String,
}

/// Extracted market cell information used for navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCell {
    pub row: usize,
    pub col: usize,
    pub value: Option<i32>,
    pub raw: String,
    pub is_par: bool,
}

/// Train definition sourced from the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainType {
    pub name: String,
    pub distance: serde_json::Value,
    pub price: Option<i64>,
    pub total: i64,
    pub rusts_on: serde_json::Value,
    pub obsolete_on: serde_json::Value,
}

/// Entry representing the remaining train supply for a type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainPoolEntry {
    pub name: String,
    pub remaining: i64,
}

/// Aggregated state describing a game session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSession {
    pub info: GameInfo,
    pub corporations: Vec<Corporation>,
    pub market: Vec<Vec<String>>,
    pub market_cells: Vec<MarketCell>,
    #[serde(
        serialize_with = "serialize_market_index",
        deserialize_with = "deserialize_market_index"
    )]
    pub market_index: HashMap<(usize, usize), MarketCell>,
    pub par_cells: Vec<MarketCell>,
    pub train_types: Vec<TrainType>,
    pub train_pool: Vec<TrainPoolEntry>,
    pub phases: Vec<serde_json::Value>,
    pub loaded_at: DateTime<Utc>,
}

impl GameSession {
    pub fn market_cell(&self, row: usize, col: usize) -> Option<&MarketCell> {
        self.market_index.get(&(row, col))
    }
}

fn serialize_market_index<S>(
    value: &HashMap<(usize, usize), MarketCell>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(value.len()))?;
    for ((row, col), cell) in value {
        map.serialize_entry(&format!("{row},{col}"), cell)?;
    }
    map.end()
}

fn deserialize_market_index<'de, D>(
    deserializer: D,
) -> Result<HashMap<(usize, usize), MarketCell>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: HashMap<String, MarketCell> = HashMap::deserialize(deserializer)?;
    let mut result = HashMap::with_capacity(raw.len());
    for (key, cell) in raw {
        let mut parts = key.splitn(2, ',');
        let row_str = parts
            .next()
            .ok_or_else(|| de::Error::custom(format!("invalid key '{key}'")))?;
        let col_str = parts
            .next()
            .ok_or_else(|| de::Error::custom(format!("invalid key '{key}'")))?;
        let row = row_str
            .parse::<usize>()
            .map_err(|_| de::Error::custom(format!("invalid row in key '{key}'")))?;
        let col = col_str
            .parse::<usize>()
            .map_err(|_| de::Error::custom(format!("invalid column in key '{key}'")))?;
        result.insert((row, col), cell);
    }
    Ok(result)
}
