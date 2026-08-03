//! Armature Logging Framework
//!
//! Provides structured logging for the Armature framework with JSON output
//! by default, and configurable pretty-printing for development.
//!
//! # Features
//!
//! - **JSON by default**: Production-ready structured logging
//! - **Pretty printing**: Human-readable output for development
//! - **Environment-controlled**: Configure via environment variables
//! - **Runtime-gated**: `debug!`/`trace!` are skipped via a cheap runtime
//!   check (`is_debug_enabled()`/`is_level_enabled()`), not compiled out —
//!   the branch and its argument formatting are always present in release
//!   builds
//! - **Runtime configurable**: Change format/level at runtime
//!
//! # Quick Start
//!
//! ```rust
//! use armature_log::{debug, info, warn, error};
//!
//! // Default: JSON output
//! info!("Server started on port {}", 8080);
//! // {"timestamp":"2024-12-20T12:00:00Z","level":"INFO","target":"my_app","message":"Server started on port 8080"}
//!
//! // With target
//! debug!(target: "armature::router", "Matching route: {}", "/api/users");
//! ```
//!
//! # Switching to Pretty Logging
//!
//! ## Option 1: Environment Variable (Recommended)
//!
//! ```bash
//! # Pretty format for development
//! ARMATURE_LOG_FORMAT=pretty cargo run
//!
//! # JSON format for production (default)
//! cargo run
//!
//! # Compact format
//! ARMATURE_LOG_FORMAT=compact cargo run
//! ```
//!
//! ## Option 2: Programmatic Configuration
//!
//! ```rust,no_run
//! use armature_log::{configure, Format, Level};
//!
//! // Configure for development
//! configure()
//!     .format(Format::Pretty)
//!     .level(Level::Debug)
//!     .color(true)
//!     .apply();
//!
//! // Or use presets
//! armature_log::preset_development();  // Pretty + Debug + Colors
//! armature_log::preset_production();   // JSON + Info
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Values | Default | Description |
//! |----------|--------|---------|-------------|
//! | `ARMATURE_DEBUG` | `1`, `true` | `false` | Enable debug logging |
//! | `ARMATURE_LOG_LEVEL` | `trace`, `debug`, `info`, `warn`, `error` | `info` | Minimum log level |
//! | `ARMATURE_LOG_FORMAT` | `json`, `pretty`, `compact` | `json` | Output format |
//! | `ARMATURE_LOG_COLOR` | `1`, `true`, `0`, `false` | auto-detect | Enable ANSI colors |
//! | `ARMATURE_LOG_TIMESTAMPS` | `1`, `0` | `1` | Include timestamps |
//! | `ARMATURE_LOG_MODULE` | `1`, `0` | `1` | Include module path |
//!
//! # Output Formats
//!
//! ## JSON (Default)
//! ```text
//! {"timestamp":"2024-12-20T12:00:00Z","level":"INFO","target":"my_app","message":"Server started"}
//! ```
//!
//! ## Pretty
//! ```text
//! 2024-12-20 12:00:00.123 INFO  [my_app] Server started
//! ```
//!
//! ## Compact
//! ```text
//! 12:00:00 I my_app: Server started
//! ```

use once_cell::sync::Lazy;
use std::env;
use std::io::Write;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

// ============================================================================
// Log Levels
// ============================================================================

/// Log level for Armature logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    /// Trace level (most verbose)
    Trace = 0,
    /// Debug level
    Debug = 1,
    /// Info level
    Info = 2,
    /// Warning level
    Warn = 3,
    /// Error level (least verbose)
    Error = 4,
    /// Off (no logging)
    Off = 5,
}

impl Level {
    /// Parse level from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "trace" => Some(Level::Trace),
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            "off" | "none" => Some(Level::Off),
            _ => None,
        }
    }

    /// Get level name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Off => "OFF",
        }
    }

    /// Get colored level name (if color feature enabled).
    #[cfg(feature = "color")]
    pub fn colored(&self) -> colored::ColoredString {
        use colored::Colorize;
        match self {
            Level::Trace => "TRACE".magenta(),
            Level::Debug => "DEBUG".blue(),
            Level::Info => "INFO".green(),
            Level::Warn => "WARN".yellow(),
            Level::Error => "ERROR".red().bold(),
            Level::Off => "OFF".white(),
        }
    }
}

impl std::str::FromStr for Level {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Log Format
// ============================================================================

/// Output format for log messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Format {
    /// Pretty format with colors (default for TTY)
    Pretty = 0,
    /// Compact single-line format
    Compact = 1,
    /// JSON format for structured logging
    Json = 2,
}

impl Format {
    /// Parse format from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pretty" => Some(Format::Pretty),
            "compact" => Some(Format::Compact),
            "json" => Some(Format::Json),
            _ => None,
        }
    }
}

impl std::str::FromStr for Format {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

// ============================================================================
// Global Configuration
// ============================================================================

/// Global debug flag - checked by macros.
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Global log level.
static LOG_LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Global configuration (lazy initialized).
static CONFIG: Lazy<LogConfig> = Lazy::new(LogConfig::from_env);

/// Logging configuration.
#[derive(Debug)]
pub struct LogConfig {
    /// Whether debug mode is enabled
    pub debug: bool,
    /// Minimum log level
    pub level: Level,
    /// Output format
    pub format: Format,
    /// Whether colors are enabled
    pub color: bool,
    /// Whether to include timestamps
    pub timestamps: bool,
    /// Whether to include module path
    pub module_path: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            debug: false,
            level: Level::Info,
            format: Format::Json,
            color: false, // JSON output doesn't use colors
            timestamps: true,
            module_path: true,
        }
    }
}

impl LogConfig {
    /// Create config from environment variables.
    ///
    /// This is a pure computation — it reads the current `ARMATURE_*`
    /// environment variables and returns a [`LogConfig`], but it does not
    /// mutate any global state. Syncing the runtime atomics that the
    /// `log`/`trace`/`debug`/`info`/`warn`/`error` macros actually read is
    /// the job of the crate's one-time `ensure_init` routine, invoked
    /// automatically from every public entry point (see [`init`]).
    pub fn from_env() -> Self {
        let debug = env::var("ARMATURE_DEBUG")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        let level = env::var("ARMATURE_LOG_LEVEL")
            .ok()
            .and_then(|s| Level::parse(&s))
            .unwrap_or(if debug { Level::Debug } else { Level::Info });

        let format = env::var("ARMATURE_LOG_FORMAT")
            .ok()
            .and_then(|s| Format::parse(&s))
            .unwrap_or(Format::Json);

        let color = env::var("ARMATURE_LOG_COLOR")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(atty::is(atty::Stream::Stderr));

        let timestamps = env::var("ARMATURE_LOG_TIMESTAMPS")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(true);

        let module_path = env::var("ARMATURE_LOG_MODULE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(true);

        Self {
            debug,
            level,
            format,
            color,
            timestamps,
            module_path,
        }
    }
}

/// Real TTY detection (for color detection fallback), used when
/// `ARMATURE_LOG_COLOR` is not explicitly set.
mod atty {
    use std::io::IsTerminal;

    /// The stream color output would be written to.
    ///
    /// Only `Stderr` is used today (every call site in this crate logs to
    /// stderr), but the type stays stream-specific rather than collapsing
    /// to a bare `bool`/no-op so a real per-stream check is actually
    /// possible to add here if a stdout-writing path is introduced later.
    pub enum Stream {
        Stderr,
    }

    impl Stream {
        fn is_terminal(&self) -> bool {
            match self {
                Stream::Stderr => std::io::stderr().is_terminal(),
            }
        }
    }

    /// Whether ANSI colors should be used when writing to `stream`.
    ///
    /// Performs a real terminal check via [`std::io::IsTerminal`] on the
    /// requested stream, honoring which stream was actually passed in.
    /// `NO_COLOR` (<https://no-color.org/>) and `TERM=dumb` are applied as
    /// overrides *on top of* that check: they can only turn color off, and
    /// can never force color on for a stream that isn't really a terminal.
    pub fn is(stream: Stream) -> bool {
        should_color(stream.is_terminal())
    }

    /// Pure override-composition logic, split out from the actual terminal
    /// probe so the `NO_COLOR`/`TERM` behavior can be unit-tested
    /// deterministically without depending on the test process's real fd
    /// state (which varies across CI/interactive runs).
    pub(crate) fn should_color(is_terminal: bool) -> bool {
        if !is_terminal {
            return false;
        }
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        if std::env::var_os("TERM").as_deref() == Some(std::ffi::OsStr::new("dumb")) {
            return false;
        }
        true
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Process-wide guard for [`ensure_init`].
static INIT: Once = Once::new();

/// Perform the one-time global initialization of the logging system: force
/// the [`CONFIG`] lazy (which reads the `ARMATURE_*` environment variables)
/// and sync *every* runtime atomic — level, debug, format, color,
/// timestamps, and module path — from it in one atomic-consistent step.
///
/// This is invoked automatically from every public entry point that reads
/// or writes logging state: [`init`], [`config`], [`is_level_enabled`],
/// [`is_debug_enabled`], [`log`], [`current_level`], [`current_format`],
/// and the `set_*` runtime setters. That means the very first touch of
/// this crate — whether it's the first use of a `trace!`/`debug!`/`info!`/
/// `warn!`/`error!` macro, or a direct call to `config()`/`init()`, or an
/// eager `set_level()`/`configure()...apply()` call at startup — is enough
/// to make `ARMATURE_*` env vars take effect, with no explicit `init()`
/// call required.
///
/// It's guarded by a [`Once`], so repeated calls are cheap no-ops after the
/// first. Because every setter also calls this *before* applying its own
/// explicit store, an explicit override always wins regardless of whether
/// it happens before or after the env-derived seed: if the setter is the
/// first thing to run, this seeds the atomics from env and the setter's
/// own store immediately overwrites that one field; if this has already
/// run (e.g. via an earlier macro use), the setter's store simply applies
/// on top of the already-seeded atomics.
fn ensure_init() {
    INIT.call_once(|| {
        let config = Lazy::force(&CONFIG);
        DEBUG_ENABLED.store(config.debug, Ordering::SeqCst);
        LOG_LEVEL.store(config.level as u8, Ordering::SeqCst);
        LOG_FORMAT.store(config.format as u8, Ordering::SeqCst);
        LOG_COLOR.store(config.color, Ordering::SeqCst);
        LOG_TIMESTAMPS.store(config.timestamps, Ordering::SeqCst);
        LOG_MODULE_PATH.store(config.module_path, Ordering::SeqCst);
    });
}

/// Initialize the logging system.
///
/// This runs automatically the first time a log macro (or any other public
/// entry point, such as [`config`]) is used, so `ARMATURE_*` environment
/// variables take effect without calling this explicitly. Calling it
/// explicitly is still useful for eager initialization — e.g. to force env
/// parsing to happen at a known point during startup, before any logging
/// occurs. It is idempotent: only the first call (from *any* entry point)
/// has an effect.
pub fn init() {
    ensure_init();
}

/// Check if debug logging is enabled.
#[inline]
pub fn is_debug_enabled() -> bool {
    ensure_init();
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// Check if a log level is enabled.
#[inline]
pub fn is_level_enabled(level: Level) -> bool {
    ensure_init();
    level as u8 >= LOG_LEVEL.load(Ordering::Relaxed)
}

/// Get current log level.
pub fn current_level() -> Level {
    ensure_init();
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => Level::Trace,
        1 => Level::Debug,
        2 => Level::Info,
        3 => Level::Warn,
        4 => Level::Error,
        _ => Level::Off,
    }
}

/// Set log level at runtime.
pub fn set_level(level: Level) {
    ensure_init();
    LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

/// Enable or disable debug mode at runtime.
pub fn set_debug(enabled: bool) {
    ensure_init();
    DEBUG_ENABLED.store(enabled, Ordering::SeqCst);
    if enabled && current_level() > Level::Debug {
        set_level(Level::Debug);
    }
}

/// Get the global configuration.
///
/// This performs the same full atomic initialization as [`init`] (via the
/// crate's internal `ensure_init` routine) — level, debug, format, color,
/// timestamps, and module path are all synced from `ARMATURE_*` env vars on
/// first call, so calling `config()` alone (without ever calling `init()`)
/// still leaves the runtime in a fully-initialized state rather than a
/// partial one.
pub fn config() -> &'static LogConfig {
    ensure_init();
    &CONFIG
}

// ============================================================================
// Runtime Configuration
// ============================================================================

use std::sync::atomic::AtomicU8 as AtomicFormat;

/// Global format setting (can be changed at runtime).
static LOG_FORMAT: AtomicFormat = AtomicFormat::new(Format::Json as u8);

/// Global color setting.
static LOG_COLOR: AtomicBool = AtomicBool::new(false);

/// Global timestamps setting.
static LOG_TIMESTAMPS: AtomicBool = AtomicBool::new(true);

/// Global module path setting.
static LOG_MODULE_PATH: AtomicBool = AtomicBool::new(true);

/// Get the current log format.
pub fn current_format() -> Format {
    ensure_init();
    match LOG_FORMAT.load(Ordering::Relaxed) {
        0 => Format::Pretty,
        1 => Format::Compact,
        _ => Format::Json,
    }
}

/// Set log format at runtime.
///
/// # Example
///
/// ```rust
/// use armature_log::{set_format, Format};
///
/// // Switch to pretty format for development
/// set_format(Format::Pretty);
///
/// // Switch back to JSON for production
/// set_format(Format::Json);
/// ```
pub fn set_format(format: Format) {
    ensure_init();
    LOG_FORMAT.store(format as u8, Ordering::SeqCst);
    // Also update color based on format
    if format == Format::Pretty {
        LOG_COLOR.store(atty::is(atty::Stream::Stderr), Ordering::SeqCst);
    } else if format == Format::Json {
        LOG_COLOR.store(false, Ordering::SeqCst);
    }
}

/// Set whether colors are enabled.
pub fn set_color(enabled: bool) {
    ensure_init();
    LOG_COLOR.store(enabled, Ordering::SeqCst);
}

/// Set whether timestamps are included.
pub fn set_timestamps(enabled: bool) {
    ensure_init();
    LOG_TIMESTAMPS.store(enabled, Ordering::SeqCst);
}

/// Set whether module path is included.
pub fn set_module_path(enabled: bool) {
    ensure_init();
    LOG_MODULE_PATH.store(enabled, Ordering::SeqCst);
}

/// Configuration builder for fluent API.
///
/// # Example
///
/// ```rust
/// use armature_log::{configure, Format, Level};
///
/// configure()
///     .format(Format::Pretty)
///     .level(Level::Debug)
///     .color(true)
///     .timestamps(true)
///     .apply();
/// ```
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    format: Option<Format>,
    level: Option<Level>,
    color: Option<bool>,
    timestamps: Option<bool>,
    module_path: Option<bool>,
    debug: Option<bool>,
}

impl ConfigBuilder {
    /// Create a new configuration builder.
    pub fn new() -> Self {
        Self {
            format: None,
            level: None,
            color: None,
            timestamps: None,
            module_path: None,
            debug: None,
        }
    }

    /// Set the output format.
    pub fn format(mut self, format: Format) -> Self {
        self.format = Some(format);
        self
    }

    /// Set the log level.
    pub fn level(mut self, level: Level) -> Self {
        self.level = Some(level);
        self
    }

    /// Enable or disable colors.
    pub fn color(mut self, enabled: bool) -> Self {
        self.color = Some(enabled);
        self
    }

    /// Enable or disable timestamps.
    pub fn timestamps(mut self, enabled: bool) -> Self {
        self.timestamps = Some(enabled);
        self
    }

    /// Enable or disable module path in output.
    pub fn module_path(mut self, enabled: bool) -> Self {
        self.module_path = Some(enabled);
        self
    }

    /// Enable or disable debug mode.
    pub fn debug(mut self, enabled: bool) -> Self {
        self.debug = Some(enabled);
        self
    }

    /// Apply the configuration.
    pub fn apply(self) {
        if let Some(format) = self.format {
            set_format(format);
        }
        if let Some(level) = self.level {
            set_level(level);
        }
        if let Some(color) = self.color {
            set_color(color);
        }
        if let Some(timestamps) = self.timestamps {
            set_timestamps(timestamps);
        }
        if let Some(module_path) = self.module_path {
            set_module_path(module_path);
        }
        if let Some(debug) = self.debug {
            set_debug(debug);
        }
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a configuration builder.
///
/// # Example
///
/// ```rust
/// use armature_log::{configure, Format, Level};
///
/// // Development config
/// configure()
///     .format(Format::Pretty)
///     .level(Level::Debug)
///     .color(true)
///     .apply();
/// ```
pub fn configure() -> ConfigBuilder {
    ConfigBuilder::new()
}

/// Apply development preset: Pretty format, Debug level, colors enabled.
///
/// # Example
///
/// ```rust
/// armature_log::preset_development();
/// ```
pub fn preset_development() {
    configure()
        .format(Format::Pretty)
        .level(Level::Debug)
        .color(true)
        .timestamps(true)
        .module_path(true)
        .debug(true)
        .apply();
}

/// Apply production preset: JSON format, Info level, no colors.
///
/// # Example
///
/// ```rust
/// armature_log::preset_production();
/// ```
pub fn preset_production() {
    configure()
        .format(Format::Json)
        .level(Level::Info)
        .color(false)
        .timestamps(true)
        .module_path(true)
        .debug(false)
        .apply();
}

/// Apply quiet preset: JSON format, Warn level only.
pub fn preset_quiet() {
    configure()
        .format(Format::Json)
        .level(Level::Warn)
        .color(false)
        .apply();
}

// ============================================================================
// Log Output
// ============================================================================

/// Log a message with the given level.
#[doc(hidden)]
pub fn log(level: Level, target: &str, message: &str) {
    ensure_init();
    if !is_level_enabled(level) {
        return;
    }

    // Use runtime-configurable format instead of static config
    let format = current_format();
    let color = LOG_COLOR.load(Ordering::Relaxed);
    let timestamps = LOG_TIMESTAMPS.load(Ordering::Relaxed);
    let module_path = LOG_MODULE_PATH.load(Ordering::Relaxed);

    match format {
        Format::Pretty => {
            log_pretty_runtime(level, target, message, color, timestamps, module_path)
        }
        Format::Compact => log_compact_runtime(level, target, message, timestamps, module_path),
        Format::Json => log_json(level, target, message),
    }
}

// Runtime-configurable versions

fn log_pretty_runtime(
    level: Level,
    target: &str,
    message: &str,
    color: bool,
    timestamps: bool,
    module_path: bool,
) {
    let mut stderr = std::io::stderr().lock();
    write_pretty(
        &mut stderr,
        level,
        target,
        message,
        color,
        timestamps,
        module_path,
    );
}

/// Render a single Pretty-format log line into `w`.
///
/// Split out from [`log_pretty_runtime`] (which always writes to process
/// stderr) so tests can capture the exact emitted bytes via an in-memory
/// writer without any new public API.
fn write_pretty<W: Write>(
    w: &mut W,
    level: Level,
    target: &str,
    message: &str,
    color: bool,
    timestamps: bool,
    module_path: bool,
) {
    // Timestamp
    if timestamps {
        let now = chrono::Local::now();
        let _ = write!(w, "{} ", now.format("%Y-%m-%d %H:%M:%S%.3f"));
    }

    // Level
    #[cfg(feature = "color")]
    if color {
        let _ = write!(w, "{:5} ", level.colored());
    } else {
        let _ = write!(w, "{:5} ", level.as_str());
    }

    #[cfg(not(feature = "color"))]
    {
        let _ = color; // suppress warning
        let _ = write!(w, "{:5} ", level.as_str());
    }

    // Target
    if module_path && !target.is_empty() {
        // The brackets belong to the format, not to the absence of color:
        // dropping them when colors are on made the colored output disagree
        // with the documented `[my_app]` shape - and colors are on by default
        // in `preset_development()`, so the documented shape was the one
        // almost nobody saw.
        #[cfg(feature = "color")]
        if color {
            use colored::Colorize;
            let _ = write!(w, "{} ", format!("[{}]", target).dimmed());
        } else {
            let _ = write!(w, "[{}] ", target);
        }

        #[cfg(not(feature = "color"))]
        let _ = write!(w, "[{}] ", target);
    }

    // Message
    let _ = writeln!(w, "{}", message);
}

fn log_compact_runtime(
    level: Level,
    target: &str,
    message: &str,
    timestamps: bool,
    module_path: bool,
) {
    let mut stderr = std::io::stderr().lock();
    write_compact(&mut stderr, level, target, message, timestamps, module_path);
}

/// Render a single Compact-format log line into `w`.
///
/// Split out from [`log_compact_runtime`] for the same testability reason
/// as [`write_pretty`].
fn write_compact<W: Write>(
    w: &mut W,
    level: Level,
    target: &str,
    message: &str,
    timestamps: bool,
    module_path: bool,
) {
    if timestamps {
        let now = chrono::Local::now();
        let _ = write!(w, "{} ", now.format("%H:%M:%S"));
    }

    let _ = write!(w, "{} ", level.as_str().chars().next().unwrap_or('?'));

    if module_path && !target.is_empty() {
        let _ = write!(w, "{}: ", target);
    }

    let _ = writeln!(w, "{}", message);
}

fn log_json(level: Level, target: &str, message: &str) {
    if let Some(json) = render_json(level, target, message) {
        eprintln!("{}", json);
    }
}

/// Render a single JSON-format log line (no trailing newline).
///
/// Split out from [`log_json`] (which always writes to process stderr via
/// `eprintln!`) so tests can assert on the emitted shape/fields directly.
/// Returns `None` only if serialization somehow fails (practically
/// unreachable for this all-string payload) — matching `log_json`'s prior
/// "emit nothing on serialization error" behavior.
#[cfg(feature = "json")]
fn render_json(level: Level, target: &str, message: &str) -> Option<String> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct LogEntry<'a> {
        timestamp: String,
        level: &'a str,
        target: &'a str,
        message: &'a str,
    }

    let entry = LogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.as_str(),
        target,
        message,
    };

    serde_json::to_string(&entry).ok()
}

#[cfg(not(feature = "json"))]
fn render_json(level: Level, target: &str, message: &str) -> Option<String> {
    // Fallback without serde - manually escape JSON strings
    let timestamp = chrono::Utc::now().to_rfc3339();
    Some(format!(
        r#"{{"timestamp":"{}","level":"{}","target":"{}","message":"{}"}}"#,
        timestamp,
        level.as_str(),
        escape_json(target),
        escape_json(message)
    ))
}

#[cfg(not(feature = "json"))]
fn escape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

// ============================================================================
// Macros
// ============================================================================

/// Log a trace message.
///
/// Only enabled when `ARMATURE_DEBUG=1` or `ARMATURE_LOG_LEVEL=trace`.
#[macro_export]
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Trace) {
            $crate::log($crate::Level::Trace, $target, &format!($($arg)+));
        }
    };
    ($($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Trace) {
            $crate::log($crate::Level::Trace, module_path!(), &format!($($arg)+));
        }
    };
}

/// Log a debug message.
///
/// Only enabled when `ARMATURE_DEBUG=1` or `ARMATURE_LOG_LEVEL=debug`.
///
/// # Example
///
/// ```rust
/// use armature_log::debug;
///
/// debug!("Processing request");
/// let username = "alice";
/// debug!("User {} logged in", username);
/// let path = "/api/users";
/// debug!(target: "armature::router", "Matching route: {}", path);
/// ```
#[macro_export]
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::is_debug_enabled() || $crate::is_level_enabled($crate::Level::Debug) {
            $crate::log($crate::Level::Debug, $target, &format!($($arg)+));
        }
    };
    ($($arg:tt)+) => {
        if $crate::is_debug_enabled() || $crate::is_level_enabled($crate::Level::Debug) {
            $crate::log($crate::Level::Debug, module_path!(), &format!($($arg)+));
        }
    };
}

/// Log an info message.
#[macro_export]
macro_rules! info {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Info) {
            $crate::log($crate::Level::Info, $target, &format!($($arg)+));
        }
    };
    ($($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Info) {
            $crate::log($crate::Level::Info, module_path!(), &format!($($arg)+));
        }
    };
}

/// Log a warning message.
#[macro_export]
macro_rules! warn {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Warn) {
            $crate::log($crate::Level::Warn, $target, &format!($($arg)+));
        }
    };
    ($($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Warn) {
            $crate::log($crate::Level::Warn, module_path!(), &format!($($arg)+));
        }
    };
}

/// Log an error message.
#[macro_export]
macro_rules! error {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Error) {
            $crate::log($crate::Level::Error, $target, &format!($($arg)+));
        }
    };
    ($($arg:tt)+) => {
        if $crate::is_level_enabled($crate::Level::Error) {
            $crate::log($crate::Level::Error, module_path!(), &format!($($arg)+));
        }
    };
}

// ============================================================================
// Tracing Integration
// ============================================================================

#[cfg(feature = "tracing")]
pub mod tracing_compat {
    //! Tracing compatibility layer.
    //!
    //! When the `tracing` feature is enabled, this module provides
    //! a subscriber that respects `ARMATURE_DEBUG`.

    use super::*;

    /// Create a tracing subscriber that respects Armature config.
    pub fn subscriber() -> impl tracing::Subscriber {
        use tracing_subscriber::prelude::*;
        use tracing_subscriber::{EnvFilter, fmt};

        let config = config();
        let level = match config.level {
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
            Level::Off => "off",
        };

        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_ansi(config.color))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::IsTerminal;
    use std::sync::Mutex;

    /// `std::env` and the crate's runtime atomics are process-global, and
    /// `ensure_init()` reads env exactly once per process (guarded by a
    /// `Once`). Serialize every test that touches either so they don't
    /// race against each other under `cargo test`'s default parallelism —
    /// otherwise one test's transient env mutation could get baked into
    /// the one-time global init by a concurrently-running test.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_level_ordering() {
        assert!(Level::Trace < Level::Debug);
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
        assert!(Level::Error < Level::Off);
    }

    #[test]
    fn test_level_parse() {
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
        assert_eq!(Level::parse("DEBUG"), Some(Level::Debug));
        assert_eq!(Level::parse("warn"), Some(Level::Warn));
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert_eq!(Level::parse("invalid"), None);
    }

    #[test]
    fn test_format_parse() {
        assert_eq!(Format::parse("pretty"), Some(Format::Pretty));
        assert_eq!(Format::parse("compact"), Some(Format::Compact));
        assert_eq!(Format::parse("json"), Some(Format::Json));
        assert_eq!(Format::parse("invalid"), None);
    }

    #[test]
    fn test_set_level() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original = current_level();

        set_level(Level::Error);
        assert_eq!(current_level(), Level::Error);

        set_level(Level::Debug);
        assert_eq!(current_level(), Level::Debug);

        set_level(original);
    }

    #[test]
    fn test_debug_flag() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original = is_debug_enabled();

        set_debug(true);
        assert!(is_debug_enabled());

        set_debug(false);
        assert!(!is_debug_enabled());

        set_debug(original);
    }

    #[test]
    fn test_macros_compile() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Just verify macros compile correctly
        trace!("trace message");
        debug!("debug message");
        info!("info message");
        warn!("warn message");
        error!("error message");

        trace!(target: "test", "with target");
        debug!(target: "test", "with target");
        info!(target: "test", "with target");
        warn!(target: "test", "with target");
        error!(target: "test", "with target");

        let x = 42;
        debug!("formatted: {}", x);
    }

    // ------------------------------------------------------------------
    // `LogConfig::from_env()` — pure parsing, no global side effects
    // ------------------------------------------------------------------

    #[test]
    fn from_env_parses_all_fields_from_env_vars() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: guarded by ENV_LOCK against other tests in this module
        // that also mutate process env vars.
        unsafe {
            env::set_var("ARMATURE_DEBUG", "1");
            env::set_var("ARMATURE_LOG_LEVEL", "trace");
            env::set_var("ARMATURE_LOG_FORMAT", "pretty");
            env::set_var("ARMATURE_LOG_COLOR", "1");
            env::set_var("ARMATURE_LOG_TIMESTAMPS", "0");
            env::set_var("ARMATURE_LOG_MODULE", "0");
        }

        let config = LogConfig::from_env();

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("ARMATURE_DEBUG");
            env::remove_var("ARMATURE_LOG_LEVEL");
            env::remove_var("ARMATURE_LOG_FORMAT");
            env::remove_var("ARMATURE_LOG_COLOR");
            env::remove_var("ARMATURE_LOG_TIMESTAMPS");
            env::remove_var("ARMATURE_LOG_MODULE");
        }

        assert!(config.debug);
        assert_eq!(config.level, Level::Trace);
        assert_eq!(config.format, Format::Pretty);
        assert!(config.color);
        assert!(!config.timestamps);
        assert!(!config.module_path);
    }

    #[test]
    fn from_env_defaults_level_to_debug_when_armature_debug_set_without_explicit_level() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::set_var("ARMATURE_DEBUG", "true");
            env::remove_var("ARMATURE_LOG_LEVEL");
        }

        let config = LogConfig::from_env();

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("ARMATURE_DEBUG");
        }

        assert!(config.debug);
        assert_eq!(config.level, Level::Debug);
    }

    #[test]
    fn from_env_does_not_mutate_runtime_atomics() {
        // Regression for the "config() partially initializes" finding at
        // its root: from_env() must be a pure computation. Previously it
        // had a side effect of storing straight into the DEBUG_ENABLED /
        // LOG_LEVEL atomics itself, which is what made `config()`'s atomic
        // sync inconsistent depending on whether `init()` had also run
        // (init() additionally synced format/color/timestamps/module_path,
        // from_env() only synced level/debug). All atomic syncing now
        // happens solely in `ensure_init()`.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        set_level(Level::Warn); // explicit baseline via the public setter
        let before = current_level();
        assert_eq!(before, Level::Warn);

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::set_var("ARMATURE_LOG_LEVEL", "trace");
        }
        let _ = LogConfig::from_env(); // must NOT touch the LOG_LEVEL atomic
        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("ARMATURE_LOG_LEVEL");
        }

        assert_eq!(
            current_level(),
            before,
            "from_env() must not mutate global atomics as a side effect"
        );

        set_level(before); // restore
    }

    // ------------------------------------------------------------------
    // `atty::should_color` — NO_COLOR/TERM override composition
    // ------------------------------------------------------------------

    #[test]
    fn should_color_false_when_not_a_terminal_regardless_of_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("NO_COLOR");
            env::set_var("TERM", "xterm-256color");
        }

        assert!(!atty::should_color(false));

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("TERM");
        }
    }

    #[test]
    fn should_color_true_when_terminal_and_no_overrides() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("NO_COLOR");
            env::set_var("TERM", "xterm-256color");
        }

        assert!(atty::should_color(true));

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("TERM");
        }
    }

    #[test]
    fn should_color_no_color_env_disables_even_on_a_real_terminal() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::set_var("NO_COLOR", "1");
            env::set_var("TERM", "xterm-256color");
        }

        assert!(!atty::should_color(true));

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("NO_COLOR");
            env::remove_var("TERM");
        }
    }

    #[test]
    fn should_color_term_dumb_disables_even_on_a_real_terminal() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("NO_COLOR");
            env::set_var("TERM", "dumb");
        }

        assert!(!atty::should_color(true));

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("TERM");
        }
    }

    #[test]
    fn atty_is_honors_the_real_terminal_state_of_the_requested_stream() {
        // Regression: previously `atty::is` ignored its `Stream` argument
        // entirely and returned `NO_COLOR unset && TERM set` regardless of
        // whether the stream was actually a terminal — meaning color could
        // be auto-enabled for a piped/redirected stderr. It must now agree
        // with a real `IsTerminal` check whenever no override is active.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("NO_COLOR");
            env::set_var("TERM", "xterm-256color");
        }

        let real_tty = std::io::stderr().is_terminal();
        let result = atty::is(atty::Stream::Stderr);

        // SAFETY: guarded by ENV_LOCK
        unsafe {
            env::remove_var("TERM");
        }

        assert_eq!(
            result, real_tty,
            "atty::is must reflect the stream's real terminal status, not \
             a NO_COLOR/TERM heuristic"
        );
    }

    // ------------------------------------------------------------------
    // Output-format renderers — assert emitted shape/fields
    // ------------------------------------------------------------------

    #[test]
    fn render_json_includes_all_expected_fields() {
        let json = render_json(Level::Info, "my_target", "hello world")
            .expect("render_json should succeed for plain string payloads");

        assert!(
            json.contains("\"timestamp\":\""),
            "missing timestamp field: {json}"
        );
        assert!(
            json.contains("\"level\":\"INFO\""),
            "missing/wrong level field: {json}"
        );
        assert!(
            json.contains("\"target\":\"my_target\""),
            "missing/wrong target field: {json}"
        );
        assert!(
            json.contains("\"message\":\"hello world\""),
            "missing/wrong message field: {json}"
        );
        assert!(json.starts_with('{') && json.ends_with('}'));
    }

    #[test]
    fn render_json_escapes_special_characters_in_message() {
        let json = render_json(Level::Error, "t", "line1\nline2 \"quoted\"")
            .expect("render_json should succeed");

        // Must not contain a literal unescaped newline or quote breaking
        // the JSON structure.
        assert!(!json.contains("line1\nline2"));
        assert!(json.contains("\\n"));
        assert!(json.contains("\\\"quoted\\\""));
    }

    #[cfg(feature = "json")]
    #[test]
    fn render_json_is_valid_parseable_json() {
        let json =
            render_json(Level::Warn, "svc::mod", "boom").expect("render_json should succeed");

        let value: serde_json::Value =
            serde_json::from_str(&json).expect("render_json output must be valid JSON");

        assert_eq!(value["level"], "WARN");
        assert_eq!(value["target"], "svc::mod");
        assert_eq!(value["message"], "boom");
        assert!(value["timestamp"].is_string());
        assert_eq!(
            value.as_object().map(|o| o.len()),
            Some(4),
            "expected exactly timestamp/level/target/message, got: {value}"
        );
    }

    #[test]
    fn write_pretty_renders_expected_shape_without_timestamp() {
        let mut buf: Vec<u8> = Vec::new();
        write_pretty(
            &mut buf,
            Level::Warn,
            "my::target",
            "careful now",
            false,
            false,
            true,
        );
        let out = String::from_utf8(buf).unwrap();

        assert_eq!(out, "WARN  [my::target] careful now\n");
    }

    #[test]
    fn write_pretty_includes_timestamp_when_enabled() {
        let mut buf: Vec<u8> = Vec::new();
        write_pretty(&mut buf, Level::Info, "", "started", false, true, true);
        let out = String::from_utf8(buf).unwrap();

        // "YYYY-MM-DD HH:MM:SS.mmm " prefix before the level.
        assert!(
            out.contains("INFO  started"),
            "expected level+message, got: {out:?}"
        );
        let ts_prefix = out.split("INFO").next().unwrap();
        assert!(
            ts_prefix.len() >= 20,
            "expected a timestamp-shaped prefix, got: {ts_prefix:?}"
        );
    }

    #[test]
    fn write_pretty_omits_target_when_module_path_disabled() {
        let mut buf: Vec<u8> = Vec::new();
        write_pretty(
            &mut buf,
            Level::Error,
            "should::not::appear",
            "oops",
            false,
            false,
            false,
        );
        let out = String::from_utf8(buf).unwrap();

        assert_eq!(out, "ERROR oops\n");
        assert!(!out.contains("should::not::appear"));
    }

    #[test]
    fn write_compact_renders_expected_shape_without_timestamp() {
        let mut buf: Vec<u8> = Vec::new();
        write_compact(&mut buf, Level::Debug, "svc", "message here", false, true);
        let out = String::from_utf8(buf).unwrap();

        assert_eq!(out, "D svc: message here\n");
    }

    #[test]
    fn write_compact_omits_target_when_module_path_disabled() {
        let mut buf: Vec<u8> = Vec::new();
        write_compact(&mut buf, Level::Trace, "svc", "hi", false, false);
        let out = String::from_utf8(buf).unwrap();

        assert_eq!(out, "T hi\n");
    }

    #[test]
    fn write_compact_includes_timestamp_when_enabled() {
        let mut buf: Vec<u8> = Vec::new();
        write_compact(&mut buf, Level::Info, "", "go", true, true);
        let out = String::from_utf8(buf).unwrap();

        // "HH:MM:SS " prefix (8 chars + space) before the level letter.
        assert!(out.contains("I go"), "expected level+message, got: {out:?}");
        let ts_prefix = out.split('I').next().unwrap();
        assert_eq!(
            ts_prefix.len(),
            9,
            "expected an 'HH:MM:SS ' shaped prefix, got: {ts_prefix:?}"
        );
    }
}
