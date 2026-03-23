//! Hot-reload file watcher for development.

use crate::engine::RhaiEngine;
use crate::error::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// File watcher for hot-reloading scripts during development.
pub struct ScriptWatcher {
    engine: Arc<RhaiEngine>,
    watch_dirs: Vec<PathBuf>,
    debounce_ms: u64,
}

impl ScriptWatcher {
    /// Create a new script watcher.
    pub fn new(engine: Arc<RhaiEngine>) -> Self {
        Self {
            engine,
            watch_dirs: Vec::new(),
            debounce_ms: 100,
        }
    }

    /// Add a directory to watch.
    pub fn watch(mut self, dir: impl Into<PathBuf>) -> Self {
        self.watch_dirs.push(dir.into());
        self
    }

    /// Set debounce duration in milliseconds.
    pub fn debounce(mut self, ms: u64) -> Self {
        self.debounce_ms = ms;
        self
    }

    /// Start watching for changes.
    ///
    /// Returns a channel receiver that emits paths of changed scripts.
    pub async fn start(self) -> Result<mpsc::Receiver<PathBuf>> {
        let (tx, rx) = mpsc::channel(100);
        let engine = self.engine.clone();
        let debounce_ms = self.debounce_ms;

        // Create sync channel for notify
        let (sync_tx, mut sync_rx) = mpsc::channel(100);

        // Spawn watcher in blocking task
        let watch_dirs = self.watch_dirs.clone();
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();

            let mut watcher = RecommendedWatcher::new(
                move |res: std::result::Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        for path in event.paths {
                            if path.extension().map(|e| e == "rhai").unwrap_or(false) {
                                let _ = rt.block_on(sync_tx.send(path));
                            }
                        }
                    }
                },
                Config::default(),
            )
            .expect("Failed to create watcher");

            for dir in &watch_dirs {
                if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                    warn!("Failed to watch {}: {}", dir.display(), e);
                } else {
                    info!("Watching for changes: {}", dir.display());
                }
            }

            // Keep watcher alive
            std::thread::park();
        });

        // Spawn debounce processor
        tokio::spawn(async move {
            let mut pending: Option<PathBuf> = None;
            let mut last_event = std::time::Instant::now();

            loop {
                tokio::select! {
                    path = sync_rx.recv() => {
                        match path {
                            Some(p) => {
                                pending = Some(p);
                                last_event = std::time::Instant::now();
                            }
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(debounce_ms)) => {
                        if let Some(path) = pending.take() {
                            if last_event.elapsed() >= Duration::from_millis(debounce_ms) {
                                debug!("Script changed: {}", path.display());

                                // Invalidate cache
                                engine.invalidate(&path);

                                // Notify listeners
                                if tx.send(path).await.is_err() {
                                    break;
                                }
                            } else {
                                pending = Some(path);
                            }
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

/// Extension trait for RhaiEngine to enable hot reload.
pub trait HotReloadExt {
    /// Create a watcher for this engine.
    fn watcher(self: &Arc<Self>) -> ScriptWatcher;
}

impl HotReloadExt for RhaiEngine {
    fn watcher(self: &Arc<Self>) -> ScriptWatcher {
        ScriptWatcher::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watcher_creation() {
        let engine = Arc::new(RhaiEngine::default());
        let watcher = ScriptWatcher::new(engine).watch("./scripts").debounce(50);

        // Just test creation, not actual watching
        assert_eq!(watcher.debounce_ms, 50);
    }
}
