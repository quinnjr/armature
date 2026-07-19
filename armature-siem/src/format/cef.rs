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
            .replace(['\n', '\r'], " ")
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

        // Add custom metadata into distinct CEF custom slots: string-valued
        // entries go into `cs1..cs6` (with `cs{n}Label`), numeric-valued
        // entries go into `cn1..cn3` (with `cn{n}Label`). CEF's `cn1` is
        // only reserved for `httpStatusCode` when that field is present
        // above; otherwise numeric metadata may use `cn1` too. Once all
        // slots for a value's kind are exhausted, remaining metadata is
        // serialized as a single JSON blob into a non-standard
        // `additionalMetadata` extension key (distinct from `cs6`, which
        // may independently still hold a 6th string metadata value) rather
        // than being silently dropped or overwriting an already-used slot.
        const MAX_CS: usize = 6;
        const MAX_CN: usize = 3;
        // cn1 is reserved for httpStatusCode above only when it was set.
        let cn_start: usize = if event.http_status.is_some() { 2 } else { 1 };

        let mut next_cs = 1usize;
        let mut next_cn = cn_start;
        let mut overflow = serde_json::Map::new();

        // Iterate in a stable, sorted-by-key order rather than the
        // HashMap's arbitrary (and run-to-run varying) iteration order, so
        // slot assignment (cs1..cs6 / cn1..cn3 / overflow) is deterministic
        // for a given set of metadata keys. This keeps CEF output
        // reproducible, which SIEM correlation rules pinned on slot numbers
        // depend on.
        let mut entries: Vec<_> = event.metadata.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        for (key, value) in entries {
            let is_numeric = value.is_number();

            if is_numeric && next_cn <= MAX_CN {
                ext.push(format!("cn{}={}", next_cn, value));
                ext.push(format!(
                    "cn{}Label={}",
                    next_cn,
                    Self::escape_extension(key)
                ));
                next_cn += 1;
                continue;
            }

            if !is_numeric && next_cs <= MAX_CS {
                let val_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                ext.push(format!(
                    "cs{}={} cs{}Label={}",
                    next_cs,
                    Self::escape_extension(&val_str),
                    next_cs,
                    Self::escape_extension(key)
                ));
                next_cs += 1;
                continue;
            }

            // Slots for this value's kind (or both) are exhausted: don't
            // drop the data, carry it in the overflow blob instead.
            overflow.insert(key.clone(), value.clone());
        }

        if !overflow.is_empty() {
            let overflow_json = serde_json::Value::Object(overflow).to_string();
            ext.push(format!(
                "additionalMetadata={}",
                Self::escape_extension(&overflow_json)
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
    fn test_cef_metadata_uses_distinct_custom_slots() {
        let formatter = CefFormatter;
        let event = sample_event()
            .metadata("tenant", serde_json::json!("acme-corp"))
            .metadata("risk_score", serde_json::json!("high"));
        let config = sample_config();

        let result = formatter.format(&event, &config).unwrap();

        // Both metadata values must appear, in distinct cs slots (not one
        // overwriting the other).
        assert!(
            result.contains("cs1=acme-corp") || result.contains("cs2=acme-corp"),
            "expected tenant value in a cs slot: {result}"
        );
        assert!(
            result.contains("cs1=high") || result.contains("cs2=high"),
            "expected risk_score value in a cs slot: {result}"
        );
        assert!(
            result.contains("acme-corp") && result.contains("high"),
            "both metadata values must survive formatting: {result}"
        );
        // The two values must not land in the same slot.
        assert!(
            !(result.contains("cs1=acme-corp") && result.contains("cs1=high")),
            "metadata entries collided on the same cs slot: {result}"
        );
    }

    #[test]
    fn test_cef_metadata_overflow_preserves_all_string_values() {
        let formatter = CefFormatter;
        let mut event = sample_event();
        // 8 string metadata entries: first 6 go into cs1..cs6, the
        // remaining 2 must still appear, carried in additionalMetadata.
        for i in 1..=8 {
            event = event.metadata(format!("key{i}"), serde_json::json!(format!("val{i}")));
        }
        let config = sample_config();

        let result = formatter.format(&event, &config).unwrap();

        for i in 1..=6 {
            assert!(
                result.contains(&format!("val{i}")),
                "expected val{i} in a cs slot: {result}"
            );
        }
        assert!(
            result.contains("cs6="),
            "expected cs6 slot to be used: {result}"
        );
        assert!(
            result.contains("additionalMetadata="),
            "expected overflow extension key: {result}"
        );
        for i in 7..=8 {
            assert!(
                result.contains(&format!("val{i}")),
                "expected overflow val{i} to survive in additionalMetadata: {result}"
            );
        }
    }

    #[test]
    fn test_cef_numeric_metadata_uses_cn1_when_no_http_status() {
        let formatter = CefFormatter;
        let event = sample_event().metadata("risk_score", serde_json::json!(42));
        let config = sample_config();
        assert!(event.http_status.is_none());

        let result = formatter.format(&event, &config).unwrap();

        assert!(
            result.contains("cn1=42"),
            "expected numeric metadata to use cn1 when http_status is absent: {result}"
        );
        assert!(
            result.contains("cn1Label=risk_score"),
            "expected cn1Label to be set: {result}"
        );
    }

    #[test]
    fn test_cef_metadata_slot_assignment_is_deterministic() {
        let formatter = CefFormatter;
        let config = sample_config();

        // More than 6 string metadata keys, so slot assignment (which keys
        // land in cs1..cs6 vs the additionalMetadata overflow) exercises
        // the full assignment logic. Build the event fresh for each run to
        // rule out any incidental HashMap ordering artifacts from reuse.
        let build_event = || {
            let mut event = sample_event();
            for i in 1..=8 {
                event = event.metadata(format!("key{i}"), serde_json::json!(format!("val{i}")));
            }
            event
        };

        let result_a = formatter.format(&build_event(), &config).unwrap();
        let result_b = formatter.format(&build_event(), &config).unwrap();

        // Compare only the metadata-slot portion (from cs1 onward); the
        // rest of the extension (e.g. `externalId`, a per-event UUID) is
        // expected to differ between two freshly built events and isn't
        // relevant to slot-assignment determinism.
        let slots_a = &result_a[result_a.find("cs1=").expect("cs1 present")..];
        let slots_b = &result_b[result_b.find("cs1=").expect("cs1 present")..];
        assert_eq!(
            slots_a, slots_b,
            "CEF slot assignment must be deterministic across runs for the same input"
        );

        // Since keys are sorted lexicographically before assignment,
        // key1..key6 must land in cs1..cs6 respectively, and key7/key8 must
        // overflow into additionalMetadata.
        for i in 1..=6 {
            assert!(
                result_a.contains(&format!("cs{i}=val{i} cs{i}Label=key{i}")),
                "expected key{i}/val{i} pinned to cs{i}: {result_a}"
            );
        }
        assert!(
            result_a.contains("additionalMetadata="),
            "expected overflow for key7/key8: {result_a}"
        );
        assert!(result_a.contains("val7") && result_a.contains("val8"));
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
