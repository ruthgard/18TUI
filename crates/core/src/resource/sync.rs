use std::{path::PathBuf, process::Stdio};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use tokio::{process::Command, sync::mpsc};
use tracing::info;

use crate::{config::AppConfig, manifest, manifest::ResourceMetadata};

/// Events emitted by the async resource synchroniser.
#[derive(Debug)]
pub enum SyncEvent {
    /// Sync succeeded with new metadata.
    Success {
        /// Filesystem path to the refreshed engine repository.
        path: PathBuf,
        /// Manifest metadata describing the new snapshot.
        metadata: ResourceMetadata,
    },
    /// Sync failed with an error.
    Error(anyhow::Error),
}

/// Coordinates fetching the 18xx engine repository.
pub struct ResourceSync {
    config: AppConfig,
}

impl ResourceSync {
    /// Create a new synchroniser from configuration.
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    /// Path to the local engine repository.
    pub fn repo_path(&self) -> PathBuf {
        self.config.cache_root.join("engine")
    }

    fn manifest_path(&self) -> PathBuf {
        manifest::manifest_path(self.repo_path())
    }

    /// Ensure a checkout exists locally, cloning when missing.
    pub async fn prepare(&self) -> Result<ResourceMetadata> {
        let repo_path = self.repo_path();
        if !repo_path.exists() {
            info!("cloning engine repository into {}", repo_path.display());
            self.clone_repo().await?;
        }

        let metadata = self.capture_metadata().await?;
        self.write_manifest(metadata.clone()).await?;
        Ok(metadata)
    }

    /// Spawn a background task that fetches updates, sending events to the provided channel.
    pub async fn run(self, sender: mpsc::Sender<SyncEvent>) -> Result<()> {
        if let Err(err) = self.update_repo().await {
            let _ = sender.send(SyncEvent::Error(err)).await;
            return Ok(());
        }

        match self.capture_metadata().await {
            Ok(metadata) => {
                if let Err(err) = self.write_manifest(metadata.clone()).await {
                    let _ = sender.send(SyncEvent::Error(err)).await;
                    return Ok(());
                }

                sender
                    .send(SyncEvent::Success {
                        path: self.repo_path(),
                        metadata,
                    })
                    .await
                    .context("failed to send sync success event")?;
            }
            Err(err) => {
                let _ = sender.send(SyncEvent::Error(err)).await;
            }
        }

        Ok(())
    }

    async fn clone_repo(&self) -> Result<()> {
        let repo_path = self.repo_path();
        if let Some(parent) = repo_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("failed to create cache directory")?;
        }

        let status = Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--branch")
            .arg(&self.config.repo_branch)
            .arg(&self.config.repo_url)
            .arg(&repo_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to execute git clone")?;

        if !status.success() {
            anyhow::bail!("git clone exited with {}", status);
        }

        Ok(())
    }

    async fn update_repo(&self) -> Result<()> {
        if !self.repo_path().exists() {
            self.clone_repo().await?;
            return Ok(());
        }

        let status = Command::new("git")
            .arg("fetch")
            .arg("--depth")
            .arg("1")
            .arg("origin")
            .arg(&self.config.repo_branch)
            .current_dir(self.repo_path())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to execute git fetch")?;

        if !status.success() {
            anyhow::bail!("git fetch exited with {}", status);
        }

        let status = Command::new("git")
            .arg("reset")
            .arg("--hard")
            .arg(format!("origin/{}", self.config.repo_branch))
            .current_dir(self.repo_path())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to execute git reset")?;

        if !status.success() {
            anyhow::bail!("git reset exited with {}", status);
        }

        Ok(())
    }

    async fn capture_metadata(&self) -> Result<ResourceMetadata> {
        let commit = self.capture(&["rev-parse", "HEAD"]).await?;
        let updated_at = self.capture(&["log", "-1", "--format=%cI"]).await?;
        let commit = commit.trim().to_string();
        let updated_at = DateTime::parse_from_rfc3339(updated_at.trim())?.with_timezone(&Utc);
        Ok(ResourceMetadata {
            commit: Some(commit),
            updated_at: Some(updated_at),
        })
    }

    async fn capture(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.repo_path())
            .output()
            .await
            .with_context(|| format!("failed to execute git {}", args.join(" ")))?;

        if !output.status.success() {
            anyhow::bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    async fn write_manifest(&self, metadata: ResourceMetadata) -> Result<()> {
        metadata.persist(self.manifest_path())
    }
}
