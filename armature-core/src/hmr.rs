//! Hot Module Reload (HMR) System
//!
//! Provides file watching and hot reload capabilities for development mode.
//! Supports automatic reloading of JavaScript, TypeScript, CSS, and other web assets.
//!
//! ## How it works
//!
//! 1. [`HmrManager::start_watching`] spawns a background task that watches the
//!    configured paths with the `notify` crate. Relevant file-system events are
//!    debounced and published as [`HmrEvent`]s on an internal
//!    `tokio::sync::broadcast` channel.
//! 2. [`HmrManager::start_websocket_server`] binds a real WebSocket server
//!    (`tokio_tungstenite`) on `127.0.0.1:{websocket_port}`. Each accepted
//!    connection subscribes to the broadcast channel and forwards every
//!    [`HmrEvent`] to the browser as a JSON message
//!    (`{"type": "css-update" | "js-update" | "full-reload", "path": "..."}`).
//!    The per-connection task ends as soon as the client disconnects or a send
//!    fails, so connections never accumulate.
//! 3. [`inject_hmr_script`] injects a small client script into served HTML that
//!    connects to the WebSocket server and reloads CSS in place or triggers a
//!    full page refresh based on the received message type.
//!
//! The accept loop is aborted when [`HmrManager::stop`] is called or the
//! manager is dropped, and events can also be published manually with
//! [`HmrManager::publish`].

use crate::Error;
use futures_util::{SinkExt, StreamExt};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;

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
    client_count: Arc<AtomicUsize>,
    last_events: Arc<RwLock<HashMap<PathBuf, std::time::SystemTime>>>,
    server_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl HmrManager {
    /// Create a new HMR manager
    pub fn new(config: HmrConfig) -> Self {
        let (event_tx, _) = broadcast::channel(100);

        Self {
            config,
            event_tx,
            client_count: Arc::new(AtomicUsize::new(0)),
            last_events: Arc::new(RwLock::new(HashMap::new())),
            server_handle: std::sync::Mutex::new(None),
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

    /// Start the HMR WebSocket notification server.
    ///
    /// Binds a `TcpListener` on `127.0.0.1:{websocket_port}` (use port `0` for
    /// an ephemeral port) and spawns an accept loop. Each accepted connection
    /// is upgraded with `tokio_tungstenite::accept_async` and forwards every
    /// broadcast [`HmrEvent`] to the client as a JSON reload message until the
    /// client disconnects or a send fails.
    ///
    /// Returns the actual bound port. The accept loop runs until [`stop`] is
    /// called or the manager is dropped.
    ///
    /// [`stop`]: HmrManager::stop
    pub async fn start_websocket_server(&self) -> Result<u16, Error> {
        let listener = TcpListener::bind(("127.0.0.1", self.config.websocket_port))
            .await
            .map_err(|e| Error::Internal(format!("HMR WebSocket bind failed: {}", e)))?;
        let port = listener
            .local_addr()
            .map_err(|e| Error::Internal(format!("HMR WebSocket local_addr failed: {}", e)))?
            .port();

        if self.config.verbose {
            println!("🔥 HMR WebSocket server listening on ws://127.0.0.1:{port}");
        }

        let event_tx = self.event_tx.clone();
        let client_count = self.client_count.clone();
        let verbose = self.config.verbose;

        let handle = tokio::spawn(async move {
            loop {
                let (stream, _addr) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        if verbose {
                            eprintln!("❌ HMR WebSocket accept error: {}", e);
                        }
                        continue;
                    }
                };

                // Subscribe before the handshake so no event published during
                // the handshake window is missed.
                let rx = event_tx.subscribe();
                let client_count = client_count.clone();

                tokio::spawn(async move {
                    // Bound the handshake so a client that opens TCP but never
                    // completes the WebSocket upgrade (slow-loris) cannot park
                    // this task forever. On timeout or handshake error we drop
                    // the connection and return without ever touching the
                    // client count.
                    let handshake = tokio::time::timeout(
                        Duration::from_secs(10),
                        tokio_tungstenite::accept_async(stream),
                    )
                    .await;

                    match handshake {
                        Ok(Ok(ws)) => {
                            client_count.fetch_add(1, Ordering::SeqCst);
                            Self::forward_events(ws, rx, verbose).await;
                            client_count.fetch_sub(1, Ordering::SeqCst);
                        }
                        Ok(Err(e)) => {
                            if verbose {
                                eprintln!("❌ HMR WebSocket handshake failed: {}", e);
                            }
                        }
                        Err(_elapsed) => {
                            if verbose {
                                eprintln!(
                                    "❌ HMR WebSocket handshake timed out; dropping connection"
                                );
                            }
                        }
                    }
                });
            }
        });

        let mut slot = self
            .server_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(old) = slot.replace(handle) {
            old.abort();
        }

        Ok(port)
    }

    /// Forward broadcast HMR events to a single WebSocket client until the
    /// client disconnects, a send fails, or the event channel closes.
    async fn forward_events(
        mut ws: WebSocketStream<TcpStream>,
        mut rx: broadcast::Receiver<HmrEvent>,
        verbose: bool,
    ) {
        loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Ok(event) => {
                        let message = Self::reload_message(&event);
                        if ws.send(WsMessage::text(message)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        if verbose {
                            eprintln!("⚠️  HMR client lagged, skipped {} events", skipped);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                incoming = ws.next() => match incoming {
                    Some(Ok(msg)) if msg.is_close() => break,
                    Some(Ok(_)) => {} // Ignore client messages (pings are auto-handled)
                    Some(Err(_)) | None => break,
                },
            }
        }

        let _ = ws.close(None).await;
    }

    /// Build the JSON reload message the embedded client script expects.
    fn reload_message(event: &HmrEvent) -> String {
        let msg_type = match event.extension.as_deref() {
            Some("css") | Some("scss") | Some("less") => "css-update",
            Some("js") | Some("ts") | Some("jsx") | Some("tsx") => "js-update",
            _ => "full-reload",
        };

        serde_json::json!({
            "type": msg_type,
            "path": event.path.to_string_lossy(),
        })
        .to_string()
    }

    /// Publish an HMR event to all connected WebSocket clients.
    ///
    /// Returns the number of subscribers the event was delivered to. The file
    /// watcher publishes on the same channel; this method exists for manual /
    /// programmatic reload triggers.
    pub fn publish(&self, event: HmrEvent) -> usize {
        self.event_tx.send(event).unwrap_or(0)
    }

    /// Get number of currently connected WebSocket clients
    pub fn client_count(&self) -> usize {
        self.client_count.load(Ordering::SeqCst)
    }

    /// Stop the WebSocket server's accept loop.
    ///
    /// Existing client connections finish on their own when they disconnect or
    /// when the manager (and its broadcast channel) is dropped.
    pub fn stop(&self) {
        let mut slot = self
            .server_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(handle) = slot.take() {
            handle.abort();
        }
    }
}

impl Drop for HmrManager {
    fn drop(&mut self) {
        self.stop();
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

        assert_eq!(manager.client_count(), 0);
    }

    #[test]
    fn test_reload_message_types() {
        let event = |ext: &str| HmrEvent {
            kind: HmrEventKind::Modified,
            path: PathBuf::from(format!("src/app.{ext}")),
            extension: Some(ext.to_string()),
            timestamp: std::time::SystemTime::now(),
        };

        let css: serde_json::Value =
            serde_json::from_str(&HmrManager::reload_message(&event("css"))).unwrap();
        assert_eq!(css["type"], "css-update");
        assert_eq!(css["path"], "src/app.css");

        let js: serde_json::Value =
            serde_json::from_str(&HmrManager::reload_message(&event("ts"))).unwrap();
        assert_eq!(js["type"], "js-update");

        let html: serde_json::Value =
            serde_json::from_str(&HmrManager::reload_message(&event("html"))).unwrap();
        assert_eq!(html["type"], "full-reload");
    }

    #[tokio::test]
    async fn test_websocket_server_delivers_reload_message() {
        // Bind on an ephemeral port so the test never collides with a real server.
        let config = HmrConfig::new().websocket_port(0);
        let manager = HmrManager::new(config);
        let port = manager.start_websocket_server().await.unwrap();

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
            .await
            .expect("client should connect to HMR WebSocket server");

        // Wait for the server side of the handshake to complete.
        for _ in 0..200 {
            if manager.client_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(manager.client_count(), 1);

        // Trigger a change notification via the publish path directly.
        let delivered = manager.publish(HmrEvent {
            kind: HmrEventKind::Modified,
            path: PathBuf::from("public/styles/app.css"),
            extension: Some("css".to_string()),
            timestamp: std::time::SystemTime::now(),
        });
        assert_eq!(delivered, 1);

        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for reload message")
            .expect("stream ended before reload message")
            .expect("websocket error");

        let text = msg.into_text().expect("reload message should be text");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["type"], "css-update");
        assert_eq!(json["path"], "public/styles/app.css");

        // Disconnect and verify the per-connection task cleans up.
        drop(ws);
        for _ in 0..200 {
            if manager.client_count() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(manager.client_count(), 0);

        manager.stop();
    }

    #[tokio::test]
    async fn test_incomplete_handshake_never_counts_as_client() {
        // A client that opens a raw TCP connection but never sends a WebSocket
        // handshake must not be counted as a connected client, and the handshake
        // is bounded by a timeout so the per-connection task cannot leak.
        let config = HmrConfig::new().websocket_port(0);
        let manager = HmrManager::new(config);
        let port = manager.start_websocket_server().await.unwrap();

        // Open TCP but send nothing — the handshake stays pending.
        let _sock = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("raw TCP connect should succeed");

        // Give the accept loop time to spawn the handshake task, then confirm
        // the client count never rises: an unfinished handshake is not a client.
        for _ in 0..40 {
            assert_eq!(manager.client_count(), 0);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(manager.client_count(), 0);

        manager.stop();
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
