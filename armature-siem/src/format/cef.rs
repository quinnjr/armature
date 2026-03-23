//! Common Event Format (CEF) formatter
//!
//! CEF is used by ArcSight, Splunk, and other enterprise SIEMs.
//!
//! Format: CEF:Version|Device Vendor|Device Product|Device Version|Signature ID|Name|Severity|Extension

use super::EventFormatter;
use crate::{SiemConfig, SiemEvent, SiemResult};

/// CEF (Common Event Format) formatter
///
/// Formats events according to the CEF specification:
/// `CEF:0|Vendor|Product|Version|SignatureID|Name|Severity|Extension`
pub struct CefFormatter;

impl CefFormatter {
    /// Escape special characters in CEF header fields
    fn escape_header(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('|', "\\|")
            .replace('\n', " ")
            .replace('\r', " ")
    }

    /// Escape special characters in CEF extension values
    fn escape_extension(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('=', "\\=")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    }

    /// Build the CEF extension string
    fn build_extension(event: &SiemEvent) -> String {
        let mut ext = Vec::new();

        // Standard CEF extension fields
        ext.push(format!("rt={}", event.timestamp.timestamp_millis()));

        if let Some(ref ip) = event.src_ip {
            ext.push(format!("src={}", Self::escape_extension(ip)));
        }
        if let Some(port) = event.src_port {
            ext.push(format!("spt={}", port));
        }
        if let Some(ref host) = event.src_host {
            ext.push(format!("shost={}", Self::escape_extension(host)));
        }
        if let Some(ref user) = event.src_user {
            ext.push(format!("suser={}", Self::escape_extension(user)));
        }
        if let Some(ref process) = event.src_process {
            ext.push(format!("sproc={}", Self::escape_extension(process)));
        }

        if let Some(ref ip) = event.dst_ip {
            ext.push(format!("dst={}", Self::escape_extension(ip)));
        }
        if let Some(port) = event.dst_port {
            ext.push(format!("dpt={}", port));
        }
        if let Some(ref host) = event.dst_host {
            ext.push(format!("dhost={}", Self::escape_extension(host)));
        }
        if let Some(ref user) = event.dst_user {
            ext.push(format!("duser={}", Self::escape_extension(user)));
        }

        if let Some(ref method) = event.http_method {
            ext.push(format!("requestMethod={}", Self::escape_extension(method)));
        }
        if let Some(ref url) = event.url {
            ext.push(format!("request={}", Self::escape_extension(url)));
        }
        if let Some(ref ua) = event.user_agent {
            ext.push(format!(
                "requestClientApplication={}",
                Self::escape_extension(ua)
            ));
        }
        if let Some(status) = event.http_status {
            ext.push(format!("cn1={}", status));
            ext.push("cn1Label=httpStatusCode".to_string());
        }

        if let Some(bytes) = event.bytes {
            ext.push(format!("out={}", bytes));
        }

        if let Some(ref msg) = event.message {
            ext.push(format!("msg={}", Self::escape_extension(msg)));
        }

        if let Some(ref proto) = event.protocol {
            ext.push(format!("proto={}", Self::escape_extension(proto)));
        }

        if let Some(ref path) = event.file_path {
            ext.push(format!("filePath={}", Self::escape_extension(path)));
        }
        if let Some(ref hash) = event.file_hash {
            ext.push(format!("fileHash={}", Self::escape_extension(hash)));
        }
        if let Some(size) = event.file_size {
            ext.push(format!("fsize={}", size));
        }

        if let Some(ref app) = event.application {
            ext.push(format!("app={}", Self::escape_extension(app)));
        }

        if let Some(ref error) = event.error_message {
            ext.push(format!("reason={}", Self::escape_extension(error)));
        }

        ext.push(format!("externalId={}", event.id));
        ext.push(format!(
            "outcome={}",
            match event.outcome {
                crate::EventOutcome::Success => "Success",
                crate::EventOutcome::Failure => "Failure",
                crate::EventOutcome::Unknown => "Unknown",
            }
        ));

        ext.push(format!("cat={}", event.category));

        // Add custom metadata
        for (key, value) in &event.metadata {
            let val_str = match value {
                serde_json::Value::String(s) => s.clone(),
                _ => value.to_string(),
            };
            ext.push(format!(
                "cs1={} cs1Label={}",
                Self::escape_extension(&val_str),
                Self::escape_extension(key)
            ));
        }

        ext.join(" ")
    }
}

impl EventFormatter for CefFormatter {
    fn format(&self, event: &SiemEvent, config: &SiemConfig) -> SiemResult<String> {
        // CEF:Version|Device Vendor|Device Product|Device Version|Signature ID|Name|Severity|Extension
        let cef = format!(
            "CEF:0|{}|{}|{}|{}|{}|{}|{}",
            Self::escape_header(&config.cef_vendor),
            Self::escape_header(&config.cef_product),
            Self::escape_header(&config.cef_version),
            Self::escape_header(&event.event_type),
            Self::escape_header(&event.action),
            event.severity.as_cef_severity(),
            Self::build_extension(event)
        );

        Ok(cef)
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
            .severity(SiemSeverity::Low)
            .outcome(EventOutcome::Success)
            .src_ip("192.168.1.100")
            .src_user("alice")
            .action("login")
            .message("User logged in successfully")
    }

    fn sample_config() -> SiemConfig {
        SiemConfig {
            cef_vendor: "Armature".to_string(),
            cef_product: "WebApp".to_string(),
            cef_version: "1.0".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_cef_format() {
        let formatter = CefFormatter;
        let event = sample_event();
        let config = sample_config();

        let result = formatter.format(&event, &config).unwrap();

        assert!(result.starts_with("CEF:0|Armature|WebApp|1.0|"));
        assert!(result.contains("login"));
        assert!(result.contains("src=192.168.1.100"));
        assert!(result.contains("suser=alice"));
    }

    #[test]
    fn test_cef_escape_header() {
        assert_eq!(CefFormatter::escape_header("test|pipe"), "test\\|pipe");
        assert_eq!(CefFormatter::escape_header("test\\slash"), "test\\\\slash");
    }

    #[test]
    fn test_cef_escape_extension() {
        assert_eq!(CefFormatter::escape_extension("key=value"), "key\\=value");
        assert_eq!(
            CefFormatter::escape_extension("line1\nline2"),
            "line1\\nline2"
        );
    }
}
