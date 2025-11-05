#![warn(clippy::all, missing_docs)]

//! Core domain logic for the 18TUI Rust port.
//!
//! This crate hosts the data models, configuration handling,
//! resource discovery/synchronisation, and persistence layers
//! used by the terminal UI and any future frontends.

pub mod config;
pub mod manifest;
pub mod models;
pub mod resource;
pub mod save;
pub mod session;

pub use config::AppConfig;
pub use manifest::ResourceMetadata;
pub use models::GameInfo;
pub use session::{
    Corporation, CorporationTrain, GameSession, MarketCell, MarketPosition, SessionLoader,
    TrainPoolEntry, TrainType,
};
