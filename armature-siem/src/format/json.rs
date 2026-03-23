//! JSON formatter for SIEM events
//!
//! Simple JSON serialization for events, compatible with most SIEM systems.

use super::EventFormatter;
use crate::{SiemConfig, SiemEvent, SiemResult};
use serde::Serialize;

/// JSON formatter
pub struct JsonFormatter;

/// Splunk HEC event wrapper
#[derive(Serialize)]
struct SplunkHecEvent<'a> {
    time: i64,
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sourcetype: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<&'a str>,
    event: &'a SiemEvent,
}

impl JsonFormatter {
    /// Get hostname
    fn get_hostname() -> String {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

impl EventFormatter for JsonFormatter {
    fn format(&self, event: &SiemEvent, config: &SiemConfig) -> SiemResult<String> {
        // For Splunk HEC, wrap in HEC format
        if config.provider == crate::SiemProvider::Splunk {
            let hec_event = SplunkHecEvent {
                time: event.timestamp.timestamp(),
                host: Self::get_hostname(),
                source: config.source.as_deref(),
                sourcetype: config.source_type.as_deref(),
                index: config.index.as_deref(),
                event,
            };
            Ok(serde_json::to_string(&hec_event)?)
        } else {
            // Generic JSON output
            Ok(serde_json::to_string(event)?)
        }
    }

    fn format_batch(&self, events: &[SiemEvent], config: &SiemConfig) -> SiemResult<String> {
        if config.provider == crate::SiemProvider::Splunk {
            // Splunk HEC accepts newline-delimited JSON
            let formatted: SiemResult<Vec<String>> =
                events.iter().map(|e| self.format(e, config)).collect();
            Ok(formatted?.join("\n"))
        } else {
            // Generic: JSON array
            Ok(serde_json::to_string(events)?)
        }
    }

    fn content_type(&self) -> &'static str {
        "application/json"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventCategory, EventOutcome, SiemProvider, SiemSeverity};

    fn sample_event() -> SiemEvent {
        SiemEvent::new("user.login")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::Low)
            .outcome(EventOutcome::Success)
            .src_ip("192.168.1.100")
            .src_user("alice")
            .action("login")
    }

    #[test]
    fn test_json_format_generic() {
        let formatter = JsonFormatter;
        let event = sample_event();
        let config = SiemConfig {
            provider: SiemProvider::Custom,
            ..Default::default()
        };

        let result = formatter.format(&event, &config).unwrap();

        assert!(result.contains("\"event_type\":\"user.login\""));
        assert!(result.contains("\"src_ip\":\"192.168.1.100\""));
    }

    #[test]
    fn test_json_format_splunk() {
        let formatter = JsonFormatter;
        let event = sample_event();
        let config = SiemConfig {
            provider: SiemProvider::Splunk,
            source: Some("armature".to_string()),
            source_type: Some("security".to_string()),
            index: Some("main".to_string()),
            ..Default::default()
        };

        let result = formatter.format(&event, &config).unwrap();

        assert!(result.contains("\"time\":"));
        assert!(result.contains("\"host\":"));
        assert!(result.contains("\"source\":\"armature\""));
        assert!(result.contains("\"sourcetype\":\"security\""));
        assert!(result.contains("\"event\":"));
    }

    #[test]
    fn test_batch_format() {
        let formatter = JsonFormatter;
        let events = vec![sample_event(), sample_event()];
        let config = SiemConfig {
            provider: SiemProvider::Splunk,
            ..Default::default()
        };

        let result = formatter.format_batch(&events, &config).unwrap();

        // Should be newline-delimited for Splunk
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2);
    }
}
