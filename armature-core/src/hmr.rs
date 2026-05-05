//! Hot Module Reload (HMR) System
//!
//! Provides file watching and hot reload capabilities for development mode.
//! Supports automatic reloading of JavaScript, TypeScript, CSS, and other web assets.
//!
//! ## Features
//!
//! - File system watching
//! - WebSocket-based change notification
//! - Automatic browser refresh
//! - Module replacement without full page reload
//! - Support for static assets and templates

use crate::{Error, HttpRequest, HttpResponse};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast};

/// HMR file change event
#[derive(Debug, Clone)]
pub struct HmrEvent {
    /// Type of change (modified, created, deleted)
    pub kind: HmrEventKind,
    /// Path to the changed file
    pub path: PathBuf,
    /// File extension
    pub extension: Option<String>,
    /// Timestamp of the event
    pub timestamp: std::time::SystemTime,
}

/// Type of HMR event
#[derive(Debug, Clone, PartialEq)]
pub enum HmrEventKind {
    /// File was modified
    Modified,
    /// File was created
    Created,
    /// File was deleted
    Deleted,
}

/// HMR configuration
#[derive(Clone, Debug)]
pub struct HmrConfig {
    /// Enable HMR (typically only in development)
    pub enabled: bool,

    /// Paths to watch for changes
    pub watch_paths: Vec<PathBuf>,

    /// File extensions to watch (e.g., ["js", "ts", "css", "html"])
    pub watch_extensions: Vec<String>,

    /// Paths to ignore
    pub ignore_patterns: Vec<String>,

    /// Debounce delay (ms) to avoid rapid-fire events
    pub debounce_ms: u64,

    /// WebSocket port for HMR client
    pub websocket_port: u16,

    /// Enable verbose logging
    pub verbose: bool,
}

impl Default for HmrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            watch_paths: vec![PathBuf::from("src"), PathBuf::from("public")],
            watch_extensions: vec![
                "js".to_string(),
                "ts".to_string(),
                "jsx".to_string(),
                "tsx".to_string(),
                "css".to_string(),
                "scss".to_string(),
                "less".to_string(),
                "html".to_string(),
                "vue".to_string(),
                "svelte".to_string(),
            ],
            ignore_patterns: vec![
                "node_modules".to_string(),
                "dist".to_string(),
                "build".to_string(),
                ".git".to_string(),
                "target".to_string(),
            ],
            debounce_ms: 100,
            websocket_port: 3001,
            verbose: false,
        }
    }
}

impl HmrConfig {
    /// Create a new HMR configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a path to watch
    pub fn watch_path(mut self, path: PathBuf) -> Self {
        self.watch_paths.push(path);
        self
    }

    /// Add file extensions to watch
    pub fn watch_extension(mut self, ext: String) -> Self {
        self.watch_extensions.push(ext);
        self
    }

    /// Add ignore pattern
    pub fn ignore_pattern(mut self, pattern: String) -> Self {
        self.ignore_patterns.push(pattern);
        self
    }

    /// Set debounce delay
    pub fn debounce(mut self, ms: u64) -> Self {
        self.debounce_ms = ms;
        self
    }

    /// Set WebSocket port
    pub fn websocket_port(mut self, port: u16) -> Self {
        self.websocket_port = port;
        self
    }

    /// Enable verbose logging
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }
}

/// HMR Manager - Coordinates file watching and client notifications
pub struct HmrManager {
    config: HmrConfig,
    event_tx: broadcast::Sender<HmrEvent>,
    clients: Arc<RwLock<Vec<ClientConnection>>>,
    last_events: Arc<RwLock<HashMap<PathBuf, std::time::SystemTime>>>,
}

/// WebSocket client connection
#[allow(dead_code)]
struct ClientConnection {
    id: String,
    connected_at: std::time::SystemTime,
}

impl HmrManager {
    /// Create a new HMR manager
    pub fn new(config: HmrConfig) -> Self {
        let (event_tx, _) = broadcast::channel(100);

        Self {
            config,
            event_tx,
            clients: Arc::new(RwLock::new(Vec::new())),
            last_events: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start watching for file changes
    pub async fn start_watching(&self) -> Result<(), Error> {
        if !self.config.enabled {
            println!("🔥 HMR disabled");
            return Ok(());
        }

        println!("🔥 HMR enabled - watching for changes...");

        if self.config.verbose {
            println!("   Watching paths:");
            for path in &self.config.watch_paths {
                println!("     - {}", path.display());
            }
            println!("   Extensions: {:?}", self.config.watch_extensions);
        }

        let event_tx = self.event_tx.clone();
        let config = self.config.clone();
        let last_events = self.last_events.clone();

        // Spawn file watcher in background task
        tokio::spawn(async move {
            if let Err(e) = Self::watch_files(event_tx, config, last_events).await {
                eprintln!("❌ HMR file watcher error: {}", e);
            }
        });

        Ok(())
    }

    /// Watch files for changes
    async fn watch_files(
        event_tx: broadcast::Sender<HmrEvent>,
        config: HmrConfig,
        last_events: Arc<RwLock<HashMap<PathBuf, std::time::SystemTime>>>,
    ) -> Result<(), Error> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        // Create file watcher
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(config.debounce_ms)),
        )
        .map_err(|e| Error::Internal(format!("Failed to create watcher: {}", e)))?;

        // Watch all configured paths
        for path in &config.watch_paths {
            if path.exists() {
                watcher
                    .watch(path, RecursiveMode::Recursive)
                    .map_err(|e| Error::Internal(format!("Failed to watch path: {}", e)))?;
            } else if config.verbose {
                println!("⚠️  Path not found: {}", path.display());
            }
        }

        // Process file system events
        while let Some(event) = rx.recv().await {
            if let Some(hmr_event) = Self::process_event(event, &config, &last_events).await {
                // Broadcast to all listeners
                let _ = event_tx.send(hmr_event);
            }
        }

        Ok(())
    }

    /// Process a file system event
    async fn process_event(
        event: Event,
        config: &HmrConfig,
        last_events: &Arc<RwLock<HashMap<PathBuf, std::time::SystemTime>>>,
    ) -> Option<HmrEvent> {
        let kind = match event.kind {
            EventKind::Modify(_) => HmrEventKind::Modified,
            EventKind::Create(_) => HmrEventKind::Created,
            EventKind::Remove(_) => HmrEventKind::Deleted,
            _ => return None,
        };

        // Get the first path from the event
        let path = event.paths.first()?.clone();

        // Check if path should be ignored
        if Self::should_ignore(&path, &config.ignore_patterns) {
            return None;
        }

        // Check file extension
        let extension = path.extension()?.to_str()?.to_string();
        if !config.watch_extensions.contains(&extension) {
            return None;
        }

        // Debounce: Check if this event is too recent
        let now = std::time::SystemTime::now();
        let mut last_events_map = last_events.write().await;

        if let Some(last_time) = last_events_map.get(&path)
            && let Ok(duration) = now.duration_since(*last_time)
            && duration.as_millis() < config.debounce_ms as u128
        {
            // Too recent, skip
            return None;
        }

        // Update last event time
        last_events_map.insert(path.clone(), now);

        if config.verbose {
            println!("🔄 HMR: {:?} - {}", kind, path.display());
        }

        Some(HmrEvent {
            kind,
            path: path.clone(),
            extension: Some(extension),
            timestamp: now,
        })
    }

    /// Check if path should be ignored
    fn should_ignore(path: &Path, ignore_patterns: &[String]) -> bool {
        let path_str = path.to_string_lossy();
        ignore_patterns
            .iter()
            .any(|pattern| path_str.contains(pattern))
    }

    /// Subscribe to HMR events
    pub fn subscribe(&self) -> broadcast::Receiver<HmrEvent> {
        self.event_tx.subscribe()
    }

    /// Get HMR client script for injection into HTML
    pub fn get_client_script(&self) -> String {
        format!(
            r#"<script>
(function() {{
  console.log('🔥 HMR Client initialized');

  let ws;
  let reconnectAttempts = 0;
  const maxReconnectAttempts = 10;

  function connect() {{
    ws = new WebSocket('ws://localhost:{}');

    ws.onopen = function() {{
      console.log('🔥 HMR Connected');
      reconnectAttempts = 0;
    }};

    ws.onmessage = function(event) {{
      const data = JSON.parse(event.data);
      console.log('🔥 HMR Update:', data);

      if (data.type === 'full-reload') {{
        console.log('🔥 HMR: Full page reload');
        window.location.reload();
      }} else if (data.type === 'css-update') {{
        console.log('🔥 HMR: CSS hot reload');
        reloadCSS(data.path);
      }} else if (data.type === 'js-update') {{
        console.log('🔥 HMR: JavaScript update, reloading...');
        window.location.reload();
      }}
    }};

    ws.onclose = function() {{
      console.log('🔥 HMR Disconnected');
      if (reconnectAttempts < maxReconnectAttempts) {{
        reconnectAttempts++;
        setTimeout(connect, 1000 * reconnectAttempts);
      }}
    }};

    ws.onerror = function(error) {{
      console.error('🔥 HMR Error:', error);
    }};
  }}

  function reloadCSS(path) {{
    const links = document.querySelectorAll('link[rel="stylesheet"]');
    links.forEach(link => {{
      if (!path || link.href.includes(path)) {{
        const href = link.href.split('?')[0];
        link.href = href + '?t=' + Date.now();
      }}
    }});
  }}

  connect();
}})();
</script>"#,
            self.config.websocket_port
        )
    }

    /// Handle WebSocket upgrade request
    pub async fn handle_websocket(&self, _req: &HttpRequest) -> Result<HttpResponse, Error> {
        // This would be implemented with a WebSocket library
        // For now, return a placeholder
        Ok(HttpResponse::ok().with_body(b"WebSocket upgrade".to_vec()))
    }

    /// Register a new client connection
    pub async fn register_client(&self, client_id: String) {
        let mut clients = self.clients.write().await;
        clients.push(ClientConnection {
            id: client_id,
            connected_at: std::time::SystemTime::now(),
        });
    }

    /// Get number of connected clients
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }
}

/// HMR middleware for injecting client script
pub async fn inject_hmr_script(html: String, hmr_manager: &HmrManager) -> String {
    if !hmr_manager.config.enabled {
        return html;
    }

    let script = hmr_manager.get_client_script();

    // Inject before </body> if possible, otherwise append
    if let Some(pos) = html.rfind("</body>") {
        let mut result = html;
        result.insert_str(pos, &script);
        result
    } else {
        format!("{}{}", html, script)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmr_config_builder() {
        let config = HmrConfig::new()
            .watch_path(PathBuf::from("src"))
            .watch_extension("rs".to_string())
            .debounce(200)
            .websocket_port(3002);

        assert_eq!(config.debounce_ms, 200);
        assert_eq!(config.websocket_port, 3002);
        assert!(config.watch_paths.contains(&PathBuf::from("src")));
    }

    #[test]
    fn test_should_ignore() {
        let ignore_patterns = vec!["node_modules".to_string(), ".git".to_string()];

        assert!(HmrManager::should_ignore(
            Path::new("node_modules/package/index.js"),
            &ignore_patterns
        ));

        assert!(HmrManager::should_ignore(
            Path::new(".git/config"),
            &ignore_patterns
        ));

        assert!(!HmrManager::should_ignore(
            Path::new("src/main.ts"),
            &ignore_patterns
        ));
    }

    #[tokio::test]
    async fn test_hmr_manager_creation() {
        let config = HmrConfig::new();
        let manager = HmrManager::new(config);

        assert_eq!(manager.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_client_registration() {
        let config = HmrConfig::new();
        let manager = HmrManager::new(config);

        manager.register_client("test-client-1".to_string()).await;
        manager.register_client("test-client-2".to_string()).await;

        assert_eq!(manager.client_count().await, 2);
    }

    #[test]
    fn test_client_script_generation() {
        let config = HmrConfig::new().websocket_port(3333);
        let manager = HmrManager::new(config);

        let script = manager.get_client_script();

        assert!(script.contains("ws://localhost:3333"));
        assert!(script.contains("HMR Client initialized"));
    }

    #[tokio::test]
    async fn test_inject_hmr_script() {
        let config = HmrConfig::new();
        let manager = HmrManager::new(config);

        let html = "<html><body><h1>Hello</h1></body></html>".to_string();
        let result = inject_hmr_script(html, &manager).await;

        assert!(result.contains("<script>"));
        assert!(result.contains("HMR Client"));
        assert!(result.contains("</body>"));
    }

    #[tokio::test]
    async fn test_inject_hmr_script_no_body() {
        let config = HmrConfig::new();
        let manager = HmrManager::new(config);

        let html = "<html><div>Content</div></html>".to_string();
        let result = inject_hmr_script(html, &manager).await;

        assert!(result.contains("<script>"));
        assert!(result.ends_with("</script>"));
    }
}
