//! SIEM event types and structures

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Severity level for SIEM events
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum SiemSeverity {
    /// Unknown severity (0)
    Unknown = 0,
    /// Low severity (1-3)
    Low = 1,
    /// Medium severity (4-6)
    Medium = 4,
    /// High severity (7-8)
    High = 7,
    /// Critical severity (9-10)
    Critical = 9,
}

impl SiemSeverity {
    /// Get numeric value (0-10 scale for CEF)
    pub fn as_cef_severity(&self) -> u8 {
        match self {
            SiemSeverity::Unknown => 0,
            SiemSeverity::Low => 3,
            SiemSeverity::Medium => 5,
            SiemSeverity::High => 8,
            SiemSeverity::Critical => 10,
        }
    }

    /// Get syslog severity (0-7)
    pub fn as_syslog_severity(&self) -> u8 {
        match self {
            SiemSeverity::Unknown => 6,  // Informational
            SiemSeverity::Low => 5,      // Notice
            SiemSeverity::Medium => 4,   // Warning
            SiemSeverity::High => 3,     // Error
            SiemSeverity::Critical => 2, // Critical
        }
    }
}

impl Default for SiemSeverity {
    fn default() -> Self {
        SiemSeverity::Unknown
    }
}

/// Event outcome
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventOutcome {
    /// Operation succeeded
    Success,
    /// Operation failed
    Failure,
    /// Outcome unknown
    Unknown,
}

impl Default for EventOutcome {
    fn default() -> Self {
        EventOutcome::Unknown
    }
}

/// Event category (aligned with ECS categories)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    /// Authentication events
    Authentication,
    /// Authorization events
    Authorization,
    /// Configuration changes
    Configuration,
    /// Database activity
    Database,
    /// File system activity
    File,
    /// Host activity
    Host,
    /// IAM activity
    Iam,
    /// Intrusion detection
    IntrusionDetection,
    /// Malware activity
    Malware,
    /// Network activity
    Network,
    /// Package management
    Package,
    /// Process activity
    Process,
    /// Registry activity
    Registry,
    /// Session activity
    Session,
    /// Threat intelligence
    Threat,
    /// Vulnerability
    Vulnerability,
    /// Web activity
    Web,
    /// Custom category
    Custom(String),
}

impl Default for EventCategory {
    fn default() -> Self {
        EventCategory::Web
    }
}

impl std::fmt::Display for EventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventCategory::Authentication => write!(f, "authentication"),
            EventCategory::Authorization => write!(f, "authorization"),
            EventCategory::Configuration => write!(f, "configuration"),
            EventCategory::Database => write!(f, "database"),
            EventCategory::File => write!(f, "file"),
            EventCategory::Host => write!(f, "host"),
            EventCategory::Iam => write!(f, "iam"),
            EventCategory::IntrusionDetection => write!(f, "intrusion_detection"),
            EventCategory::Malware => write!(f, "malware"),
            EventCategory::Network => write!(f, "network"),
            EventCategory::Package => write!(f, "package"),
            EventCategory::Process => write!(f, "process"),
            EventCategory::Registry => write!(f, "registry"),
            EventCategory::Session => write!(f, "session"),
            EventCategory::Threat => write!(f, "threat"),
            EventCategory::Vulnerability => write!(f, "vulnerability"),
            EventCategory::Web => write!(f, "web"),
            EventCategory::Custom(s) => write!(f, "{}", s),
        }
    }
}

/// SIEM event structure
///
/// A normalized event structure that can be converted to various SIEM formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiemEvent {
    /// Unique event ID
    pub id: String,

    /// Timestamp when event occurred
    pub timestamp: DateTime<Utc>,

    /// Event type/name (e.g., "user.login", "file.access")
    pub event_type: String,

    /// Event category
    pub category: EventCategory,

    /// Severity level
    pub severity: SiemSeverity,

    /// Event outcome
    pub outcome: EventOutcome,

    /// Event action (e.g., "login", "logout", "create", "delete")
    pub action: String,

    /// Event message/description
    pub message: Option<String>,

    // Source information
    /// Source IP address
    pub src_ip: Option<String>,
    /// Source port
    pub src_port: Option<u16>,
    /// Source hostname
    pub src_host: Option<String>,
    /// Source user
    pub src_user: Option<String>,
    /// Source process
    pub src_process: Option<String>,

    // Destination information
    /// Destination IP address
    pub dst_ip: Option<String>,
    /// Destination port
    pub dst_port: Option<u16>,
    /// Destination hostname
    pub dst_host: Option<String>,
    /// Destination user
    pub dst_user: Option<String>,

    // Request information (for web events)
    /// HTTP method
    pub http_method: Option<String>,
    /// Request URL/path
    pub url: Option<String>,
    /// User agent
    pub user_agent: Option<String>,
    /// HTTP status code
    pub http_status: Option<u16>,
    /// Request/response size in bytes
    pub bytes: Option<u64>,

    // Resource information
    /// Resource type being accessed
    pub resource_type: Option<String>,
    /// Resource identifier
    pub resource_id: Option<String>,
    /// Resource name
    pub resource_name: Option<String>,

    // Application context
    /// Application name
    pub application: Option<String>,
    /// Service name
    pub service: Option<String>,
    /// Environment (prod, staging, dev)
    pub environment: Option<String>,

    // Error information
    /// Error code
    pub error_code: Option<String>,
    /// Error message
    pub error_message: Option<String>,

    // Duration
    /// Event duration in milliseconds
    pub duration_ms: Option<u64>,

    // Protocol information
    /// Network protocol
    pub protocol: Option<String>,

    // Device information
    /// Device ID
    pub device_id: Option<String>,
    /// Device type
    pub device_type: Option<String>,

    // Threat information
    /// Threat indicator type
    pub threat_indicator_type: Option<String>,
    /// Threat indicator value
    pub threat_indicator: Option<String>,

    // File information (for file events)
    /// File path
    pub file_path: Option<String>,
    /// File hash
    pub file_hash: Option<String>,
    /// File size
    pub file_size: Option<u64>,

    /// Additional metadata
    pub metadata: HashMap<String, serde_json::Value>,

    /// Raw event data (if preserving original)
    pub raw: Option<String>,
}

impl SiemEvent {
    /// Create a new SIEM event
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_siem::*;
    ///
    /// let event = SiemEvent::new("user.login")
    ///     .category(EventCategory::Authentication)
    ///     .severity(SiemSeverity::Low)
    ///     .outcome(EventOutcome::Success)
    ///     .src_ip("192.168.1.100")
    ///     .src_user("alice")
    ///     .action("login");
    /// ```
    pub fn new(event_type: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type: event_type.into(),
            category: EventCategory::default(),
            severity: SiemSeverity::default(),
            outcome: EventOutcome::default(),
            action: "unknown".to_string(),
            message: None,
            src_ip: None,
            src_port: None,
            src_host: None,
            src_user: None,
            src_process: None,
            dst_ip: None,
            dst_port: None,
            dst_host: None,
            dst_user: None,
            http_method: None,
            url: None,
            user_agent: None,
            http_status: None,
            bytes: None,
            resource_type: None,
            resource_id: None,
            resource_name: None,
            application: None,
            service: None,
            environment: None,
            error_code: None,
            error_message: None,
            duration_ms: None,
            protocol: None,
            device_id: None,
            device_type: None,
            threat_indicator_type: None,
            threat_indicator: None,
            file_path: None,
            file_hash: None,
            file_size: None,
            metadata: HashMap::new(),
            raw: None,
        }
    }

    /// Set event category
    pub fn category(mut self, category: EventCategory) -> Self {
        self.category = category;
        self
    }

    /// Set severity
    pub fn severity(mut self, severity: SiemSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Set outcome
    pub fn outcome(mut self, outcome: EventOutcome) -> Self {
        self.outcome = outcome;
        self
    }

    /// Set action
    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = action.into();
        self
    }

    /// Set message
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Set source IP
    pub fn src_ip(mut self, ip: impl Into<String>) -> Self {
        self.src_ip = Some(ip.into());
        self
    }

    /// Set source port
    pub fn src_port(mut self, port: u16) -> Self {
        self.src_port = Some(port);
        self
    }

    /// Set source hostname
    pub fn src_host(mut self, host: impl Into<String>) -> Self {
        self.src_host = Some(host.into());
        self
    }

    /// Set source user
    pub fn src_user(mut self, user: impl Into<String>) -> Self {
        self.src_user = Some(user.into());
        self
    }

    /// Set source process
    pub fn src_process(mut self, process: impl Into<String>) -> Self {
        self.src_process = Some(process.into());
        self
    }

    /// Set destination IP
    pub fn dst_ip(mut self, ip: impl Into<String>) -> Self {
        self.dst_ip = Some(ip.into());
        self
    }

    /// Set destination port
    pub fn dst_port(mut self, port: u16) -> Self {
        self.dst_port = Some(port);
        self
    }

    /// Set destination hostname
    pub fn dst_host(mut self, host: impl Into<String>) -> Self {
        self.dst_host = Some(host.into());
        self
    }

    /// Set destination user
    pub fn dst_user(mut self, user: impl Into<String>) -> Self {
        self.dst_user = Some(user.into());
        self
    }

    /// Set HTTP method
    pub fn http_method(mut self, method: impl Into<String>) -> Self {
        self.http_method = Some(method.into());
        self
    }

    /// Set URL
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set user agent
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    /// Set HTTP status
    pub fn http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    /// Set bytes
    pub fn bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    /// Set resource type
    pub fn resource_type(mut self, rt: impl Into<String>) -> Self {
        self.resource_type = Some(rt.into());
        self
    }

    /// Set resource ID
    pub fn resource_id(mut self, id: impl Into<String>) -> Self {
        self.resource_id = Some(id.into());
        self
    }

    /// Set resource name
    pub fn resource_name(mut self, name: impl Into<String>) -> Self {
        self.resource_name = Some(name.into());
        self
    }

    /// Set application
    pub fn application(mut self, app: impl Into<String>) -> Self {
        self.application = Some(app.into());
        self
    }

    /// Set service
    pub fn service(mut self, service: impl Into<String>) -> Self {
        self.service = Some(service.into());
        self
    }

    /// Set environment
    pub fn environment(mut self, env: impl Into<String>) -> Self {
        self.environment = Some(env.into());
        self
    }

    /// Set error code
    pub fn error_code(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self
    }

    /// Set error message
    pub fn error_message(mut self, msg: impl Into<String>) -> Self {
        self.error_message = Some(msg.into());
        self
    }

    /// Set duration in milliseconds
    pub fn duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    /// Set protocol
    pub fn protocol(mut self, proto: impl Into<String>) -> Self {
        self.protocol = Some(proto.into());
        self
    }

    /// Set device ID
    pub fn device_id(mut self, id: impl Into<String>) -> Self {
        self.device_id = Some(id.into());
        self
    }

    /// Set device type
    pub fn device_type(mut self, dt: impl Into<String>) -> Self {
        self.device_type = Some(dt.into());
        self
    }

    /// Set threat indicator
    pub fn threat_indicator(
        mut self,
        indicator_type: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.threat_indicator_type = Some(indicator_type.into());
        self.threat_indicator = Some(value.into());
        self
    }

    /// Set file path
    pub fn file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Set file hash
    pub fn file_hash(mut self, hash: impl Into<String>) -> Self {
        self.file_hash = Some(hash.into());
        self
    }

    /// Set file size
    pub fn file_size(mut self, size: u64) -> Self {
        self.file_size = Some(size);
        self
    }

    /// Add metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Set raw event data
    pub fn raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }

    /// Set timestamp
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Set custom ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
}

#[cfg(feature = "audit")]
impl From<armature_audit::AuditEvent> for SiemEvent {
    fn from(audit: armature_audit::AuditEvent) -> Self {
        let severity = match audit.severity {
            armature_audit::AuditSeverity::Info => SiemSeverity::Low,
            armature_audit::AuditSeverity::Warning => SiemSeverity::Medium,
            armature_audit::AuditSeverity::Error => SiemSeverity::High,
            armature_audit::AuditSeverity::Critical => SiemSeverity::Critical,
        };

        let outcome = match audit.status {
            armature_audit::AuditStatus::Success => EventOutcome::Success,
            armature_audit::AuditStatus::Failure
            | armature_audit::AuditStatus::Denied
            | armature_audit::AuditStatus::Error => EventOutcome::Failure,
        };

        let mut event = SiemEvent::new(&audit.event_type)
            .with_id(&audit.id)
            .with_timestamp(audit.timestamp)
            .category(EventCategory::Web)
            .severity(severity)
            .outcome(outcome)
            .action(&audit.action);

        if let Some(user) = audit.user_id {
            event = event.src_user(user);
        }
        if let Some(ip) = audit.ip_address {
            event = event.src_ip(ip);
        }
        if let Some(ua) = audit.user_agent {
            event = event.user_agent(ua);
        }
        if let Some(method) = audit.method {
            event = event.http_method(method);
        }
        if let Some(path) = audit.path {
            event = event.url(path);
        }
        if let Some(status) = audit.status_code {
            event = event.http_status(status);
        }
        if let Some(rt) = audit.resource_type {
            event = event.resource_type(rt);
        }
        if let Some(rid) = audit.resource_id {
            event = event.resource_id(rid);
        }
        if let Some(err) = audit.error {
            event = event.error_message(err);
        }
        if let Some(dur) = audit.duration_ms {
            event = event.duration_ms(dur);
        }

        // Copy metadata
        for (k, v) in audit.metadata {
            event = event.metadata(k, v);
        }

        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let event = SiemEvent::new("user.login")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::Low)
            .outcome(EventOutcome::Success)
            .src_ip("192.168.1.100")
            .src_user("alice")
            .action("login");

        assert_eq!(event.event_type, "user.login");
        assert_eq!(event.category, EventCategory::Authentication);
        assert_eq!(event.severity, SiemSeverity::Low);
        assert_eq!(event.outcome, EventOutcome::Success);
        assert_eq!(event.src_ip, Some("192.168.1.100".to_string()));
        assert_eq!(event.src_user, Some("alice".to_string()));
    }

    #[test]
    fn test_severity_conversion() {
        assert_eq!(SiemSeverity::Unknown.as_cef_severity(), 0);
        assert_eq!(SiemSeverity::Low.as_cef_severity(), 3);
        assert_eq!(SiemSeverity::Medium.as_cef_severity(), 5);
        assert_eq!(SiemSeverity::High.as_cef_severity(), 8);
        assert_eq!(SiemSeverity::Critical.as_cef_severity(), 10);

        assert_eq!(SiemSeverity::Critical.as_syslog_severity(), 2);
    }

    #[test]
    fn test_event_metadata() {
        let event = SiemEvent::new("test")
            .metadata("custom_field", serde_json::json!("value"))
            .metadata("count", serde_json::json!(42));

        assert_eq!(
            event.metadata.get("custom_field"),
            Some(&serde_json::json!("value"))
        );
        assert_eq!(event.metadata.get("count"), Some(&serde_json::json!(42)));
    }
}
