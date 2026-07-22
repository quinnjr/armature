//! Integration tests for `CloudRunConfig::from_env` override parsing and
//! `service_url` sourcing. These mutate process-global env vars, so this is a
//! single sequential test to avoid inter-test races.

use armature_cloudrun::CloudRunConfig;

#[test]
fn from_env_parses_user_overrides_and_reads_service_url_from_env() {
    unsafe {
        std::env::set_var("PORT", "9090");
        std::env::set_var("MEMORY_LIMIT_MB", "512");
        std::env::set_var("CPU_LIMIT", "2.5");
        std::env::set_var("CLOUD_RUN_TIMEOUT_SECONDS", "120");
        std::env::set_var("CLOUD_RUN_CONCURRENCY", "42");
        std::env::set_var("CLOUD_RUN_URL", "https://svc-abc123-uc.a.run.app");
    }

    let cfg = CloudRunConfig::from_env();

    assert_eq!(cfg.port, 9090);
    assert_eq!(cfg.memory_limit_mb, Some(512));
    assert_eq!(cfg.cpu_limit, Some(2.5));
    assert_eq!(cfg.timeout_seconds, 120);
    assert_eq!(cfg.max_concurrent_requests, 42);

    // service_url comes straight from CLOUD_RUN_URL, not a fabricated string.
    assert_eq!(
        cfg.service_url().as_deref(),
        Some("https://svc-abc123-uc.a.run.app")
    );

    // Without the URL env var there is no fabricated URL.
    unsafe {
        std::env::remove_var("CLOUD_RUN_URL");
    }
    let cfg_no_url = CloudRunConfig::from_env();
    assert_eq!(cfg_no_url.service_url(), None);

    unsafe {
        std::env::remove_var("PORT");
        std::env::remove_var("MEMORY_LIMIT_MB");
        std::env::remove_var("CPU_LIMIT");
        std::env::remove_var("CLOUD_RUN_TIMEOUT_SECONDS");
        std::env::remove_var("CLOUD_RUN_CONCURRENCY");
    }
}
