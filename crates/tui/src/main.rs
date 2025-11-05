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

#[tokio::main]
async fn main() -> Result<()> {
    init_logging()?;

    config::ensure_default_config()?;
    let config = AppConfig::load()?;

    let sync = ResourceSync::new(config.clone());
    let metadata = sync.prepare().await?;
    let repo_path = sync.repo_path();
    let loader = ResourceLoader::new(repo_path, metadata.clone());
    let session_loader = SessionLoader::new(loader.root_path());

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

fn init_logging() -> Result<()> {
    let log_dir = std::env::current_dir()?.join("logs");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("tui18.log");

    let env_filter = EnvFilter::from_default_env();

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact()
        .with_writer(std::io::stdout);

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
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(())
}
