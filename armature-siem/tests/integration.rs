//! Integration tests for armature-siem

use armature_siem::*;

#[test]
fn test_siem_event_builder() {
    let event = SiemEvent::new("security.alert")
        .category(EventCategory::IntrusionDetection)
        .severity(SiemSeverity::Critical)
        .outcome(EventOutcome::Success)
        .src_ip("10.0.0.100")
        .src_port(12345)
        .dst_ip("192.168.1.1")
        .dst_port(443)
        .src_user("attacker")
        .protocol("TCP")
        .action("blocked")
        .message("Suspicious connection blocked")
        .threat_indicator("ip", "10.0.0.100")
        .metadata("rule_id", serde_json::json!("RULE-001"))
        .metadata("confidence", serde_json::json!(0.95));

    assert_eq!(event.event_type, "security.alert");
    assert_eq!(event.severity, SiemSeverity::Critical);
    assert_eq!(event.src_ip, Some("10.0.0.100".to_string()));
    assert_eq!(event.dst_port, Some(443));
    assert_eq!(event.threat_indicator, Some("10.0.0.100".to_string()));
    assert_eq!(event.metadata.len(), 2);
}

#[test]
fn test_cef_formatting() {
    let event = SiemEvent::new("user.login")
        .category(EventCategory::Authentication)
        .severity(SiemSeverity::Low)
        .outcome(EventOutcome::Success)
        .src_ip("192.168.1.100")
        .src_user("alice")
        .action("login")
        .message("User logged in");

    let config = SiemConfig {
        provider: SiemProvider::ArcSight,
        cef_vendor: "TestVendor".to_string(),
        cef_product: "TestProduct".to_string(),
        cef_version: "1.0".to_string(),
        ..Default::default()
    };

    let formatter = get_formatter(EventFormat::Cef);
    let result = formatter.format(&event, &config).unwrap();

    assert!(result.starts_with("CEF:0|TestVendor|TestProduct|1.0|"));
    assert!(result.contains("src=192.168.1.100"));
    assert!(result.contains("suser=alice"));
    assert!(result.contains("|3|")); // Severity 3 for Low
}

#[test]
fn test_leef_formatting() {
    let event = SiemEvent::new("auth.failure")
        .category(EventCategory::Authentication)
        .severity(SiemSeverity::High)
        .outcome(EventOutcome::Failure)
        .src_ip("10.0.0.50")
        .src_user("badactor")
        .action("login")
        .message("Authentication failed");

    let config = SiemConfig {
        provider: SiemProvider::QRadar,
        cef_vendor: "Armature".to_string(),
        cef_product: "Security".to_string(),
        cef_version: "1.0".to_string(),
        ..Default::default()
    };

    let formatter = get_formatter(EventFormat::Leef);
    let result = formatter.format(&event, &config).unwrap();

    assert!(result.starts_with("LEEF:2.0|Armature|Security|1.0|auth.failure|"));
    assert!(result.contains("src=10.0.0.50"));
    assert!(result.contains("usrName=badactor"));
    assert!(result.contains("outcome=failure"));
}

#[test]
fn test_syslog_formatting() {
    let event = SiemEvent::new("session.created")
        .category(EventCategory::Session)
        .severity(SiemSeverity::Low)
        .outcome(EventOutcome::Success)
        .src_user("testuser")
        .action("create")
        .message("Session started");

    let config = SiemConfig {
        provider: SiemProvider::Syslog,
        app_name: "testapp".to_string(),
        syslog_facility: SyslogFacility::Local0,
        ..Default::default()
    };

    let formatter = get_formatter(EventFormat::Syslog);
    let result = formatter.format(&event, &config).unwrap();

    // Local0 (16) * 8 + Notice (5) = 133
    assert!(result.starts_with("<133>1 "));
    assert!(result.contains("testapp"));
    assert!(result.contains("session.created"));
    assert!(result.contains("[armature@32473"));
}

#[test]
fn test_ecs_formatting() {
    let event = SiemEvent::new("http.request")
        .category(EventCategory::Web)
        .severity(SiemSeverity::Low)
        .outcome(EventOutcome::Success)
        .src_ip("192.168.1.50")
        .http_method("GET")
        .http_status(200)
        .url("/api/users")
        .user_agent("Mozilla/5.0")
        .action("request");

    let config = SiemConfig::default();

    let formatter = get_formatter(EventFormat::Ecs);
    let result = formatter.format(&event, &config).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed.get("@timestamp").is_some());
    assert_eq!(parsed["event"]["action"], "request");
    assert_eq!(parsed["source"]["ip"], "192.168.1.50");
    assert_eq!(parsed["http"]["request"]["method"], "GET");
    assert_eq!(parsed["http"]["response"]["status_code"], 200);
    assert_eq!(parsed["ecs"]["version"], "8.11");
}

#[test]
fn test_json_formatting_splunk() {
    let event = SiemEvent::new("test.event")
        .category(EventCategory::Web)
        .action("test");

    let config = SiemConfig {
        provider: SiemProvider::Splunk,
        source: Some("armature".to_string()),
        source_type: Some("security".to_string()),
        index: Some("main".to_string()),
        ..Default::default()
    };

    let formatter = get_formatter(EventFormat::Json);
    let result = formatter.format(&event, &config).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed.get("time").is_some());
    assert!(parsed.get("host").is_some());
    assert_eq!(parsed["source"], "armature");
    assert_eq!(parsed["sourcetype"], "security");
    assert!(parsed.get("event").is_some());
}

#[test]
fn test_provider_configs() {
    // Test Splunk HEC config
    let splunk_config = SplunkConfig::hec("https://splunk.example.com:8088")
        .token("test-token")
        .build()
        .unwrap();
    assert_eq!(splunk_config.provider, SiemProvider::Splunk);
    assert!(splunk_config.endpoint.contains("/services/collector"));

    // Test Elastic config
    let elastic_config = ElasticConfig::new("https://elastic.example.com:9200")
        .index("security")
        .build()
        .unwrap();
    assert_eq!(elastic_config.provider, SiemProvider::Elastic);
    assert_eq!(elastic_config.format, EventFormat::Ecs);

    // Test QRadar config
    let qradar_config = QRadarConfig::leef("qradar.example.com:514")
        .build()
        .unwrap();
    assert_eq!(qradar_config.provider, SiemProvider::QRadar);
    assert_eq!(qradar_config.format, EventFormat::Leef);

    // Test Sentinel config
    let sentinel_config = SentinelConfig::new("workspace-id", "shared-key")
        .build()
        .unwrap();
    assert_eq!(sentinel_config.provider, SiemProvider::Sentinel);
    assert!(sentinel_config.endpoint.contains("opinsights.azure.com"));

    // Test Datadog config
    let datadog_config = DatadogConfig::new("api-key").build().unwrap();
    assert_eq!(datadog_config.provider, SiemProvider::Datadog);
    assert!(datadog_config.endpoint.contains("datadoghq.com"));
}

#[test]
fn test_config_validation() {
    // Empty endpoint should fail
    let result = SiemConfig::builder().provider(SiemProvider::Splunk).build();
    assert!(result.is_err());

    // Wrong transport/endpoint combo should fail
    let result = SiemConfig::builder()
        .provider(SiemProvider::Splunk)
        .endpoint("splunk.example.com") // Missing http://
        .transport(Transport::Https)
        .build();
    assert!(result.is_err());

    // Valid config should succeed
    let result = SiemConfig::builder()
        .provider(SiemProvider::Splunk)
        .endpoint("https://splunk.example.com:8088")
        .token("test")
        .build();
    assert!(result.is_ok());
}

#[test]
fn test_severity_mappings() {
    // CEF severity (0-10)
    assert_eq!(SiemSeverity::Unknown.as_cef_severity(), 0);
    assert_eq!(SiemSeverity::Low.as_cef_severity(), 3);
    assert_eq!(SiemSeverity::Medium.as_cef_severity(), 5);
    assert_eq!(SiemSeverity::High.as_cef_severity(), 8);
    assert_eq!(SiemSeverity::Critical.as_cef_severity(), 10);

    // Syslog severity (0-7, lower is more severe)
    assert_eq!(SiemSeverity::Unknown.as_syslog_severity(), 6); // Info
    assert_eq!(SiemSeverity::Low.as_syslog_severity(), 5); // Notice
    assert_eq!(SiemSeverity::Medium.as_syslog_severity(), 4); // Warning
    assert_eq!(SiemSeverity::High.as_syslog_severity(), 3); // Error
    assert_eq!(SiemSeverity::Critical.as_syslog_severity(), 2); // Critical
}

#[test]
fn test_batch_formatting() {
    let events = vec![
        SiemEvent::new("event.1").action("test1"),
        SiemEvent::new("event.2").action("test2"),
        SiemEvent::new("event.3").action("test3"),
    ];

    let config = SiemConfig {
        provider: SiemProvider::Splunk,
        ..Default::default()
    };

    let formatter = get_formatter(EventFormat::Json);
    let result = formatter.format_batch(&events, &config).unwrap();

    // Splunk expects newline-delimited JSON
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 3);
}

#[tokio::test]
async fn test_memory_transport() {
    let transport = MemoryTransport::new();

    transport.send("message 1", "text/plain").await.unwrap();
    transport.send("message 2", "text/plain").await.unwrap();

    let messages = transport.get_messages().await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], "message 1");
    assert_eq!(messages[1], "message 2");

    transport.clear().await;
    let messages = transport.get_messages().await;
    assert_eq!(messages.len(), 0);
}
