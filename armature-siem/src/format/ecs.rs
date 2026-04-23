//! Elastic Common Schema (ECS) formatter
//!
//! Formats events according to Elastic Common Schema for use with
//! Elasticsearch, Elastic Security, and Elastic SIEM.

use super::EventFormatter;
use crate::{SiemConfig, SiemEvent, SiemResult};
use serde::Serialize;
use std::collections::HashMap;

/// ECS (Elastic Common Schema) formatter
pub struct EcsFormatter;

/// ECS formatted event structure
#[derive(Serialize)]
struct EcsEvent<'a> {
    #[serde(rename = "@timestamp")]
    timestamp: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,

    event: EcsEventField<'a>,

    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<EcsSource<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    destination: Option<EcsDestination<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<EcsUser<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    http: Option<EcsHttp<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<EcsUrl<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<EcsUserAgent<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<EcsFile<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<EcsError<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<EcsProcess<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<EcsHost>,

    #[serde(skip_serializing_if = "Option::is_none")]
    service: Option<EcsService<'a>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<EcsNetwork<'a>>,

    #[serde(skip_serializing_if = "HashMap::is_empty")]
    labels: HashMap<String, serde_json::Value>,

    ecs: EcsVersion,
}

#[derive(Serialize)]
struct EcsEventField<'a> {
    id: &'a str,
    kind: &'static str,
    category: Vec<String>,
    #[serde(rename = "type")]
    event_type: Vec<&'a str>,
    action: &'a str,
    outcome: &'static str,
    severity: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u64>,
    original: &'a str,
}

#[derive(Serialize)]
struct EcsSource<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsDestination<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsUser<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsHttp<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<EcsHttpRequest<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<EcsHttpResponse>,
}

#[derive(Serialize)]
struct EcsHttpRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
}

#[derive(Serialize)]
struct EcsHttpResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    status_code: Option<u16>,
}

#[derive(Serialize)]
struct EcsUrl<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    original: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsUserAgent<'a> {
    original: &'a str,
}

#[derive(Serialize)]
struct EcsFile<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<EcsFileHash<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

#[derive(Serialize)]
struct EcsFileHash<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsError<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsProcess<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsHost {
    name: String,
}

#[derive(Serialize)]
struct EcsService<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'a str>,
}

#[derive(Serialize)]
struct EcsNetwork<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
}

#[derive(Serialize)]
struct EcsVersion {
    version: &'static str,
}

impl EcsFormatter {
    fn get_hostname() -> String {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    fn map_outcome(outcome: crate::EventOutcome) -> &'static str {
        match outcome {
            crate::EventOutcome::Success => "success",
            crate::EventOutcome::Failure => "failure",
            crate::EventOutcome::Unknown => "unknown",
        }
    }
}

impl EventFormatter for EcsFormatter {
    fn format(&self, event: &SiemEvent, _config: &SiemConfig) -> SiemResult<String> {
        // Build source if any source fields are present
        let source =
            if event.src_ip.is_some() || event.src_port.is_some() || event.src_host.is_some() {
                Some(EcsSource {
                    ip: event.src_ip.as_deref(),
                    port: event.src_port,
                    domain: event.src_host.as_deref(),
                })
            } else {
                None
            };

        // Build destination if any dest fields are present
        let destination =
            if event.dst_ip.is_some() || event.dst_port.is_some() || event.dst_host.is_some() {
                Some(EcsDestination {
                    ip: event.dst_ip.as_deref(),
                    port: event.dst_port,
                    domain: event.dst_host.as_deref(),
                })
            } else {
                None
            };

        // Build user
        let user = if event.src_user.is_some() {
            Some(EcsUser {
                name: event.src_user.as_deref(),
                id: None,
            })
        } else {
            None
        };

        // Build HTTP
        let http = if event.http_method.is_some() || event.http_status.is_some() {
            Some(EcsHttp {
                request: if event.http_method.is_some() || event.bytes.is_some() {
                    Some(EcsHttpRequest {
                        method: event.http_method.as_deref(),
                        bytes: event.bytes,
                    })
                } else {
                    None
                },
                response: if event.http_status.is_some() {
                    Some(EcsHttpResponse {
                        status_code: event.http_status,
                    })
                } else {
                    None
                },
            })
        } else {
            None
        };

        // Build URL
        let url = event.url.as_ref().map(|u| EcsUrl {
            original: Some(u.as_str()),
            path: Some(u.as_str()),
        });

        // Build user agent
        let user_agent = event.user_agent.as_ref().map(|ua| EcsUserAgent {
            original: ua.as_str(),
        });

        // Build file
        let file = if event.file_path.is_some()
            || event.file_hash.is_some()
            || event.file_size.is_some()
        {
            Some(EcsFile {
                path: event.file_path.as_deref(),
                hash: event.file_hash.as_ref().map(|h| EcsFileHash {
                    sha256: Some(h.as_str()),
                }),
                size: event.file_size,
            })
        } else {
            None
        };

        // Build error
        let error = if event.error_code.is_some() || event.error_message.is_some() {
            Some(EcsError {
                code: event.error_code.as_deref(),
                message: event.error_message.as_deref(),
            })
        } else {
            None
        };

        // Build process
        let process = event.src_process.as_ref().map(|p| EcsProcess {
            name: Some(p.as_str()),
        });

        // Build service
        let service = if event.service.is_some() || event.environment.is_some() {
            Some(EcsService {
                name: event.service.as_deref(),
                environment: event.environment.as_deref(),
            })
        } else {
            None
        };

        // Build network
        let network = if event.protocol.is_some() || event.bytes.is_some() {
            Some(EcsNetwork {
                protocol: event.protocol.as_deref(),
                bytes: event.bytes,
            })
        } else {
            None
        };

        let ecs_event = EcsEvent {
            timestamp: event.timestamp.to_rfc3339(),
            message: event.message.as_deref(),
            event: EcsEventField {
                id: &event.id,
                kind: "event",
                category: vec![event.category.to_string()],
                event_type: vec![&event.event_type],
                action: &event.action,
                outcome: Self::map_outcome(event.outcome),
                severity: event.severity.as_cef_severity(),
                duration: event.duration_ms.map(|d| d * 1_000_000), // Convert to nanoseconds
                original: &event.event_type,
            },
            source,
            destination,
            user,
            http,
            url,
            user_agent,
            file,
            error,
            process,
            host: Some(EcsHost {
                name: Self::get_hostname(),
            }),
            service,
            network,
            labels: event.metadata.clone(),
            ecs: EcsVersion { version: "8.11" },
        };

        Ok(serde_json::to_string(&ecs_event)?)
    }

    fn content_type(&self) -> &'static str {
        "application/json"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventCategory, EventOutcome, SiemSeverity};

    fn sample_event() -> SiemEvent {
        SiemEvent::new("user.authentication")
            .category(EventCategory::Authentication)
            .severity(SiemSeverity::Low)
            .outcome(EventOutcome::Success)
            .src_ip("192.168.1.100")
            .src_user("alice")
            .action("login")
            .message("User logged in successfully")
            .http_method("POST")
            .http_status(200)
            .url("/api/auth/login")
    }

    fn sample_config() -> SiemConfig {
        SiemConfig::default()
    }

    #[test]
    fn test_ecs_format() {
        let formatter = EcsFormatter;
        let event = sample_event();
        let config = sample_config();

        let result = formatter.format(&event, &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert!(parsed.get("@timestamp").is_some());
        assert_eq!(parsed["event"]["action"], "login");
        assert_eq!(parsed["event"]["outcome"], "success");
        assert_eq!(parsed["source"]["ip"], "192.168.1.100");
        assert_eq!(parsed["user"]["name"], "alice");
        assert_eq!(parsed["http"]["request"]["method"], "POST");
        assert_eq!(parsed["http"]["response"]["status_code"], 200);
        assert_eq!(parsed["ecs"]["version"], "8.11");
    }

    #[test]
    fn test_ecs_content_type() {
        let formatter = EcsFormatter;
        assert_eq!(formatter.content_type(), "application/json");
    }
}
