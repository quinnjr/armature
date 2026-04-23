//! Syslog RFC 5424 formatter
//!
//! Formats events according to RFC 5424 (The Syslog Protocol)

use super::EventFormatter;
use crate::{SiemConfig, SiemEvent, SiemResult, SyslogFacility};

/// Syslog RFC 5424 formatter
pub struct SyslogFormatter;

impl SyslogFormatter {
    /// Calculate the PRI value (facility * 8 + severity)
    fn calculate_pri(facility: SyslogFacility, severity: u8) -> u8 {
        (facility as u8) * 8 + severity
    }

    /// Get hostname
    fn get_hostname() -> String {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "-".to_string())
    }

    /// Escape structured data parameter value
    fn escape_sd_value(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(']', "\\]")
    }

    /// Build structured data section
    fn build_structured_data(event: &SiemEvent, _config: &SiemConfig) -> String {
        let mut sd_parts = Vec::new();

        // armature@32473 - using a private enterprise number style ID
        let mut armature_sd = vec![
            format!("eventId=\"{}\"", Self::escape_sd_value(&event.id)),
            format!("eventType=\"{}\"", Self::escape_sd_value(&event.event_type)),
            format!("category=\"{}\"", event.category),
            format!(
                "outcome=\"{}\"",
                match event.outcome {
                    crate::EventOutcome::Success => "success",
                    crate::EventOutcome::Failure => "failure",
                    crate::EventOutcome::Unknown => "unknown",
                }
            ),
        ];

        if let Some(ref ip) = event.src_ip {
            armature_sd.push(format!("srcIp=\"{}\"", Self::escape_sd_value(ip)));
        }
        if let Some(ref user) = event.src_user {
            armature_sd.push(format!("srcUser=\"{}\"", Self::escape_sd_value(user)));
        }
        if let Some(ref ip) = event.dst_ip {
            armature_sd.push(format!("dstIp=\"{}\"", Self::escape_sd_value(ip)));
        }
        if let Some(ref method) = event.http_method {
            armature_sd.push(format!("httpMethod=\"{}\"", Self::escape_sd_value(method)));
        }
        if let Some(ref url) = event.url {
            armature_sd.push(format!("url=\"{}\"", Self::escape_sd_value(url)));
        }
        if let Some(status) = event.http_status {
            armature_sd.push(format!("httpStatus=\"{}\"", status));
        }
        if let Some(ref app) = event.application {
            armature_sd.push(format!("application=\"{}\"", Self::escape_sd_value(app)));
        }
        if let Some(dur) = event.duration_ms {
            armature_sd.push(format!("durationMs=\"{}\"", dur));
        }

        sd_parts.push(format!("[armature@32473 {}]", armature_sd.join(" ")));

        // Add custom metadata as additional structured data
        if !event.metadata.is_empty() {
            let meta_params: Vec<String> = event
                .metadata
                .iter()
                .map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    };
                    format!("{}=\"{}\"", k, Self::escape_sd_value(&val_str))
                })
                .collect();

            if !meta_params.is_empty() {
                sd_parts.push(format!("[meta@32473 {}]", meta_params.join(" ")));
            }
        }

        if sd_parts.is_empty() {
            "-".to_string()
        } else {
            sd_parts.join("")
        }
    }
}

impl EventFormatter for SyslogFormatter {
    fn format(&self, event: &SiemEvent, config: &SiemConfig) -> SiemResult<String> {
        // RFC 5424 format:
        // <PRI>VERSION TIMESTAMP HOSTNAME APP-NAME PROCID MSGID STRUCTURED-DATA MSG

        let pri = Self::calculate_pri(config.syslog_facility, event.severity.as_syslog_severity());
        let version = 1;
        let timestamp = event.timestamp.to_rfc3339();
        let hostname = Self::get_hostname();
        let app_name = &config.app_name;
        let proc_id = std::process::id();
        let msg_id = &event.event_type;
        let structured_data = Self::build_structured_data(event, config);

        // Build message
        let msg = event
            .message
            .as_ref()
            .map(|m| format!(" {}", m))
            .unwrap_or_default();

        let syslog = format!(
            "<{}>{} {} {} {} {} {} {}{}",
            pri, version, timestamp, hostname, app_name, proc_id, msg_id, structured_data, msg
        );

        Ok(syslog)
    }

    fn content_type(&self) -> &'static str {
        "text/plain"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventCategory, EventOutcome, SiemSeverity};

    fn sample_event() -> SiemEvent {
        SiemEvent::new("auth.failure")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::High)
            .outcome(EventOutcome::Failure)
            .src_ip("192.168.1.100")
            .src_user("attacker")
            .action("login")
            .message("Authentication failed: invalid password")
    }

    fn sample_config() -> SiemConfig {
        SiemConfig {
            app_name: "myapp".to_string(),
            syslog_facility: SyslogFacility::Auth,
            ..Default::default()
        }
    }

    #[test]
    fn test_syslog_format() {
        let formatter = SyslogFormatter;
        let event = sample_event();
        let config = sample_config();

        let result = formatter.format(&event, &config).unwrap();

        // Auth facility (4) * 8 + severity 3 (error) = 35
        assert!(result.starts_with("<35>1 "));
        assert!(result.contains("myapp"));
        assert!(result.contains("auth.failure"));
        assert!(result.contains("[armature@32473"));
        assert!(result.contains("Authentication failed"));
    }

    #[test]
    fn test_pri_calculation() {
        // User facility (1) * 8 + info severity (6) = 14
        assert_eq!(SyslogFormatter::calculate_pri(SyslogFacility::User, 6), 14);
        // Local0 facility (16) * 8 + critical (2) = 130
        assert_eq!(
            SyslogFormatter::calculate_pri(SyslogFacility::Local0, 2),
            130
        );
    }

    #[test]
    fn test_sd_escape() {
        assert_eq!(
            SyslogFormatter::escape_sd_value("test\"quote"),
            "test\\\"quote"
        );
        assert_eq!(
            SyslogFormatter::escape_sd_value("test]bracket"),
            "test\\]bracket"
        );
    }
}
