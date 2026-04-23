//! Enterprise SIEM integration for Armature
//!
//! This crate provides integration with enterprise Security Information and Event
//! Management (SIEM) systems including Splunk, Elasticsearch, IBM QRadar,
//! Microsoft Sentinel, Datadog, ArcSight, and more.
//!
//! # Features
//!
//! - **Multiple SIEM Providers** - Splunk, Elastic, QRadar, Sentinel, Datadog, ArcSight
//! - **Multiple Formats** - JSON, CEF, LEEF, Syslog RFC 5424, Elastic Common Schema
//! - **Multiple Transports** - HTTPS, TCP, UDP, TLS
//! - **Batching** - Automatic event batching for efficiency
//! - **Retry Logic** - Built-in retry with exponential backoff
//! - **Audit Integration** - Convert armature-audit events to SIEM events
//!
//! # Quick Start
//!
//! ## Splunk HEC
//!
//! ```no_run
//! use armature_siem::*;
//!
//! # async fn example() -> Result<(), SiemError> {
//! let config = SplunkConfig::hec("https://splunk.example.com:8088")
//!     .token("your-hec-token")
//!     .index("security")
//!     .build()?;
//!
//! let client = SiemClient::new(config)?;
//!
//! client.send(SiemEvent::new("user.login")
//!     .category(EventCategory::Authentication)
//!     .severity(SiemSeverity::Low)
//!     .outcome(EventOutcome::Success)
//!     .src_user("alice")
//!     .src_ip("192.168.1.100")
//!     .action("login")).await?;
//!
//! client.close().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Elasticsearch with ECS
//!
//! ```no_run
//! use armature_siem::*;
//!
//! # async fn example() -> Result<(), SiemError> {
//! let config = ElasticConfig::new("https://elastic.example.com:9200")
//!     .index("security-events")
//!     .basic_auth("elastic", "password")
//!     .build()?;
//!
//! let client = SiemClient::new(config)?;
//!
//! client.send(SiemEvent::new("file.access")
//!     .category(EventCategory::File)
//!     .severity(SiemSeverity::Medium)
//!     .file_path("/etc/passwd")
//!     .src_user("root")
//!     .action("read")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## IBM QRadar with LEEF
//!
//! ```no_run
//! use armature_siem::*;
//!
//! # async fn example() -> Result<(), SiemError> {
//! let config = QRadarConfig::leef("qradar.example.com:514")
//!     .cef_vendor("MyCompany")
//!     .cef_product("MyApp")
//!     .build()?;
//!
//! let client = SiemClient::new(config)?;
//!
//! client.send(SiemEvent::new("auth.failure")
//!     .category(EventCategory::Authentication)
//!     .severity(SiemSeverity::High)
//!     .outcome(EventOutcome::Failure)
//!     .src_ip("10.0.0.50")
//!     .action("login_attempt")
//!     .message("Brute force attempt detected")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Generic Syslog
//!
//! ```no_run
//! use armature_siem::*;
//!
//! # async fn example() -> Result<(), SiemError> {
//! let config = SyslogConfig::udp("syslog.example.com:514")
//!     .app_name("myapp")
//!     .syslog_facility(SyslogFacility::Auth)
//!     .build()?;
//!
//! let client = SiemClient::new(config)?;
//!
//! client.send(SiemEvent::new("session.created")
//!     .category(EventCategory::Session)
//!     .src_user("bob")
//!     .action("create")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Event Categories
//!
//! Events are categorized using standard security event categories:
//!
//! - `Authentication` - Login, logout, password changes
//! - `Authorization` - Permission checks, access control
//! - `File` - File access, modification, deletion
//! - `Network` - Network connections, traffic
//! - `Process` - Process creation, termination
//! - `Session` - Session management
//! - `Threat` - Threat detection, malware
//! - `Web` - HTTP requests, API calls
//!
//! # Severity Levels
//!
//! Severity levels map to standard SIEM severity scales:
//!
//! | Level | CEF (0-10) | Syslog (0-7) |
//! |-------|------------|--------------|
//! | Unknown | 0 | 6 (Info) |
//! | Low | 3 | 5 (Notice) |
//! | Medium | 5 | 4 (Warning) |
//! | High | 8 | 3 (Error) |
//! | Critical | 10 | 2 (Critical) |
//!
//! # Feature Flags
//!
//! - `http` - Enable HTTP transport (required for Splunk HEC, Elastic, etc.)
//! - `audit` - Enable armature-audit integration
//! - `di` - Enable dependency injection support
//! - `full` - Enable all features

pub mod client;
pub mod config;
pub mod error;
pub mod event;
pub mod format;
pub mod provider;

pub use client::*;
pub use config::*;
pub use error::*;
pub use event::*;
pub use format::{EventFormatter, get_formatter};
pub use provider::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Ensure all public types are accessible
        let _ = SiemEvent::new("test");
        let _ = SiemSeverity::Low;
        let _ = EventCategory::Authentication;
        let _ = EventOutcome::Success;
        let _ = SiemProvider::Splunk;
        let _ = EventFormat::Json;
    }

    #[test]
    fn test_event_creation() {
        let event = SiemEvent::new("user.login")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::Low)
            .outcome(EventOutcome::Success)
            .src_ip("192.168.1.100")
            .src_user("alice")
            .action("login")
            .message("User logged in successfully");

        assert_eq!(event.event_type, "user.login");
        assert_eq!(event.category, EventCategory::Authentication);
        assert_eq!(event.severity, SiemSeverity::Low);
        assert_eq!(event.src_user, Some("alice".to_string()));
    }

    #[test]
    fn test_splunk_config_builder() {
        let config = SplunkConfig::hec("https://splunk.example.com:8088")
            .token("test-token")
            .index("main")
            .source_type("armature:security")
            .build()
            .unwrap();

        assert_eq!(config.provider, SiemProvider::Splunk);
        assert_eq!(config.format, EventFormat::Json);
        assert!(config.endpoint.contains("/services/collector"));
    }

    #[test]
    fn test_format_selection() {
        let formatter = get_formatter(EventFormat::Cef);
        assert_eq!(formatter.content_type(), "text/plain");

        let formatter = get_formatter(EventFormat::Json);
        assert_eq!(formatter.content_type(), "application/json");
    }
}
