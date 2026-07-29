//! Comprehensive logging system for Armature
//!
//! Provides highly configurable logging with multiple formats, outputs,
//! and log levels. Defaults to JSON output to STDOUT.
//!
//! # Features
//!
//! - **Multiple Formats:** JSON, Plain, Pretty, Compact
//! - **Multiple Outputs:** STDOUT, STDERR, File, Rolling File
//! - **Log Levels:** TRACE, DEBUG, INFO, WARN, ERROR
//! - **Structured Logging:** Key-value pairs for context
//! - **Request Logging:** Automatic HTTP request/response logging
//! - **Performance:** Low overhead with async support
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```no_run
//! use armature_core::logging::*;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Initialize with default JSON logging to STDOUT
//!     let _guard = LogConfig::default().init();
//!
//!     info!("Application started");
//!     debug!("Debug information");
//!     error!("An error occurred");
//! }
//! ```
//!
//! ## Custom Configuration
//!
//! ```no_run
//! use armature_core::logging::*;
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = LogConfig::new()
//!         .level(LogLevel::Debug)
//!         .format(LogFormat::Pretty)
//!         .output(LogOutput::Stdout)
//!         .with_timestamps(true)
//!         .with_thread_ids(true)
//!         .with_targets(true);
//!
//!     let _guard = config.init();
//!
//!     info!("Application configured with pretty logging");
//! }
//! ```
//!
//! ## File Logging with Rotation
//!
//! ```no_run
//! use armature_core::logging::*;
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = LogConfig::new()
//!         .format(LogFormat::Json)
//!         .output(LogOutput::RollingFile {
//!             directory: "logs".to_string(),
//!             prefix: "app".to_string(),
//!             rotation: Rotation::Daily,
//!         });
//!
//!     let _guard = config.init();
//!
//!     info!("Logging to rotating file");
//! }
//! ```

use std::io;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

// Re-export tracing and tracing_appender for convenience
pub use tracing::{debug, error, info, trace, warn};
pub use tracing_appender;

/// Log level for filtering messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Most verbose - shows all logs
    Trace,
    /// Debug information for development
    Debug,
    /// General information about application flow
    Info,
    /// Warning messages for potential issues
    Warn,
    /// Error messages for failures
    Error,
}

impl LogLevel {
    /// Convert to tracing Level
    pub fn to_tracing_level(&self) -> Level {
        match self {
            LogLevel::Trace => Level::TRACE,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Info => Level::INFO,
            LogLevel::Warn => Level::WARN,
            LogLevel::Error => Level::ERROR,
        }
    }

    /// Convert to string for EnvFilter
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Output format for log messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// JSON format (default) - structured, machine-readable
    Json,
    /// Plain text format - simple, human-readable
    Plain,
    /// Pretty format - colored, formatted for development
    Pretty,
    /// Compact format - minimal output
    Compact,
}

/// Output destination for logs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogOutput {
    /// Write to STDOUT (default)
    Stdout,
    /// Write to STDERR
    Stderr,
    /// Write to a single file
    File(String),
    /// Write to rotating log files
    RollingFile {
        directory: String,
        prefix: String,
        rotation: Rotation,
    },
}

/// File rotation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// Rotate every minute (for testing)
    Minutely,
    /// Rotate every hour
    Hourly,
    /// Rotate daily
    Daily,
    /// Never rotate
    Never,
}

impl Rotation {
    /// Convert to tracing_appender Rotation
    fn as_tracing_rotation(self) -> tracing_appender::rolling::Rotation {
        match self {
            Rotation::Minutely => tracing_appender::rolling::Rotation::MINUTELY,
            Rotation::Hourly => tracing_appender::rolling::Rotation::HOURLY,
            Rotation::Daily => tracing_appender::rolling::Rotation::DAILY,
            Rotation::Never => tracing_appender::rolling::Rotation::NEVER,
        }
    }
}

/// Comprehensive logging configuration
///
/// # Examples
///
/// ```no_run
/// use armature_core::logging::*;
///
/// // Default configuration (JSON to STDOUT)
/// let config = LogConfig::default();
///
/// // Custom configuration
/// let config = LogConfig::new()
///     .level(LogLevel::Debug)
///     .format(LogFormat::Pretty)
///     .output(LogOutput::Stdout)
///     .with_timestamps(true)
///     .with_thread_ids(true);
/// ```
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Log level filter
    pub level: LogLevel,
    /// Output format
    pub format: LogFormat,
    /// Output destination
    pub output: LogOutput,
    /// Include timestamps
    pub timestamps: bool,
    /// Include thread IDs
    pub thread_ids: bool,
    /// Include target (module path)
    pub targets: bool,
    /// Include file and line numbers
    pub file_line: bool,
    /// Include span information
    pub spans: bool,
    /// Enable ANSI colors (for terminal output)
    pub colors: bool,
    /// Custom environment filter (overrides level if set)
    pub env_filter: Option<String>,
}

impl LogConfig {
    /// Create a new logging configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set log level
    ///
    /// # Examples
    ///
    /// ```
    /// # use armature_core::logging::*;
    /// let config = LogConfig::new().level(LogLevel::Debug);
    /// ```
    pub fn level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }

    /// Set output format
    ///
    /// # Examples
    ///
    /// ```
    /// # use armature_core::logging::*;
    /// let config = LogConfig::new().format(LogFormat::Json);
    /// ```
    pub fn format(mut self, format: LogFormat) -> Self {
        self.format = format;
        self
    }

    /// Set output destination
    ///
    /// # Examples
    ///
    /// ```
    /// # use armature_core::logging::*;
    /// let config = LogConfig::new().output(LogOutput::Stdout);
    /// ```
    pub fn output(mut self, output: LogOutput) -> Self {
        self.output = output;
        self
    }

    /// Enable or disable timestamps
    pub fn with_timestamps(mut self, enable: bool) -> Self {
        self.timestamps = enable;
        self
    }

    /// Enable or disable thread IDs
    pub fn with_thread_ids(mut self, enable: bool) -> Self {
        self.thread_ids = enable;
        self
    }

    /// Enable or disable target (module path)
    pub fn with_targets(mut self, enable: bool) -> Self {
        self.targets = enable;
        self
    }

    /// Enable or disable file and line numbers
    pub fn with_file_line(mut self, enable: bool) -> Self {
        self.file_line = enable;
        self
    }

    /// Enable or disable span information
    pub fn with_spans(mut self, enable: bool) -> Self {
        self.spans = enable;
        self
    }

    /// Enable or disable ANSI colors
    pub fn with_colors(mut self, enable: bool) -> Self {
        self.colors = enable;
        self
    }

    /// Set custom environment filter
    ///
    /// This allows complex filtering like "armature=debug,tower=info"
    ///
    /// # Examples
    ///
    /// ```
    /// # use armature_core::logging::*;
    /// let config = LogConfig::new()
    ///     .with_env_filter("armature=debug,hyper=info");
    /// ```
    pub fn with_env_filter(mut self, filter: impl Into<String>) -> Self {
        self.env_filter = Some(filter.into());
        self
    }

    /// Initialize the logging system
    ///
    /// Returns a guard that must be kept alive for the duration of the program.
    /// When dropped, the logger will flush remaining logs.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_core::logging::*;
    /// #[tokio::main]
    /// async fn main() {
    ///     let _guard = LogConfig::default().init();
    ///     info!("Logging initialized");
    ///     // ... rest of application
    /// } // guard dropped here, logs flushed
    /// ```
    pub fn init(self) -> Option<WorkerGuard> {
        // Create environment filter
        let env_filter = if let Some(filter_str) = &self.env_filter {
            EnvFilter::try_new(filter_str).unwrap_or_else(|_| {
                EnvFilter::new(format!(
                    "{}={}",
                    env!("CARGO_PKG_NAME"),
                    self.level.as_str()
                ))
            })
        } else {
            // Try to read from RUST_LOG env var, otherwise use config level
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(self.level.as_str()))
        };

        // Create the writer based on output configuration
        match &self.output {
            LogOutput::Stdout => {
                let (non_blocking, guard) = tracing_appender::non_blocking(io::stdout());
                self.init_with_writer(non_blocking, env_filter);
                Some(guard)
            }
            LogOutput::Stderr => {
                let (non_blocking, guard) = tracing_appender::non_blocking(io::stderr());
                self.init_with_writer(non_blocking, env_filter);
                Some(guard)
            }
            LogOutput::File(path) => {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("Failed to open log file");
                let (non_blocking, guard) = tracing_appender::non_blocking(file);
                self.init_with_writer(non_blocking, env_filter);
                Some(guard)
            }
            LogOutput::RollingFile {
                directory,
                prefix,
                rotation,
            } => {
                let file_appender = tracing_appender::rolling::RollingFileAppender::new(
                    rotation.as_tracing_rotation(),
                    directory,
                    prefix,
                );
                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                self.init_with_writer(non_blocking, env_filter);
                Some(guard)
            }
        }
    }

    /// Initialize with a specific writer
    fn init_with_writer<W>(&self, writer: W, env_filter: EnvFilter)
    where
        W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
    {
        let fmt_span = if self.spans {
            FmtSpan::CLOSE
        } else {
            FmtSpan::NONE
        };

        match self.format {
            LogFormat::Json => {
                let layer = fmt::layer()
                    .json()
                    .with_writer(writer)
                    .with_current_span(self.spans)
                    .with_span_list(self.spans)
                    .with_target(self.targets)
                    .with_thread_ids(self.thread_ids)
                    .with_file(self.file_line)
                    .with_line_number(self.file_line)
                    .with_span_events(fmt_span);

                if self.timestamps {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .init();
                } else {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer.without_time())
                        .init();
                }
            }
            LogFormat::Plain => {
                let layer = fmt::layer()
                    .with_writer(writer)
                    .with_target(self.targets)
                    .with_thread_ids(self.thread_ids)
                    .with_file(self.file_line)
                    .with_line_number(self.file_line)
                    .with_ansi(self.colors)
                    .with_span_events(fmt_span);

                if self.timestamps {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .init();
                } else {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer.without_time())
                        .init();
                }
            }
            LogFormat::Pretty => {
                let layer = fmt::layer()
                    .pretty()
                    .with_writer(writer)
                    .with_target(self.targets)
                    .with_thread_ids(self.thread_ids)
                    .with_file(self.file_line)
                    .with_line_number(self.file_line)
                    .with_ansi(self.colors)
                    .with_span_events(fmt_span);

                if self.timestamps {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .init();
                } else {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer.without_time())
                        .init();
                }
            }
            LogFormat::Compact => {
                let layer = fmt::layer()
                    .compact()
                    .with_writer(writer)
                    .with_target(self.targets)
                    .with_thread_ids(self.thread_ids)
                    .with_file(false) // Compact doesn't show file/line
                    .with_line_number(false)
                    .with_ansi(self.colors)
                    .with_span_events(fmt_span);

                if self.timestamps {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .init();
                } else {
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer.without_time())
                        .init();
                }
            }
        }
    }
}

impl Default for LogConfig {
    /// Default configuration: JSON format to STDOUT at INFO level
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Json,
            output: LogOutput::Stdout,
            timestamps: true,
            thread_ids: false,
            targets: true,
            file_line: false,
            spans: false,
            colors: false, // Disabled for JSON
            env_filter: None,
        }
    }
}

impl LogConfig {
    /// Create logging configuration from environment variables.
    ///
    /// Checks the following environment variables:
    /// - `ARMATURE_DEBUG=1` - Enable debug level logging
    /// - `ARMATURE_LOG_LEVEL=trace|debug|info|warn|error` - Set log level
    /// - `ARMATURE_LOG_FORMAT=json|pretty|plain|compact` - Set output format
    /// - `ARMATURE_LOG_COLOR=1|0` - Enable/disable colors
    /// - `ARMATURE_LOG_TIMESTAMPS=1|0` - Enable/disable timestamps
    /// - `ARMATURE_LOG_TARGETS=1|0` - Enable/disable module targets
    ///
    /// # Example
    ///
    /// ```rust
    /// # use armature_core::logging::LogConfig;
    /// // With ARMATURE_DEBUG=1 set in environment
    /// let config = LogConfig::from_env();
    /// // config.level will be LogLevel::Debug
    /// ```
    pub fn from_env() -> Self {
        let debug_enabled = std::env::var("ARMATURE_DEBUG")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        let level = std::env::var("ARMATURE_LOG_LEVEL")
            .ok()
            .and_then(|s| match s.to_lowercase().as_str() {
                "trace" => Some(LogLevel::Trace),
                "debug" => Some(LogLevel::Debug),
                "info" => Some(LogLevel::Info),
                "warn" | "warning" => Some(LogLevel::Warn),
                "error" => Some(LogLevel::Error),
                _ => None,
            })
            .unwrap_or(if debug_enabled {
                LogLevel::Debug
            } else {
                LogLevel::Info
            });

        let format = std::env::var("ARMATURE_LOG_FORMAT")
            .ok()
            .and_then(|s| match s.to_lowercase().as_str() {
                "json" => Some(LogFormat::Json),
                "pretty" => Some(LogFormat::Pretty),
                "plain" => Some(LogFormat::Plain),
                "compact" => Some(LogFormat::Compact),
                _ => None,
            })
            .unwrap_or(if debug_enabled {
                LogFormat::Pretty
            } else {
                LogFormat::Json
            });

        let colors = std::env::var("ARMATURE_LOG_COLOR")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(debug_enabled); // Enable colors in debug mode

        let timestamps = std::env::var("ARMATURE_LOG_TIMESTAMPS")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(true);

        let targets = std::env::var("ARMATURE_LOG_TARGETS")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(true);

        let file_line = std::env::var("ARMATURE_LOG_FILE_LINE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(debug_enabled); // Show file/line in debug mode

        Self {
            level,
            format,
            output: LogOutput::Stderr, // Use stderr for logs (stdout for app output)
            timestamps,
            thread_ids: false,
            targets,
            file_line,
            spans: debug_enabled, // Show spans in debug mode
            colors,
            env_filter: None,
        }
    }

    /// Check if debug mode is enabled via ARMATURE_DEBUG environment variable.
    pub fn is_debug_enabled() -> bool {
        std::env::var("ARMATURE_DEBUG")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_conversion() {
        assert_eq!(LogLevel::Trace.to_tracing_level(), Level::TRACE);
        assert_eq!(LogLevel::Debug.to_tracing_level(), Level::DEBUG);
        assert_eq!(LogLevel::Info.to_tracing_level(), Level::INFO);
        assert_eq!(LogLevel::Warn.to_tracing_level(), Level::WARN);
        assert_eq!(LogLevel::Error.to_tracing_level(), Level::ERROR);
    }

    #[test]
    fn test_log_level_string() {
        assert_eq!(LogLevel::Trace.as_str(), "trace");
        assert_eq!(LogLevel::Debug.as_str(), "debug");
        assert_eq!(LogLevel::Info.as_str(), "info");
        assert_eq!(LogLevel::Warn.as_str(), "warn");
        assert_eq!(LogLevel::Error.as_str(), "error");
    }

    #[test]
    fn test_default_config() {
        let config = LogConfig::default();
        assert_eq!(config.level, LogLevel::Info);
        assert_eq!(config.format, LogFormat::Json);
        assert_eq!(config.output, LogOutput::Stdout);
        assert!(config.timestamps);
        assert!(config.targets);
    }

    #[test]
    fn test_config_builder() {
        let config = LogConfig::new()
            .level(LogLevel::Debug)
            .format(LogFormat::Pretty)
            .with_colors(true)
            .with_targets(false);

        assert_eq!(config.level, LogLevel::Debug);
        assert_eq!(config.format, LogFormat::Pretty);
        assert!(config.colors);
        assert!(!config.targets);
    }
}
