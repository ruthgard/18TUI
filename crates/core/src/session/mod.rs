#![allow(missing_docs)]

//! Game session models and loader.

pub mod loader;
mod models;

pub use loader::SessionLoader;
pub use models::{
    Corporation, CorporationTrain, GameSession, MarketCell, MarketPosition, TrainPoolEntry,
    TrainType,
};
