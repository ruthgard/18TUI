//! Resource loading and synchronisation.

/// Game discovery and metadata extraction utilities.
pub mod loader;
/// Git-based resource synchronisation helpers.
pub mod sync;

pub use loader::{GameDiscovery, ResourceLoader};
pub use sync::{ResourceSync, SyncEvent};
