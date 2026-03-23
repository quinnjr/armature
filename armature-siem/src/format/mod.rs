//! SIEM event format converters
//!
//! This module provides formatters for various SIEM event formats:
//!
//! - **CEF** - Common Event Format (ArcSight, Splunk, etc.)
//! - **LEEF** - Log Event Extended Format (IBM QRadar)
//! - **Syslog** - RFC 5424 Syslog format
//! - **JSON** - Generic JSON format
//! - **ECS** - Elastic Common Schema

mod cef;
mod ecs;
mod json;
mod leef;
mod syslog;

pub use cef::*;
pub use ecs::*;
pub use json::*;
pub use leef::*;
pub use syslog::*;

use crate::{EventFormat, SiemConfig, SiemEvent, SiemResult};

/// Trait for formatting SIEM events
pub trait EventFormatter: Send + Sync {
    /// Format an event to string
    fn format(&self, event: &SiemEvent, config: &SiemConfig) -> SiemResult<String>;

    /// Format multiple events (for batch sending)
    fn format_batch(&self, events: &[SiemEvent], config: &SiemConfig) -> SiemResult<String> {
        let formatted: SiemResult<Vec<String>> =
            events.iter().map(|e| self.format(e, config)).collect();
        Ok(formatted?.join("\n"))
    }

    /// Get the content type for HTTP transport
    fn content_type(&self) -> &'static str;
}

/// Get a formatter for the specified format
pub fn get_formatter(format: EventFormat) -> Box<dyn EventFormatter> {
    match format {
        EventFormat::Json => Box::new(JsonFormatter),
        EventFormat::Cef => Box::new(CefFormatter),
        EventFormat::Leef => Box::new(LeefFormatter),
        EventFormat::Syslog => Box::new(SyslogFormatter),
        EventFormat::Ecs => Box::new(EcsFormatter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventCategory, EventOutcome, SiemSeverity};

    fn sample_event() -> SiemEvent {
        SiemEvent::new("user.login")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::Low)
            .outcome(EventOutcome::Success)
            .src_ip("192.168.1.100")
            .src_user("alice")
            .action("login")
            .message("User logged in successfully")
    }

    fn sample_config() -> SiemConfig {
        SiemConfig {
            app_name: "test-app".to_string(),
            cef_vendor: "TestVendor".to_string(),
            cef_product: "TestProduct".to_string(),
            cef_version: "1.0".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_get_formatter() {
        let json_formatter = get_formatter(EventFormat::Json);
        assert_eq!(json_formatter.content_type(), "application/json");

        let cef_formatter = get_formatter(EventFormat::Cef);
        assert_eq!(cef_formatter.content_type(), "text/plain");
    }

    #[test]
    fn test_batch_formatting() {
        let events = vec![sample_event(), sample_event()];
        let config = sample_config();
        let formatter = get_formatter(EventFormat::Json);

        let result = formatter.format_batch(&events, &config);
        assert!(result.is_ok());
        let formatted = result.unwrap();
        assert!(formatted.contains('\n'));
    }
}
