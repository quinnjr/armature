//! GCP configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Credentials source for GCP authentication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialsSource {
    /// Use Application Default Credentials.
    #[default]
    ApplicationDefault,
    /// Use service account JSON file.
    ServiceAccountFile(String),
    /// Use service account JSON content.
    ServiceAccountJson(String),
    /// Use metadata server (for GCE, Cloud Run, etc.).
    MetadataServer,
    /// Use explicit access token.
    AccessToken(String),
}

/// GCP service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcpConfig {
    /// GCP project ID.
    pub project_id: Option<String>,
    /// Credentials source.
    #[serde(default)]
    pub credentials: CredentialsSource,
    /// Enabled services.
    #[serde(default)]
    pub enabled_services: HashSet<String>,
    /// Service-specific configurations.
    #[serde(default)]
    pub service_configs: std::collections::HashMap<String, serde_json::Value>,
    /// Custom endpoint URL (for emulators).
    pub emulator_host: Option<String>,
}

impl Default for GcpConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            credentials: CredentialsSource::ApplicationDefault,
            enabled_services: HashSet::new(),
            service_configs: std::collections::HashMap::new(),
            emulator_host: None,
        }
    }
}

impl GcpConfig {
    /// Create a new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder.
    pub fn builder() -> GcpConfigBuilder {
        GcpConfigBuilder::new()
    }

    /// Load configuration from environment variables.
    pub fn from_env() -> GcpConfigBuilder {
        let mut builder = GcpConfigBuilder::new();

        if let Ok(project) = std::env::var("GOOGLE_CLOUD_PROJECT") {
            builder = builder.project_id(project);
        } else if let Ok(project) = std::env::var("GCP_PROJECT") {
            builder = builder.project_id(project);
        } else if let Ok(project) = std::env::var("GCLOUD_PROJECT") {
            builder = builder.project_id(project);
        }

        if let Ok(creds_file) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            builder = builder.service_account_file(creds_file);
        }

        // Check for emulator hosts
        if let Ok(host) = std::env::var("PUBSUB_EMULATOR_HOST") {
            builder = builder.emulator_host(host);
        }
        if let Ok(host) = std::env::var("FIRESTORE_EMULATOR_HOST") {
            builder = builder.emulator_host(host);
        }
        if let Ok(host) = std::env::var("STORAGE_EMULATOR_HOST") {
            builder = builder.emulator_host(host);
        }

        builder
    }

    /// Check if a service is enabled.
    pub fn is_enabled(&self, service: &str) -> bool {
        self.enabled_services.contains(service)
    }

    /// Get service-specific configuration.
    pub fn service_config<T: serde::de::DeserializeOwned>(&self, service: &str) -> Option<T> {
        self.service_configs
            .get(service)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

/// Builder for GCP configuration.
#[derive(Default)]
pub struct GcpConfigBuilder {
    config: GcpConfig,
}

impl GcpConfigBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the project ID.
    pub fn project_id(mut self, project_id: impl Into<String>) -> Self {
        self.config.project_id = Some(project_id.into());
        self
    }

    /// Set the credentials source.
    pub fn credentials(mut self, credentials: CredentialsSource) -> Self {
        self.config.credentials = credentials;
        self
    }

    /// Use service account file.
    pub fn service_account_file(mut self, path: impl Into<String>) -> Self {
        self.config.credentials = CredentialsSource::ServiceAccountFile(path.into());
        self
    }

    /// Use service account JSON.
    pub fn service_account_json(mut self, json: impl Into<String>) -> Self {
        self.config.credentials = CredentialsSource::ServiceAccountJson(json.into());
        self
    }

    /// Set emulator host.
    pub fn emulator_host(mut self, host: impl Into<String>) -> Self {
        self.config.emulator_host = Some(host.into());
        self
    }

    /// Enable a service.
    pub fn enable(mut self, service: impl Into<String>) -> Self {
        self.config.enabled_services.insert(service.into());
        self
    }

    /// Enable Cloud Storage.
    pub fn enable_storage(self) -> Self {
        self.enable("storage")
    }

    /// Enable Pub/Sub.
    pub fn enable_pubsub(self) -> Self {
        self.enable("pubsub")
    }

    /// Enable Spanner.
    pub fn enable_spanner(self) -> Self {
        self.enable("spanner")
    }

    /// Enable BigQuery.
    pub fn enable_bigquery(self) -> Self {
        self.enable("bigquery")
    }

    /// Enable Secret Manager.
    pub fn enable_secret_manager(self) -> Self {
        self.enable("secret-manager")
    }

    /// Enable Cloud Run.
    pub fn enable_cloud_run(self) -> Self {
        self.enable("cloud-run")
    }

    /// Enable Cloud Functions.
    pub fn enable_cloud_functions(self) -> Self {
        self.enable("cloud-functions")
    }

    /// Enable all data services.
    pub fn enable_data(self) -> Self {
        self.enable_storage().enable_spanner().enable_bigquery()
    }

    /// Add service-specific configuration.
    pub fn service_config(mut self, service: &str, config: serde_json::Value) -> Self {
        self.config
            .service_configs
            .insert(service.to_string(), config);
        self
    }

    /// Build the configuration.
    pub fn build(self) -> GcpConfig {
        self.config
    }
}
