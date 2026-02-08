//! Ferron process management
//!
//! This module provides utilities for managing the Ferron server process,
//! including starting, stopping, and reloading configurations.

use crate::error::{FerronError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Process status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    /// Process is not running
    Stopped,
    /// Process is starting
    Starting,
    /// Process is running normally
    Running,
    /// Process is stopping
    Stopping,
    /// Process encountered an error
    Error,
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self::Stopped
    }
}

/// Configuration for the Ferron process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    /// Path to the Ferron binary
    pub binary_path: PathBuf,
    /// Path to the configuration file
    pub config_path: PathBuf,
    /// Working directory
    pub working_dir: Option<PathBuf>,
    /// Additional command line arguments
    pub extra_args: Vec<String>,
    /// Environment variables
    pub env_vars: std::collections::HashMap<String, String>,
    /// PID file path
    pub pid_file: Option<PathBuf>,
    /// Stdout log file
    pub stdout_log: Option<PathBuf>,
    /// Stderr log file
    pub stderr_log: Option<PathBuf>,
    /// Restart on crash
    pub auto_restart: bool,
    /// Maximum restart attempts
    pub max_restarts: u32,
    /// Restart delay in milliseconds
    pub restart_delay_ms: u64,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            binary_path: PathBuf::from("ferron"),
            config_path: PathBuf::from("/etc/ferron/ferron.conf"),
            working_dir: None,
            extra_args: Vec::new(),
            env_vars: std::collections::HashMap::new(),
            pid_file: Some(PathBuf::from("/var/run/ferron.pid")),
            stdout_log: None,
            stderr_log: None,
            auto_restart: true,
            max_restarts: 3,
            restart_delay_ms: 1000,
        }
    }
}

impl ProcessConfig {
    /// Create a new process configuration
    pub fn new(binary_path: impl Into<PathBuf>, config_path: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: binary_path.into(),
            config_path: config_path.into(),
            ..Default::default()
        }
    }

    /// Set the working directory
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    /// Add an extra command line argument
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Add an environment variable
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Set the PID file path
    pub fn pid_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.pid_file = Some(path.into());
        self
    }

    /// Set stdout log file
    pub fn stdout_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.stdout_log = Some(path.into());
        self
    }

    /// Set stderr log file
    pub fn stderr_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.stderr_log = Some(path.into());
        self
    }

    /// Enable/disable auto restart
    pub fn auto_restart(mut self, enabled: bool) -> Self {
        self.auto_restart = enabled;
        self
    }

    /// Set maximum restart attempts
    pub fn max_restarts(mut self, max: u32) -> Self {
        self.max_restarts = max;
        self
    }

    /// Set restart delay
    pub fn restart_delay(mut self, delay_ms: u64) -> Self {
        self.restart_delay_ms = delay_ms;
        self
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if !self.config_path.exists() {
            return Err(FerronError::Config(format!(
                "Configuration file not found: {}",
                self.config_path.display()
            )));
        }

        // Check if binary exists (might be in PATH)
        if self.binary_path.is_absolute() && !self.binary_path.exists() {
            return Err(FerronError::ProcessNotFound(
                self.binary_path.display().to_string(),
            ));
        }

        Ok(())
    }
}

/// Ferron process handle
#[derive(Debug)]
pub struct FerronProcess {
    /// Process configuration
    config: ProcessConfig,
    /// Child process handle
    child: Arc<RwLock<Option<Child>>>,
    /// Current status
    status: Arc<RwLock<ProcessStatus>>,
    /// Process ID
    pid: Arc<RwLock<Option<u32>>>,
    /// Restart count
    restart_count: Arc<RwLock<u32>>,
}

impl FerronProcess {
    /// Create a new Ferron process handle
    pub fn new(config: ProcessConfig) -> Self {
        Self {
            config,
            child: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(ProcessStatus::Stopped)),
            pid: Arc::new(RwLock::new(None)),
            restart_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Get current process status
    pub async fn status(&self) -> ProcessStatus {
        *self.status.read().await
    }

    /// Get the process ID if running
    pub async fn pid(&self) -> Option<u32> {
        *self.pid.read().await
    }

    /// Start the Ferron process
    pub async fn start(&self) -> Result<()> {
        // Check if already running
        if *self.status.read().await == ProcessStatus::Running {
            let pid = self.pid.read().await.unwrap_or(0);
            return Err(FerronError::AlreadyRunning(pid));
        }

        *self.status.write().await = ProcessStatus::Starting;
        info!("Starting Ferron server...");

        let mut cmd = Command::new(&self.config.binary_path);
        cmd.arg("-c").arg(&self.config.config_path);

        // Add extra arguments
        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        // Set working directory
        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }

        // Set environment variables
        for (key, value) in &self.config.env_vars {
            cmd.env(key, value);
        }

        // Configure stdio
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Start the process
        let mut child = cmd.spawn().map_err(|e| {
            *futures::executor::block_on(self.status.write()) = ProcessStatus::Error;
            FerronError::StartFailed(e.to_string())
        })?;

        let pid = child.id();
        *self.pid.write().await = pid;

        // Write PID file
        if let Some(ref pid_path) = self.config.pid_file
            && let Some(pid) = pid
            && let Err(e) = std::fs::write(pid_path, pid.to_string())
        {
            warn!("Failed to write PID file: {}", e);
        }

        // Spawn stdout/stderr handlers
        if let Some(stdout) = child.stdout.take() {
            let stdout_log = self.config.stdout_log.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!("[ferron] {}", line);
                    if let Some(ref log_path) = stdout_log
                        && let Err(e) = tokio::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(log_path)
                            .await
                            .and_then(|mut f| {
                                use tokio::io::AsyncWriteExt;
                                futures::executor::block_on(
                                    f.write_all(format!("{}\n", line).as_bytes()),
                                )
                            })
                    {
                        warn!("Failed to write to stdout log: {}", e);
                    }
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let stderr_log = self.config.stderr_log.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!("[ferron] {}", line);
                    if let Some(ref log_path) = stderr_log
                        && let Err(e) = tokio::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(log_path)
                            .await
                            .and_then(|mut f| {
                                use tokio::io::AsyncWriteExt;
                                futures::executor::block_on(
                                    f.write_all(format!("{}\n", line).as_bytes()),
                                )
                            })
                    {
                        warn!("Failed to write to stderr log: {}", e);
                    }
                }
            });
        }

        *self.child.write().await = Some(child);
        *self.status.write().await = ProcessStatus::Running;

        info!("Ferron server started with PID {:?}", pid);
        Ok(())
    }

    /// Stop the Ferron process
    pub async fn stop(&self) -> Result<()> {
        let current_status = *self.status.read().await;
        if current_status != ProcessStatus::Running {
            return Err(FerronError::NotRunning);
        }

        *self.status.write().await = ProcessStatus::Stopping;
        info!("Stopping Ferron server...");

        let mut child_guard = self.child.write().await;
        if let Some(ref mut child) = *child_guard {
            // Try graceful shutdown first (SIGTERM)
            #[cfg(unix)]
            {
                use nix::sys::signal::{self, Signal};
                use nix::unistd::Pid;
                if let Some(pid) = child.id() {
                    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                }
            }

            // Wait for process to exit (with timeout)
            let timeout =
                tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await;

            match timeout {
                Ok(Ok(_)) => {
                    info!("Ferron server stopped gracefully");
                }
                Ok(Err(e)) => {
                    error!("Error waiting for Ferron to stop: {}", e);
                }
                Err(_) => {
                    // Timeout - force kill
                    warn!("Ferron did not stop gracefully, forcing kill");
                    child.kill().await.ok();
                }
            }
        }

        *child_guard = None;
        *self.pid.write().await = None;
        *self.status.write().await = ProcessStatus::Stopped;

        // Remove PID file
        if let Some(ref pid_path) = self.config.pid_file
            && pid_path.exists()
            && let Err(e) = std::fs::remove_file(pid_path)
        {
            warn!("Failed to remove PID file: {}", e);
        }

        info!("Ferron server stopped");
        Ok(())
    }

    /// Restart the Ferron process
    pub async fn restart(&self) -> Result<()> {
        info!("Restarting Ferron server...");
        if *self.status.read().await == ProcessStatus::Running {
            self.stop().await?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        self.start().await
    }

    /// Reload Ferron configuration (SIGHUP)
    pub async fn reload(&self) -> Result<()> {
        let current_status = *self.status.read().await;
        if current_status != ProcessStatus::Running {
            return Err(FerronError::NotRunning);
        }

        info!("Reloading Ferron configuration...");

        #[cfg(unix)]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;

            let pid = self.pid.read().await.ok_or(FerronError::NotRunning)?;
            signal::kill(Pid::from_raw(pid as i32), Signal::SIGHUP)
                .map_err(|e| FerronError::ReloadFailed(e.to_string()))?;
        }

        #[cfg(not(unix))]
        {
            // On non-Unix systems, restart the process
            self.restart().await?;
        }

        info!("Ferron configuration reloaded");
        Ok(())
    }

    /// Wait for the process to exit
    pub async fn wait(&self) -> Result<i32> {
        let mut child_guard = self.child.write().await;
        if let Some(ref mut child) = *child_guard {
            let status = child
                .wait()
                .await
                .map_err(|e| FerronError::Process(e.to_string()))?;
            *self.status.write().await = ProcessStatus::Stopped;
            Ok(status.code().unwrap_or(-1))
        } else {
            Err(FerronError::NotRunning)
        }
    }

    /// Check if the process is still running
    pub async fn is_running(&self) -> bool {
        let child_guard = self.child.read().await;
        if child_guard.is_some() {
            // Check if process is still alive
            if let Some(pid) = *self.pid.read().await {
                #[cfg(unix)]
                {
                    use nix::sys::signal::{self, Signal};
                    use nix::unistd::Pid;
                    // Signal 0 just checks if process exists
                    return signal::kill(Pid::from_raw(pid as i32), Signal::SIGCONT).is_ok();
                }
                #[cfg(not(unix))]
                {
                    return true; // Assume running on non-Unix
                }
            }
        }
        false
    }

    /// Start with auto-restart on crash
    pub async fn start_with_supervision(self: Arc<Self>) -> Result<tokio::task::JoinHandle<()>> {
        self.start().await?;

        let process = self.clone();
        let handle = tokio::spawn(async move {
            loop {
                // Wait for process to exit
                match process.wait().await {
                    Ok(exit_code) => {
                        warn!("Ferron exited with code {}", exit_code);
                    }
                    Err(e) => {
                        error!("Error waiting for Ferron: {}", e);
                        break;
                    }
                }

                // Check if we should restart
                if !process.config.auto_restart {
                    info!("Auto-restart disabled, not restarting");
                    break;
                }

                let restart_count = *process.restart_count.read().await;
                if restart_count >= process.config.max_restarts {
                    error!(
                        "Maximum restart attempts ({}) reached, not restarting",
                        process.config.max_restarts
                    );
                    break;
                }

                *process.restart_count.write().await = restart_count + 1;
                info!(
                    "Restarting Ferron (attempt {}/{})",
                    restart_count + 1,
                    process.config.max_restarts
                );

                tokio::time::sleep(std::time::Duration::from_millis(
                    process.config.restart_delay_ms,
                ))
                .await;

                if let Err(e) = process.start().await {
                    error!("Failed to restart Ferron: {}", e);
                    break;
                }
            }
        });

        Ok(handle)
    }
}

/// Check if Ferron is installed and available
pub async fn check_ferron_installed(path: Option<&Path>) -> Result<String> {
    let binary = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("ferron"));

    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .await
        .map_err(|e| FerronError::ProcessNotFound(format!("{}: {}", binary.display(), e)))?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout);
        Ok(version.trim().to_string())
    } else {
        Err(FerronError::ProcessNotFound(format!(
            "Ferron not found or failed to execute: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_config_builder() {
        let config = ProcessConfig::new("/usr/bin/ferron", "/etc/ferron/ferron.conf")
            .working_dir("/var/www")
            .arg("--verbose")
            .env("RUST_LOG", "debug")
            .auto_restart(true)
            .max_restarts(5);

        assert_eq!(config.binary_path, PathBuf::from("/usr/bin/ferron"));
        assert_eq!(config.config_path, PathBuf::from("/etc/ferron/ferron.conf"));
        assert_eq!(config.working_dir, Some(PathBuf::from("/var/www")));
        assert_eq!(config.extra_args, vec!["--verbose".to_string()]);
        assert_eq!(config.env_vars.get("RUST_LOG"), Some(&"debug".to_string()));
        assert!(config.auto_restart);
        assert_eq!(config.max_restarts, 5);
    }

    #[tokio::test]
    async fn test_process_status() {
        let config = ProcessConfig::default();
        let process = FerronProcess::new(config);

        assert_eq!(process.status().await, ProcessStatus::Stopped);
        assert!(process.pid().await.is_none());
    }
}
