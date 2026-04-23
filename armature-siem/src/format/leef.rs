//! Log Event Extended Format (LEEF) formatter
//!
//! LEEF is used by IBM QRadar and other IBM security products.
//!
//! Format: LEEF:Version|Vendor|Product|Version|EventID|<tab-delimited attributes>

use super::EventFormatter;
use crate::{SiemConfig, SiemEvent, SiemResult};

/// LEEF (Log Event Extended Format) formatter
///
/// Formats events according to the LEEF 2.0 specification:
/// `LEEF:2.0|Vendor|Product|Version|EventID|<attributes>`
pub struct LeefFormatter;

impl LeefFormatter {
    /// Escape special characters in LEEF header fields
    fn escape_header(s: &str) -> String {
        s.replace('|', "\\|").replace('\n', " ").replace('\r', " ")
    }

    /// Escape special characters in LEEF attribute values
    fn escape_value(s: &str) -> String {
        s.replace('\t', " ")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('=', "\\=")
    }

    /// Build the LEEF attributes string (tab-delimited key=value pairs)
    fn build_attributes(event: &SiemEvent) -> String {
        let mut attrs = Vec::new();

        // Time stamp
        attrs.push(format!("devTime={}", event.timestamp.to_rfc3339()));

        // Severity mapping for QRadar
        attrs.push(format!("sev={}", event.severity.as_cef_severity()));

        // Category
        attrs.push(format!("cat={}", event.category));

        // Source fields
        if let Some(ref ip) = event.src_ip {
            attrs.push(format!("src={}", Self::escape_value(ip)));
        }
        if let Some(port) = event.src_port {
            attrs.push(format!("srcPort={}", port));
        }
        if let Some(ref host) = event.src_host {
            attrs.push(format!("srcHostName={}", Self::escape_value(host)));
        }
        if let Some(ref user) = event.src_user {
            attrs.push(format!("usrName={}", Self::escape_value(user)));
        }

        // Destination fields
        if let Some(ref ip) = event.dst_ip {
            attrs.push(format!("dst={}", Self::escape_value(ip)));
        }
        if let Some(port) = event.dst_port {
            attrs.push(format!("dstPort={}", port));
        }
        if let Some(ref host) = event.dst_host {
            attrs.push(format!("dstHostName={}", Self::escape_value(host)));
        }

        // HTTP fields
        if let Some(ref method) = event.http_method {
            attrs.push(format!("httpMethod={}", Self::escape_value(method)));
        }
        if let Some(ref url) = event.url {
            attrs.push(format!("url={}", Self::escape_value(url)));
        }
        if let Some(ref ua) = event.user_agent {
            attrs.push(format!("userAgent={}", Self::escape_value(ua)));
        }
        if let Some(status) = event.http_status {
            attrs.push(format!("httpStatusCode={}", status));
        }

        // Protocol
        if let Some(ref proto) = event.protocol {
            attrs.push(format!("proto={}", Self::escape_value(proto)));
        }

        // Bytes
        if let Some(bytes) = event.bytes {
            attrs.push(format!("bytesOut={}", bytes));
        }

        // Action and outcome
        attrs.push(format!("action={}", Self::escape_value(&event.action)));
        attrs.push(format!(
            "outcome={}",
            match event.outcome {
                crate::EventOutcome::Success => "success",
                crate::EventOutcome::Failure => "failure",
                crate::EventOutcome::Unknown => "unknown",
            }
        ));

        // Message
        if let Some(ref msg) = event.message {
            attrs.push(format!("msg={}", Self::escape_value(msg)));
        }

        // Resource fields
        if let Some(ref rt) = event.resource_type {
            attrs.push(format!("resourceType={}", Self::escape_value(rt)));
        }
        if let Some(ref rid) = event.resource_id {
            attrs.push(format!("resourceId={}", Self::escape_value(rid)));
        }

        // Application context
        if let Some(ref app) = event.application {
            attrs.push(format!("application={}", Self::escape_value(app)));
        }
        if let Some(ref svc) = event.service {
            attrs.push(format!("service={}", Self::escape_value(svc)));
        }

        // File information
        if let Some(ref path) = event.file_path {
            attrs.push(format!("fileName={}", Self::escape_value(path)));
        }
        if let Some(ref hash) = event.file_hash {
            attrs.push(format!("fileHash={}", Self::escape_value(hash)));
        }
        if let Some(size) = event.file_size {
            attrs.push(format!("fileSize={}", size));
        }

        // Error information
        if let Some(ref err) = event.error_message {
            attrs.push(format!("errorMsg={}", Self::escape_value(err)));
        }
        if let Some(ref code) = event.error_code {
            attrs.push(format!("errorCode={}", Self::escape_value(code)));
        }

        // Duration
        if let Some(dur) = event.duration_ms {
            attrs.push(format!("duration={}", dur));
        }

        // External ID
        attrs.push(format!("externalId={}", event.id));

        // Custom metadata
        for (key, value) in &event.metadata {
            let val_str = match value {
                serde_json::Value::String(s) => s.clone(),
                _ => value.to_string(),
            };
            attrs.push(format!(
                "{}={}",
                Self::escape_value(key),
                Self::escape_value(&val_str)
            ));
        }

        attrs.join("\t")
    }
}

impl EventFormatter for LeefFormatter {
    fn format(&self, event: &SiemEvent, config: &SiemConfig) -> SiemResult<String> {
        // LEEF:Version|Vendor|Product|Version|EventID|attributes
        let leef = format!(
            "LEEF:2.0|{}|{}|{}|{}|{}",
            Self::escape_header(&config.cef_vendor),
            Self::escape_header(&config.cef_product),
            Self::escape_header(&config.cef_version),
            Self::escape_header(&event.event_type),
            Self::build_attributes(event)
        );

        Ok(leef)
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
        SiemEvent::new("user.login")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::High)
            .outcome(EventOutcome::Failure)
            .src_ip("10.0.0.50")
            .src_user("bob")
            .action("login_attempt")
            .message("Failed login attempt")
    }

    fn sample_config() -> SiemConfig {
        SiemConfig {
            cef_vendor: "Armature".to_string(),
            cef_product: "Security".to_string(),
            cef_version: "2.0".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_leef_format() {
        let formatter = LeefFormatter;
        let event = sample_event();
        let config = sample_config();

        let result = formatter.format(&event, &config).unwrap();

        assert!(result.starts_with("LEEF:2.0|Armature|Security|2.0|user.login|"));
        assert!(result.contains("src=10.0.0.50"));
        assert!(result.contains("usrName=bob"));
        assert!(result.contains("outcome=failure"));
    }

    #[test]
    fn test_leef_escape() {
        assert_eq!(LeefFormatter::escape_header("test|pipe"), "test\\|pipe");
        assert_eq!(LeefFormatter::escape_value("key=value"), "key\\=value");
    }
}
