//! Ferron manager for integrated proxy management
//!
//! This module provides a high-level manager that coordinates configuration
//! generation, process management, health checking, and service discovery.

use crate::config::{Backend, FerronConfig, LoadBalancer};
use crate::error::{FerronError, Result};
use crate::health::{HealthCheckConfig, HealthState};
use crate::process::{FerronProcess, ProcessConfig, ProcessStatus};
use crate::registry::ServiceRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Ferron manager for complete proxy lifecycle management
pub struct FerronManager {
    /// Configuration file path
    config_path: PathBuf,
    /// Ferron configuration
    config: Arc<RwLock<Option<FerronConfig>>>,
    /// Ferron process handle
    process: Arc<FerronProcess>,
    /// Service registry for dynamic backends
    registry: Option<Arc<ServiceRegistry>>,
    /// Health state tracker
    health_state: Option<Arc<HealthState>>,
    /// Auto-reload on config changes
    auto_reload: bool,
    /// Watch handle for the background config-file watcher task, populated
    /// by `build()` when `auto_reload` is enabled and a Tokio runtime is
    /// available to run it on.
    watch_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// Count of reload signals to Ferron — bumped by both `reload()`
    /// (regenerate + rewrite + signal) and `signal_reload()` (signal only,
    /// the watcher's path). Exposed via `reload_count()` both as a genuinely
    /// useful operational stat and so tests can verify the config-file
    /// watcher debounces a burst of filesystem events into a single reload
    /// rather than one reload per raw event.
    reload_count: AtomicU64,
}

impl FerronManager {
    /// Create a new manager builder
    pub fn builder() -> FerronManagerBuilder {
        FerronManagerBuilder::default()
    }

    /// Get current process status
    pub async fn status(&self) -> ProcessStatus {
        self.process.status().await
    }

    /// Get process ID if running
    pub async fn pid(&self) -> Option<u32> {
        self.process.pid().await
    }

    /// Whether a background config-file watcher task is currently running
    /// for this manager (populated by `build()` when `auto_reload` is
    /// enabled).
    pub async fn is_watching(&self) -> bool {
        self.watch_handle.read().await.is_some()
    }

    /// Start Ferron with the current configuration
    pub async fn start(&self) -> Result<()> {
        // Generate configuration if using registry
        if let Some(ref registry) = self.registry {
            self.regenerate_config_from_registry(registry).await?;
        }

        // Start the process
        self.process.start().await
    }

    /// Stop the background config-file watcher task, if one is running.
    ///
    /// This aborts the watcher task via its stored `JoinHandle` and awaits
    /// its completion (discarding the resulting `Cancelled` `JoinError` --
    /// that's the expected outcome of a deliberate abort, not a failure),
    /// then clears `watch_handle` so `is_watching()` reflects the change.
    /// A no-op if no watcher is currently running, and idempotent: calling
    /// it more than once in a row is safe.
    ///
    /// ## Drop semantics
    ///
    /// The watcher task holds only a `Weak<FerronManager>` (not a strong
    /// `Arc`), so it does *not* keep the manager alive on its own, and it
    /// notices the manager has been dropped -- and exits on its own,
    /// dropping its `notify::Watcher` and closing its channel -- the next
    /// time a filesystem event wakes it up. However, if no further
    /// filesystem activity ever occurs after the last external `Arc` is
    /// dropped, the task could in principle remain parked on `recv()`
    /// indefinitely (a much smaller, bounded leak than the original
    /// strong-`Arc` version, but not instantaneous). Call `stop_watching()`
    /// (or `stop()`, which calls it) for a deterministic, immediate
    /// shutdown instead of relying on drop alone.
    pub async fn stop_watching(&self) {
        let handle = self.watch_handle.write().await.take();
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }
    }

    /// Stop Ferron.
    ///
    /// Always tears down the background config-file watcher task first
    /// (see `stop_watching()`), regardless of whether the underlying
    /// process is currently running, and then stops the process itself.
    pub async fn stop(&self) -> Result<()> {
        self.stop_watching().await;
        self.process.stop().await
    }

    /// Restart Ferron
    pub async fn restart(&self) -> Result<()> {
        self.process.restart().await
    }

    /// Reload Ferron configuration, regenerating it from the service
    /// registry first if one is configured.
    ///
    /// This is the entry point for registry-driven state changes --
    /// `register_backend`/`deregister_backend` call it (via
    /// `regenerate_config_from_registry` directly) when the registered
    /// backend set actually changes, and it's also the right thing for a
    /// caller who wants to force a full regenerate-and-reload.
    ///
    /// It must NOT be called from the config-file watcher's own
    /// change-detection path: `regenerate_config_from_registry` writes the
    /// regenerated config back to `config_path`, which is the very file the
    /// watcher watches. If the watcher reacted to *its own* filesystem
    /// events by calling this, that write would re-trigger the watcher,
    /// which would reload and write again, forever -- a self-perpetuating
    /// reload loop with no further external input after the first change.
    /// The watcher calls `signal_reload()` instead, which re-applies the
    /// on-disk config without regenerating or rewriting it.
    pub async fn reload(&self) -> Result<()> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);

        // Regenerate config if using registry
        if let Some(ref registry) = self.registry {
            self.regenerate_config_from_registry(registry).await?;
        }

        self.process.reload().await
    }

    /// Re-apply the current on-disk configuration by signalling the Ferron
    /// process to reload (SIGHUP), without regenerating or rewriting the
    /// config file.
    ///
    /// This is what the config-file watcher calls when it observes a change
    /// to `config_path`: Ferron re-reads the config file from disk on its
    /// own when signalled, so simply signalling it is sufficient to pick up
    /// an external edit. Crucially, unlike `reload()`, this never calls
    /// `regenerate_config_from_registry` (and therefore never calls
    /// `FerronConfig::write_to_file`), so it cannot produce a filesystem
    /// event that re-triggers the very watcher that called it -- breaking
    /// the self-perpetuating reload loop that combining a registry with the
    /// watcher used to cause. Still counted in `reload_count()`, since it's
    /// a real reload signal to the process.
    pub async fn signal_reload(&self) -> Result<()> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        self.process.reload().await
    }

    /// Number of reload signals sent to Ferron so far, counting both
    /// `reload()` and the watcher's `signal_reload()`.
    ///
    /// This is a genuinely useful operational stat (e.g. for dashboards or
    /// alerting on unexpectedly frequent reloads), and also lets tests
    /// verify that the config-file watcher debounces a burst of rapid
    /// filesystem events into a single reload rather than firing once per
    /// raw event.
    pub fn reload_count(&self) -> u64 {
        self.reload_count.load(Ordering::Relaxed)
    }

    /// Get the service registry if configured
    pub fn registry(&self) -> Option<&Arc<ServiceRegistry>> {
        self.registry.as_ref()
    }

    /// Get the health state if configured
    pub fn health_state(&self) -> Option<&Arc<HealthState>> {
        self.health_state.as_ref()
    }

    /// Update configuration and reload
    pub async fn update_config(&self, config: FerronConfig) -> Result<()> {
        // Write new configuration
        config.write_to_file(&self.config_path).await?;
        *self.config.write().await = Some(config);

        // Reload if running
        if self.process.status().await == ProcessStatus::Running {
            self.process.reload().await?;
        }

        Ok(())
    }

    /// Register a backend and update configuration
    pub async fn register_backend(&self, service_name: &str, url: &str) -> Result<String> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| FerronError::registry("Service registry not configured"))?;

        let id = registry.register(service_name, url).await?;

        // Regenerate and reload if auto-reload is enabled
        if self.auto_reload && self.process.status().await == ProcessStatus::Running {
            self.regenerate_config_from_registry(registry).await?;
            self.process.reload().await?;
        }

        Ok(id)
    }

    /// Deregister a backend and update configuration
    pub async fn deregister_backend(&self, service_name: &str, instance_id: &str) -> Result<()> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| FerronError::registry("Service registry not configured"))?;

        registry.deregister(service_name, instance_id).await?;

        // Regenerate and reload if auto-reload is enabled
        if self.auto_reload && self.process.status().await == ProcessStatus::Running {
            self.regenerate_config_from_registry(registry).await?;
            self.process.reload().await?;
        }

        Ok(())
    }

    /// Regenerate configuration from service registry
    async fn regenerate_config_from_registry(&self, registry: &ServiceRegistry) -> Result<()> {
        let mut config_guard = self.config.write().await;
        let config = config_guard
            .as_mut()
            .ok_or_else(|| FerronError::config("No base configuration set"))?;

        // Get all services and their URLs
        let services = registry.get_services().await;

        // Update load balancer with discovered backends
        let mut backends = Vec::new();
        for service in &services {
            let urls = registry.get_healthy_urls(service).await;
            for url in urls {
                backends.push(Backend::new(url));
            }
        }

        if !backends.is_empty() {
            let lb = LoadBalancer::new();
            let mut lb_with_backends = lb;
            for backend in backends {
                lb_with_backends = lb_with_backends.backend(backend);
            }
            config.load_balancer = Some(lb_with_backends);
        }

        // Write updated configuration
        config.write_to_file(&self.config_path).await?;
        info!("Regenerated Ferron configuration from service registry");

        Ok(())
    }

    /// Start with supervision (auto-restart on crash)
    pub async fn start_supervised(self: Arc<Self>) -> Result<tokio::task::JoinHandle<()>> {
        // Start health checking if configured
        if let Some(ref health_state) = self.health_state
            && let Some(ref registry) = self.registry
        {
            let backends: Vec<String> = {
                let services = registry.get_services().await;
                let mut urls = Vec::new();
                for service in services {
                    urls.extend(registry.get_urls(&service).await);
                }
                urls
            };

            if !backends.is_empty() {
                let _ = health_state.clone().start_background_checks(backends).await;
            }
        }

        // Start the process with supervision
        self.process.clone().start_with_supervision().await
    }
}

/// Builder for FerronManager
#[derive(Default)]
pub struct FerronManagerBuilder {
    binary_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    config: Option<FerronConfig>,
    registry: Option<Arc<ServiceRegistry>>,
    health_config: Option<HealthCheckConfig>,
    auto_reload: bool,
    auto_restart: bool,
    working_dir: Option<PathBuf>,
}

impl FerronManagerBuilder {
    /// Set the Ferron binary path
    pub fn binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = Some(path.into());
        self
    }

    /// Set the configuration file path
    pub fn config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Set the Ferron configuration
    pub fn config(mut self, config: FerronConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the service registry for dynamic discovery
    pub fn service_registry(mut self, registry: ServiceRegistry) -> Self {
        self.registry = Some(Arc::new(registry));
        self
    }

    /// Set an existing service registry
    pub fn service_registry_arc(mut self, registry: Arc<ServiceRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Enable health checking with configuration
    pub fn health_check(mut self, config: HealthCheckConfig) -> Self {
        self.health_config = Some(config);
        self
    }

    /// Enable auto-reload on configuration changes
    pub fn auto_reload(mut self, enabled: bool) -> Self {
        self.auto_reload = enabled;
        self
    }

    /// Enable auto-restart on process crash
    pub fn auto_restart(mut self, enabled: bool) -> Self {
        self.auto_restart = enabled;
        self
    }

    /// Set working directory
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    /// Build the FerronManager.
    ///
    /// Returns an `Arc<FerronManager>` (rather than a bare `FerronManager`)
    /// because when `auto_reload` is enabled, `build()` spawns a background
    /// task that watches `config_path` for changes and calls back into the
    /// manager to reload -- that task needs a shared, cloneable handle to
    /// the manager it's watching on behalf of.
    pub fn build(self) -> Result<Arc<FerronManager>> {
        let config_path = self
            .config_path
            .unwrap_or_else(|| PathBuf::from("/etc/ferron/ferron.conf"));

        let binary_path = self.binary_path.unwrap_or_else(|| PathBuf::from("ferron"));

        // Create process config
        let mut process_config = ProcessConfig::new(&binary_path, &config_path);
        process_config.auto_restart = self.auto_restart;

        if let Some(dir) = self.working_dir {
            process_config = process_config.working_dir(dir);
        }

        // Create health state if configured
        let health_state = self
            .health_config
            .map(|config| Arc::new(HealthState::new(config)));

        // Write initial config if provided
        if let Some(ref config) = self.config {
            // Write config synchronously for builder
            let kdl = config.to_kdl()?;
            std::fs::write(&config_path, kdl)?;
        }

        let auto_reload = self.auto_reload;
        let manager = Arc::new(FerronManager {
            config_path,
            config: Arc::new(RwLock::new(self.config)),
            process: Arc::new(FerronProcess::new(process_config)),
            registry: self.registry,
            health_state,
            auto_reload,
            watch_handle: Arc::new(RwLock::new(None)),
            reload_count: AtomicU64::new(0),
        });

        if auto_reload {
            match spawn_config_watcher(Arc::clone(&manager)) {
                Ok(Some(join)) => {
                    // `build()` is synchronous, so populate the freshly
                    // constructed (and therefore uncontended) lock via
                    // `try_write()` rather than requiring an async context.
                    if let Ok(mut guard) = manager.watch_handle.try_write() {
                        *guard = Some(join);
                    }
                }
                Ok(None) => {
                    // No Tokio runtime available (e.g. built outside an
                    // async context); skip watching rather than panicking.
                }
                Err(e) => {
                    warn!("Failed to start Ferron config file watcher: {}", e);
                }
            }
        }

        Ok(manager)
    }
}

/// How long the watcher waits for filesystem activity to go quiet before
/// treating a burst of events as a single logical change and reloading.
/// A single editor "save" (or a config-management tool applying a batch of
/// changes) commonly produces several raw filesystem events in quick
/// succession; without debouncing, each one would trigger its own real
/// reload signal to the child process.
const CONFIG_WATCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

/// Spawn a background task that watches `manager.config_path` for changes
/// using `notify`, signalling the Ferron process to reload (via
/// `FerronManager::signal_reload()`) whenever the file is modified or
/// (re)created -- covering both in-place writes and atomic-replace-via-rename
/// editors. Rapid successive events (e.g. from a single save) are debounced
/// into a single reload; see `CONFIG_WATCH_DEBOUNCE`.
///
/// Deliberately does *not* regenerate the config from the service registry:
/// doing so would rewrite `config_path` -- the very file this task watches
/// -- which would re-trigger this same watcher, forming a self-perpetuating
/// reload loop with no further external input after the first change.
/// Registry-driven regeneration is instead triggered directly by the actual
/// state-change events that warrant it (`register_backend`/
/// `deregister_backend`), never by the watcher observing its own write.
///
/// The task holds only a `Weak<FerronManager>`, not a strong `Arc`: it is
/// otherwise the last thing keeping its own channel's sender alive (via the
/// `notify::Watcher` it owns), so a strong `Arc` here would mean the task
/// -- and therefore the manager -- could never be dropped for the life of
/// the runtime once `auto_reload` has been used. On every received event
/// the task attempts to `upgrade()` the weak reference; once that fails
/// (the manager has been dropped), the task exits, which drops its
/// `notify::Watcher` and closes the channel. For a deterministic,
/// immediate shutdown that doesn't depend on another filesystem event
/// arriving, use `FerronManager::stop_watching()` (or `stop()`), which
/// aborts the task directly via its `JoinHandle`.
///
/// Returns `Ok(None)` (rather than spawning anything) if no Tokio runtime
/// handle is currently available, so this can safely be called from
/// synchronous contexts like `build()`.
fn spawn_config_watcher(
    manager: Arc<FerronManager>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(rt) => rt,
        Err(_) => {
            warn!(
                "No active Tokio runtime; skipping config file watcher for {}",
                manager.config_path.display()
            );
            return Ok(None);
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<Event>>();

    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        // The receiver only goes away when the watcher task itself exits,
        // at which point `watcher` (held by that same task) has already
        // been dropped, so sends can't race a live watcher.
        let _ = tx.send(res);
    })
    .map_err(|e| FerronError::Watch(e.to_string()))?;

    // Watch the parent directory (not the file directly): this tolerates
    // the config file not existing yet, and also covers editors/writers
    // that replace the file via rename rather than in-place write.
    let watch_target = manager
        .config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    watcher
        .watch(&watch_target, RecursiveMode::NonRecursive)
        .map_err(|e| FerronError::Watch(e.to_string()))?;

    let watch_path = manager.config_path.clone();
    let watch_file_name = watch_path.file_name().map(|n| n.to_owned());

    // Hold only a `Weak` reference inside the spawned task; drop our own
    // strong reference immediately so the caller's `Arc` (and any it was
    // cloned from) is the only thing keeping the manager alive from here
    // on. See the doc comment above for why this matters.
    let manager_weak = Arc::downgrade(&manager);
    drop(manager);

    // Whether an event is one we actually care about: it names the watched
    // config file, and is a modification or (re)creation of it. Anything
    // else (e.g. `Access`/`Open`/`Close` events from something merely
    // *reading* the file, or events for unrelated files in the same
    // watched directory) is noise that must neither trigger a reload nor
    // reset the debounce window below.
    let is_relevant_event = move |event: &Event| -> bool {
        event
            .paths
            .iter()
            .any(|p| p.file_name() == watch_file_name.as_deref())
            && (event.kind.is_modify() || event.kind.is_create())
    };

    let join = rt.spawn(async move {
        // Keep the watcher alive for the lifetime of this task; dropping it
        // would stop delivery of further events.
        let _watcher = watcher;

        loop {
            let res = match rx.recv().await {
                Some(res) => res,
                None => break, // Sender gone; nothing left to watch for.
            };

            // The manager going away is this task's real shutdown signal:
            // once the last external `Arc<FerronManager>` is dropped,
            // there's no one left to reload for, so stop watching (dropping
            // `_watcher` below closes the channel and ends the task).
            let Some(manager) = manager_weak.upgrade() else {
                break;
            };

            let event = match res {
                Ok(event) => event,
                Err(e) => {
                    warn!("Config file watch error: {}", e);
                    continue;
                }
            };

            if !is_relevant_event(&event) {
                continue;
            }

            if !manager.auto_reload {
                continue;
            }

            // Debounce: a single logical "save" (or a config-management
            // tool applying a batch of changes) commonly produces several
            // raw filesystem events in quick succession. Wait for a short
            // quiet window with no further *relevant* events before
            // reloading, so a burst collapses into a single reload rather
            // than one per event. Irrelevant events (e.g. some other
            // process merely reading the file, which itself generates
            // `Access` events on the very same path) are drained without
            // resetting the window, so they can't stall the reload
            // indefinitely.
            let mut deadline = tokio::time::Instant::now() + CONFIG_WATCH_DEBOUNCE;
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break; // quiet window elapsed
                }
                match tokio::time::timeout(remaining, rx.recv()).await {
                    Ok(Some(Ok(ev))) if is_relevant_event(&ev) => {
                        // More real activity; push the window back out.
                        deadline = tokio::time::Instant::now() + CONFIG_WATCH_DEBOUNCE;
                    }
                    Ok(Some(_)) => continue, // irrelevant event; keep waiting out the window
                    Ok(None) => break,       // channel closed
                    Err(_) => break,         // timed out waiting for the next event
                }
            }

            info!(
                "Detected change to {}, reloading Ferron",
                watch_path.display()
            );
            // Deliberately `signal_reload()`, not `reload()`: this path runs
            // in reaction to a filesystem event on `config_path` itself, so
            // it must not regenerate-and-rewrite that same file (that would
            // be a self-perpetuating loop -- see `signal_reload()`'s doc
            // comment). Registry-driven regeneration happens elsewhere, in
            // response to actual registry state changes
            // (`register_backend`/`deregister_backend`), not here.
            if let Err(e) = manager.signal_reload().await {
                warn!("Failed to reload Ferron after config change: {}", e);
            }
        }
    });

    Ok(Some(join))
}

/// Convenience functions for common Ferron operations
pub mod helpers {
    use super::*;

    /// Generate a basic reverse proxy configuration
    pub fn reverse_proxy_config(
        domain: impl Into<String>,
        backend_url: impl Into<String>,
    ) -> Result<FerronConfig> {
        FerronConfig::builder()
            .domain(domain)
            .backend_url(backend_url)
            .tls_auto(true)
            .gzip(true)
            .build()
    }

    /// Generate a load-balanced configuration
    pub fn load_balanced_config(
        domain: impl Into<String>,
        backends: Vec<impl Into<String>>,
    ) -> Result<FerronConfig> {
        let mut lb = LoadBalancer::new();
        for backend in backends {
            lb = lb.backend(Backend::new(backend));
        }

        FerronConfig::builder()
            .domain(domain)
            .load_balancer(lb)
            .tls_auto(true)
            .gzip(true)
            .build()
    }

    /// Generate configuration for an Armature application
    pub fn armature_app_config(domain: impl Into<String>, app_port: u16) -> Result<FerronConfig> {
        use crate::config::{Location, RateLimitConfig};

        FerronConfig::builder()
            .domain(domain)
            .backend_url(format!("http://127.0.0.1:{}", app_port))
            .tls_auto(true)
            .gzip(true)
            // API routes with rate limiting
            .location(
                Location::new("/api")
                    .proxy(format!("http://127.0.0.1:{}/api", app_port))
                    .rate_limit(RateLimitConfig::new(100).burst(200)),
            )
            // WebSocket support
            .location(Location::new("/ws").proxy(format!("http://127.0.0.1:{}/ws", app_port)))
            // Health endpoint (no rate limit)
            .location(
                Location::new("/health").proxy(format!("http://127.0.0.1:{}/health", app_port)),
            )
            // Security headers
            .header("X-Frame-Options", "DENY")
            .header("X-Content-Type-Options", "nosniff")
            .header("X-XSS-Protection", "1; mode=block")
            .header("Referrer-Policy", "strict-origin-when-cross-origin")
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_proxy_config() {
        let config = helpers::reverse_proxy_config("example.com", "http://localhost:3000").unwrap();

        assert_eq!(config.domains, vec!["example.com"]);
        assert_eq!(
            config.backend.as_ref().map(|b| b.url.as_str()),
            Some("http://localhost:3000")
        );
        assert!(config.tls.is_some());
    }

    #[test]
    fn test_load_balanced_config() {
        let config = helpers::load_balanced_config(
            "example.com",
            vec!["http://localhost:3001", "http://localhost:3002"],
        )
        .unwrap();

        assert!(config.load_balancer.is_some());
        let lb = config.load_balancer.unwrap();
        assert_eq!(lb.backends.len(), 2);
    }

    #[test]
    fn test_armature_app_config() {
        let config = helpers::armature_app_config("api.example.com", 3000).unwrap();

        assert_eq!(config.domains, vec!["api.example.com"]);
        assert!(!config.locations.is_empty());
        assert!(config.headers.contains_key("X-Frame-Options"));
    }

    #[test]
    fn test_manager_builder() {
        // Note: This test doesn't actually start Ferron, just tests builder
        let config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        // Builder should work even without Ferron installed. This is a
        // plain (non-tokio) #[test] intentionally: build() must not panic
        // even when there's no active Tokio runtime to spawn the watcher
        // task on (it should just skip watching and log a warning).
        let result = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path("/tmp/test_ferron.conf")
            .config(config)
            .auto_reload(true)
            .auto_restart(true)
            .build();

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_build_spawns_watcher_when_auto_reload_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");
        let config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(config)
            .auto_reload(true)
            .build()
            .unwrap();

        assert!(
            manager.is_watching().await,
            "watch_handle must be populated when auto_reload is enabled and \
             a Tokio runtime is available"
        );
    }

    #[tokio::test]
    async fn test_build_does_not_spawn_watcher_without_auto_reload() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");
        let config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(config)
            .auto_reload(false)
            .build()
            .unwrap();

        assert!(!manager.is_watching().await);
    }

    #[tokio::test]
    async fn test_watcher_reload_does_not_regenerate_from_registry() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");

        let base_config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        let registry = ServiceRegistry::new();
        registry
            .register("svc", "http://127.0.0.1:9999")
            .await
            .unwrap();

        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(base_config)
            .service_registry(registry)
            .auto_reload(true)
            .build()
            .unwrap();

        assert!(manager.is_watching().await);

        // Simulate an external edit to the config file.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        tokio::fs::write(&config_path, "// externally modified\n")
            .await
            .unwrap();

        // The watcher should notice the change and call `signal_reload()`
        // (observable via `reload_count()`) *without* regenerating the
        // config from the service registry and writing it back out --
        // doing so would mean the watcher's own reaction to an external
        // edit rewrites the very file it's watching, re-triggering itself
        // forever (see
        // `test_watcher_does_not_self_perpetuate_reload_loop_with_registry`).
        let mut saw_reload = false;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if manager.reload_count() >= 1 {
                saw_reload = true;
                break;
            }
        }
        assert!(
            saw_reload,
            "expected the config file watcher to trigger a reload on an external edit"
        );

        // Give an (incorrect) self-rewrite a further chance to happen, then
        // confirm the on-disk content is still exactly what the external
        // edit wrote -- i.e. the watcher's reload path did not regenerate
        // and rewrite it from the registry.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(
            contents, "// externally modified\n",
            "the watcher's reload path must not rewrite the config file it just observed"
        );
    }

    /// Regression test for the watcher-task leak: the task spawned by
    /// `spawn_config_watcher` must hold only a `Weak<FerronManager>`, so
    /// once every external `Arc<FerronManager>` is dropped, the task
    /// notices (via a failed `upgrade()`) and exits -- dropping its
    /// `notify::Watcher`, which closes the channel. Against the original
    /// code (a strong `Arc` captured in the task, `while let Some(..) =
    /// rx.recv().await` with no other termination path), the task can
    /// never observe the manager going away, so `handle.is_finished()`
    /// never becomes true and this test times out / fails.
    #[tokio::test]
    async fn test_watcher_task_ends_when_manager_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");
        tokio::fs::write(&config_path, "// initial\n")
            .await
            .unwrap();

        let base_config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        // Build with auto_reload(false) so `build()` itself doesn't also
        // spawn a watcher -- we spawn our own directly below so we can
        // retain the raw `JoinHandle` after dropping the manager.
        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(base_config)
            .auto_reload(false)
            .build()
            .unwrap();

        let handle = spawn_config_watcher(Arc::clone(&manager))
            .unwrap()
            .expect("a Tokio runtime is available in this #[tokio::test]");

        assert!(!handle.is_finished());

        // Drop the only external strong reference to the manager. If the
        // watcher task holds a `Weak` (as fixed), the manager is actually
        // deallocated now; the task just hasn't noticed yet.
        drop(manager);

        // Nudge the watcher with a filesystem event so its `rx.recv()`
        // wakes up and attempts to `upgrade()` the weak reference.
        tokio::fs::write(&config_path, "// trigger\n")
            .await
            .unwrap();

        let mut finished = false;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if handle.is_finished() {
                finished = true;
                break;
            }
        }

        assert!(
            finished,
            "watcher task must terminate once the manager it watches for has been dropped"
        );
    }

    /// Regression test for `stop()`/`stop_watching()`: they must provide an
    /// explicit, deterministic shutdown path for the watcher task rather
    /// than relying solely on the manager being dropped (which, absent
    /// further filesystem activity, could leave the task parked on
    /// `recv()` indefinitely even after the fix above).
    #[tokio::test]
    async fn test_stop_terminates_the_watcher_task() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");
        let config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(config)
            .auto_reload(true)
            .build()
            .unwrap();

        assert!(manager.is_watching().await);

        // The process was never started, so `stop()`'s call into
        // `self.process.stop()` will return `NotRunning` -- but tearing
        // down the watcher task must happen regardless, and first.
        let _ = manager.stop().await;

        assert!(
            !manager.is_watching().await,
            "stop() must abort/join the watcher task and clear watch_handle"
        );
    }

    /// Regression test for the watcher/registry reload-loop bug: with a
    /// `ServiceRegistry` configured, the watcher's reaction to a config-file
    /// change used to call `manager.reload()`, which -- because a registry
    /// is present -- regenerates the config *and writes it back to the very
    /// file being watched*. That write is itself a relevant filesystem
    /// event, so the watcher fires again, regenerates again, writes again,
    /// forever, with no further external input after the first edit.
    ///
    /// A single external touch to the config file must settle to a bounded,
    /// stable `reload_count()` rather than climbing indefinitely. This is
    /// checked by sampling the count twice, several debounce windows apart:
    /// a fixed reaction to the one external write settles and holds; a
    /// self-perpetuating loop keeps incrementing roughly once per debounce
    /// window forever, so the two samples would differ.
    #[tokio::test]
    async fn test_watcher_does_not_self_perpetuate_reload_loop_with_registry() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");
        tokio::fs::write(&config_path, "// initial\n")
            .await
            .unwrap();

        let base_config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        let registry = ServiceRegistry::new();
        registry
            .register("svc", "http://127.0.0.1:9999")
            .await
            .unwrap();

        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(base_config)
            .service_registry(registry)
            .auto_reload(true)
            .build()
            .unwrap();

        assert!(manager.is_watching().await);

        // Let the watcher fully register before triggering anything.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // The one and only externally-caused event in this test.
        tokio::fs::write(&config_path, "// externally modified\n")
            .await
            .unwrap();

        let window_ms = CONFIG_WATCH_DEBOUNCE.as_millis() as u64;

        tokio::time::sleep(std::time::Duration::from_millis(window_ms * 6)).await;
        let count_a = manager.reload_count();

        tokio::time::sleep(std::time::Duration::from_millis(window_ms * 6)).await;
        let count_b = manager.reload_count();

        assert!(
            count_a <= 2,
            "a single external edit must produce at most one reload, got {count_a}"
        );
        assert_eq!(
            count_a, count_b,
            "reload_count must stabilize after the single external edit rather than \
             climb unboundedly (sampled {count_a} then {count_b} {window_ms}ms later)"
        );
    }

    /// Regression test for the reload-storm/thrash issue: every matching
    /// filesystem event used to trigger an immediate `manager.reload()`
    /// call, so several rapid writes to the config file (e.g. an editor's
    /// save, or a config-management tool applying a batch of changes)
    /// produced one real reload signal per raw event. The watcher must
    /// instead debounce/coalesce a burst of events into a single reload.
    #[tokio::test]
    async fn test_watcher_debounces_rapid_successive_writes_into_one_reload() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ferron.conf");
        tokio::fs::write(&config_path, "// initial\n")
            .await
            .unwrap();

        let base_config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        let manager = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path(&config_path)
            .config(base_config)
            .auto_reload(true)
            .build()
            .unwrap();

        assert!(manager.is_watching().await);

        // Give the watcher a moment to be fully registered before we start
        // writing (mirrors the warm-up sleep in the existing reload test).
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Write the config file several times in quick succession, well
        // within the debounce window -- this should collapse into exactly
        // one reload.
        for i in 0..5 {
            tokio::fs::write(&config_path, format!("// edit {}\n", i))
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // Bounded poll: wait for the debounce window to close and the
        // (single) reload to fire, then confirm the count is exactly one.
        let mut last = manager.reload_count();
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            last = manager.reload_count();
        }

        assert_eq!(
            last, 1,
            "5 rapid successive writes within the debounce window must collapse into a single reload"
        );
    }
}
