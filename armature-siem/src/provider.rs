//! SIEM provider-specific configurations and helpers
//!
//! This module provides pre-configured builders for popular SIEM systems.

use crate::config::{EventFormat, SiemConfig, SiemConfigBuilder, SiemProvider, Transport};

/// Splunk configuration helper
pub struct SplunkConfig;

impl SplunkConfig {
    /// Create a Splunk HEC configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = SplunkConfig::hec("https://splunk.example.com:8088")
    ///     .token("your-hec-token")
    ///     .index("security")
    ///     .source_type("armature:security")
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn hec(endpoint: impl Into<String>) -> SiemConfigBuilder {
        let mut endpoint_str = endpoint.into();
        if !endpoint_str.ends_with("/services/collector")
            && !endpoint_str.ends_with("/services/collector/event")
        {
            if endpoint_str.ends_with('/') {
                endpoint_str.push_str("services/collector");
            } else {
                endpoint_str.push_str("/services/collector");
            }
        }

        SiemConfig::builder()
            .provider(SiemProvider::Splunk)
            .endpoint(endpoint_str)
            .format(EventFormat::Json)
            .transport(Transport::Https)
    }

    /// Create a Splunk syslog configuration
    pub fn syslog(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Splunk)
            .endpoint(endpoint)
            .format(EventFormat::Syslog)
            .transport(Transport::Tcp)
    }
}

/// Elasticsearch/Elastic Security configuration helper
pub struct ElasticConfig;

impl ElasticConfig {
    /// Create an Elasticsearch configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = ElasticConfig::new("https://elastic.example.com:9200")
    ///     .index("security-events")
    ///     .basic_auth("elastic", "password")
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn new(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Elastic)
            .endpoint(endpoint)
            .format(EventFormat::Ecs)
            .transport(Transport::Https)
    }

    /// Create an Elastic Cloud configuration
    pub fn cloud(cloud_id: impl Into<String>, api_key: impl Into<String>) -> SiemConfigBuilder {
        // In a real implementation, we'd decode the cloud_id to get the endpoint
        // For now, just use it as the endpoint
        SiemConfig::builder()
            .provider(SiemProvider::Elastic)
            .endpoint(cloud_id)
            .token(api_key)
            .format(EventFormat::Ecs)
            .transport(Transport::Https)
    }
}

/// IBM QRadar configuration helper
pub struct QRadarConfig;

impl QRadarConfig {
    /// Create a QRadar LEEF configuration over TCP
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = QRadarConfig::leef("qradar.example.com:514")
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn leef(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::QRadar)
            .endpoint(endpoint)
            .format(EventFormat::Leef)
            .transport(Transport::Tcp)
    }

    /// Create a QRadar syslog configuration
    pub fn syslog(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::QRadar)
            .endpoint(endpoint)
            .format(EventFormat::Syslog)
            .transport(Transport::Tcp)
    }
}

/// Microsoft Sentinel configuration helper
pub struct SentinelConfig;

impl SentinelConfig {
    /// Create a Microsoft Sentinel configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = SentinelConfig::new(
    ///     "workspace-id",
    ///     "shared-key"
    /// ).build().unwrap();
    /// ```
    pub fn new(
        workspace_id: impl Into<String>,
        shared_key: impl Into<String>,
    ) -> SiemConfigBuilder {
        let workspace = workspace_id.into();
        let endpoint = format!(
            "https://{}.ods.opinsights.azure.com/api/logs?api-version=2016-04-01",
            workspace
        );

        SiemConfig::builder()
            .provider(SiemProvider::Sentinel)
            .endpoint(endpoint)
            .token(shared_key)
            .format(EventFormat::Json)
            .transport(Transport::Https)
    }
}

/// Sumo Logic configuration helper
pub struct SumoLogicConfig;

impl SumoLogicConfig {
    /// Create a Sumo Logic HTTP Source configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = SumoLogicConfig::http_source(
    ///     "https://collectors.sumologic.com/receiver/v1/http/TOKEN"
    /// ).build().unwrap();
    /// ```
    pub fn http_source(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::SumoLogic)
            .endpoint(endpoint)
            .format(EventFormat::Json)
            .transport(Transport::Https)
    }
}

/// Datadog configuration helper
pub struct DatadogConfig;

impl DatadogConfig {
    /// Create a Datadog configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = DatadogConfig::new("your-api-key")
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn new(api_key: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Datadog)
            .endpoint("https://http-intake.logs.datadoghq.com/api/v2/logs")
            .token(api_key)
            .format(EventFormat::Json)
            .transport(Transport::Https)
    }

    /// Create a Datadog EU configuration
    pub fn eu(api_key: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Datadog)
            .endpoint("https://http-intake.logs.datadoghq.eu/api/v2/logs")
            .token(api_key)
            .format(EventFormat::Json)
            .transport(Transport::Https)
    }
}

/// ArcSight configuration helper
pub struct ArcSightConfig;

impl ArcSightConfig {
    /// Create an ArcSight CEF configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = ArcSightConfig::cef("arcsight.example.com:514")
    ///     .cef_vendor("MyCompany")
    ///     .cef_product("MyApp")
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn cef(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::ArcSight)
            .endpoint(endpoint)
            .format(EventFormat::Cef)
            .transport(Transport::Tcp)
    }

    /// Create an ArcSight syslog configuration
    pub fn syslog(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::ArcSight)
            .endpoint(endpoint)
            .format(EventFormat::Syslog)
            .transport(Transport::Udp)
    }
}

/// Generic syslog configuration helper
pub struct SyslogConfig;

impl SyslogConfig {
    /// Create a UDP syslog configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_siem::*;
    ///
    /// let config = SyslogConfig::udp("syslog.example.com:514")
    ///     .app_name("myapp")
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn udp(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Syslog)
            .endpoint(endpoint)
            .format(EventFormat::Syslog)
            .transport(Transport::Udp)
    }

    /// Create a TCP syslog configuration
    pub fn tcp(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Syslog)
            .endpoint(endpoint)
            .format(EventFormat::Syslog)
            .transport(Transport::Tcp)
    }

    /// Create a TLS syslog configuration
    pub fn tls(endpoint: impl Into<String>) -> SiemConfigBuilder {
        SiemConfig::builder()
            .provider(SiemProvider::Syslog)
            .endpoint(endpoint)
            .format(EventFormat::Syslog)
            .transport(Transport::Tls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_splunk_hec_config() {
        let config = SplunkConfig::hec("https://splunk.example.com:8088")
            .token("test-token")
            .index("main")
            .build()
            .unwrap();

        assert_eq!(config.provider, SiemProvider::Splunk);
        assert!(config.endpoint.contains("/services/collector"));
        assert_eq!(config.token, Some("test-token".to_string()));
        assert_eq!(config.format, EventFormat::Json);
    }

    #[test]
    fn test_elastic_config() {
        let config = ElasticConfig::new("https://elastic.example.com:9200")
            .index("security")
            .build()
            .unwrap();

        assert_eq!(config.provider, SiemProvider::Elastic);
        assert_eq!(config.format, EventFormat::Ecs);
    }

    #[test]
    fn test_qradar_config() {
        let config = QRadarConfig::leef("qradar.example.com:514")
            .build()
            .unwrap();

        assert_eq!(config.provider, SiemProvider::QRadar);
        assert_eq!(config.format, EventFormat::Leef);
        assert_eq!(config.transport, Transport::Tcp);
    }

    #[test]
    fn test_sentinel_config() {
        let config = SentinelConfig::new("workspace-123", "shared-key")
            .build()
            .unwrap();

        assert_eq!(config.provider, SiemProvider::Sentinel);
        assert!(config.endpoint.contains("opinsights.azure.com"));
    }

    #[test]
    fn test_datadog_config() {
        let config = DatadogConfig::new("api-key").build().unwrap();

        assert_eq!(config.provider, SiemProvider::Datadog);
        assert!(config.endpoint.contains("datadoghq.com"));
    }

    #[test]
    fn test_syslog_config() {
        let config = SyslogConfig::udp("syslog.example.com:514")
            .app_name("test-app")
            .build()
            .unwrap();

        assert_eq!(config.provider, SiemProvider::Syslog);
        assert_eq!(config.format, EventFormat::Syslog);
        assert_eq!(config.transport, Transport::Udp);
    }
}
