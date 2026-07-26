//! Azure configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Credentials source for Azure authentication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialsSource {
    /// Use Default Azure Credential (environment, managed identity, CLI, etc.).
    #[default]
    DefaultCredential,
    /// Resolve via the developer-tools credential chain.
    ///
    /// Despite the name, this variant does **not** read `AZURE_CLIENT_ID` /
    /// `AZURE_CLIENT_SECRET` / `AZURE_TENANT_ID` from the environment:
    /// `azure_identity` 1.0 removed the standalone `EnvironmentCredential`, so it
    /// currently resolves **identically** to [`CredentialsSource::DefaultCredential`]
    /// (Azure CLI / Azure Developer CLI) and shares its match arm. It is retained
    /// as a distinct name for callers that configure it explicitly; use
    /// [`CredentialsSource::ServicePrincipal`] to supply client-id/secret directly.
    Environment,
    /// Use managed identity.
    ManagedIdentity,
    /// Use Azure CLI credential.
    AzureCli,
    /// Use service principal with client secret.
    ServicePrincipal {
        tenant_id: String,
        client_id: String,
        client_secret: String,
    },
}

/// Azure service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureConfig {
    /// Storage account name.
    pub storage_account: Option<String>,
    /// Cosmos DB endpoint.
    pub cosmos_endpoint: Option<String>,
    /// Cosmos DB database.
    pub cosmos_database: Option<String>,
    /// Service Bus namespace.
    pub servicebus_namespace: Option<String>,
    /// Service Bus connection string (SAS). Carries the namespace + shared
    /// access policy the SAS-based Service Bus SDK requires.
    #[serde(default)]
    pub servicebus_connection_string: Option<String>,
    /// Key Vault URL.
    pub keyvault_url: Option<String>,
    /// Credentials source.
    #[serde(default)]
    pub credentials: CredentialsSource,
    /// Enabled services.
    #[serde(default)]
    pub enabled_services: HashSet<String>,
    /// Service-specific configurations.
    #[serde(default)]
    pub service_configs: std::collections::HashMap<String, serde_json::Value>,
    /// Use Azurite emulator (for local development).
    pub use_emulator: bool,
}

impl Default for AzureConfig {
    fn default() -> Self {
        Self {
            storage_account: None,
            cosmos_endpoint: None,
            cosmos_database: None,
            servicebus_namespace: None,
            servicebus_connection_string: None,
            keyvault_url: None,
            credentials: CredentialsSource::DefaultCredential,
            enabled_services: HashSet::new(),
            service_configs: std::collections::HashMap::new(),
            use_emulator: false,
        }
    }
}

impl AzureConfig {
    /// Create a new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder.
    pub fn builder() -> AzureConfigBuilder {
        AzureConfigBuilder::new()
    }

    /// Load configuration from environment variables.
    pub fn from_env() -> AzureConfigBuilder {
        let mut builder = AzureConfigBuilder::new();

        if let Ok(account) = std::env::var("AZURE_STORAGE_ACCOUNT") {
            builder = builder.storage_account(account);
        }

        if let Ok(endpoint) = std::env::var("AZURE_COSMOS_ENDPOINT") {
            builder = builder.cosmos_endpoint(endpoint);
        }

        if let Ok(database) = std::env::var("AZURE_COSMOS_DATABASE") {
            builder = builder.cosmos_database(database);
        }

        if let Ok(namespace) = std::env::var("AZURE_SERVICEBUS_NAMESPACE") {
            builder = builder.servicebus_namespace(namespace);
        }

        if let Ok(conn_str) = std::env::var("AZURE_SERVICEBUS_CONNECTION_STRING") {
            builder = builder.servicebus_connection_string(conn_str);
        }

        if let Ok(url) = std::env::var("AZURE_KEYVAULT_URL") {
            builder = builder.keyvault_url(url);
        }

        // Check for emulator
        if std::env::var("AZURITE_ACCOUNTS").is_ok() || std::env::var("USE_AZURITE").is_ok() {
            builder = builder.use_emulator();
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

/// Builder for Azure configuration.
#[derive(Debug, Clone, Default)]
pub struct AzureConfigBuilder {
    config: AzureConfig,
}

impl AzureConfigBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the storage account name.
    pub fn storage_account(mut self, account: impl Into<String>) -> Self {
        self.config.storage_account = Some(account.into());
        self
    }

    /// Set the Cosmos DB endpoint.
    pub fn cosmos_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.cosmos_endpoint = Some(endpoint.into());
        self
    }

    /// Set the Cosmos DB database.
    pub fn cosmos_database(mut self, database: impl Into<String>) -> Self {
        self.config.cosmos_database = Some(database.into());
        self
    }

    /// Set the Service Bus namespace.
    pub fn servicebus_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.config.servicebus_namespace = Some(namespace.into());
        self
    }

    /// Set the Service Bus connection string (SAS).
    pub fn servicebus_connection_string(mut self, conn_str: impl Into<String>) -> Self {
        self.config.servicebus_connection_string = Some(conn_str.into());
        self
    }

    /// Set the Key Vault URL.
    pub fn keyvault_url(mut self, url: impl Into<String>) -> Self {
        self.config.keyvault_url = Some(url.into());
        self
    }

    /// Set the credentials source.
    pub fn credentials(mut self, credentials: CredentialsSource) -> Self {
        self.config.credentials = credentials;
        self
    }

    /// Use service principal.
    pub fn service_principal(
        mut self,
        tenant_id: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.config.credentials = CredentialsSource::ServicePrincipal {
            tenant_id: tenant_id.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        };
        self
    }

    /// Use Azurite emulator.
    pub fn use_emulator(mut self) -> Self {
        self.config.use_emulator = true;
        self
    }

    /// Enable a service.
    pub fn enable(mut self, service: impl Into<String>) -> Self {
        self.config.enabled_services.insert(service.into());
        self
    }

    /// Enable Blob Storage.
    pub fn enable_blob(self) -> Self {
        self.enable("blob")
    }

    /// Enable Queue Storage.
    pub fn enable_queue(self) -> Self {
        self.enable("queue")
    }

    /// Enable Cosmos DB.
    pub fn enable_cosmos(self) -> Self {
        self.enable("cosmos")
    }

    /// Enable Service Bus.
    pub fn enable_servicebus(self) -> Self {
        self.enable("servicebus")
    }

    /// Enable Key Vault.
    pub fn enable_keyvault(self) -> Self {
        self.enable("keyvault")
    }

    /// Enable all storage services.
    pub fn enable_storage(self) -> Self {
        self.enable_blob().enable_queue()
    }

    /// Add service-specific configuration.
    pub fn service_config(mut self, service: &str, config: serde_json::Value) -> Self {
        self.config
            .service_configs
            .insert(service.to_string(), config);
        self
    }

    /// Build the configuration.
    pub fn build(self) -> AzureConfig {
        self.config
    }
}
