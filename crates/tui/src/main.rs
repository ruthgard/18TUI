//! Binary entry point for the terminal UI.
//!
//! The `tui18-core` crate hosts domain logic and resource management, while
//! this binary wires those pieces up to Ratatui/Crossterm, config loading,
//! and the async sync task.

mod app;
mod block_font;

use anyhow::Result;
use std::fs::{self, OpenOptions};

use tokio::sync::mpsc;
use tracing_subscriber::{prelude::*, EnvFilter};
use tui18_core::{
    config::{self, AppConfig},
    resource::{ResourceLoader, ResourceSync},
    session::SessionLoader,
};

/// Boots the async runtime, prepares shared services, and hands control over to the UI loop.
#[tokio::main]
async fn main() -> Result<()> {
    init_logging()?;

    // Configuration drives where the Ruby engine lives and where saves are stored.
    config::ensure_default_config()?;
    let config = AppConfig::load()?;

    // The resource sync keeps the Ruby data repo fresh in the background.
    let sync = ResourceSync::new(config.clone());
    let metadata = sync.prepare().await?;
    let repo_path = sync.repo_path();
    let loader = ResourceLoader::new(repo_path, metadata.clone());
    let session_loader = SessionLoader::new(loader.root_path());

    // Wire the long-running sync task to a channel so we can surface progress in the UI.
    let (sync_tx, sync_rx) = mpsc::channel(8);
    tokio::spawn(async move {
        if let Err(err) = sync.run(sync_tx).await {
            tracing::error!("Resource sync task error: {err}");
        }
    });

    let mut app = app::Tui18App::new(loader, metadata, session_loader);
    app.attach_sync(sync_rx);
    app.run().await
}

/// Installs both stdout and file-based logging layers so tracing spans remain
/// available while debugging user terminals or later via `logs/tui18.log`.
fn init_logging() -> Result<()> {
    let log_dir = std::env::current_dir()?.join("logs");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("tui18.log");

    let env_filter = EnvFilter::from_default_env();

    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .compact()
        .with_writer(move || {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .expect("failed to open log file")
        });

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .init();

    Ok(())
}
