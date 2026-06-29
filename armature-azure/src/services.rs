//! Azure services container with dynamic loading.

#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use std::sync::Arc;
use tracing::info;

#[allow(unused_imports)]
use crate::{AzureConfig, AzureError, CredentialsSource, Result};

/// Container for Azure service clients.
///
/// Services are loaded lazily based on configuration.
/// Only enabled services are initialized.
pub struct AzureServices {
    config: AzureConfig,

    #[cfg(feature = "auth")]
    credential: Arc<dyn azure_core::credentials::TokenCredential>,

    #[cfg(feature = "blob")]
    blob_service: RwLock<Option<Arc<azure_storage_blob::BlobServiceClient>>>,

    #[cfg(feature = "queue")]
    queue_service: RwLock<Option<Arc<azure_storage_queue::QueueServiceClient>>>,

    #[cfg(feature = "cosmos")]
    cosmos: RwLock<Option<azure_data_cosmos::CosmosClient>>,

    #[cfg(feature = "keyvault")]
    keyvault: RwLock<Option<Arc<azure_security_keyvault_secrets::SecretClient>>>,
}

impl AzureServices {
    /// Create a new Azure services container.
    pub async fn new(config: AzureConfig) -> Result<Arc<Self>> {
        #[cfg(feature = "auth")]
        let credential = Self::build_credential(&config).await?;

        info!(
            storage_account = ?config.storage_account,
            services = ?config.enabled_services,
            "Azure services initialized"
        );

        let services = Self {
            config,
            #[cfg(feature = "auth")]
            credential,
            #[cfg(feature = "blob")]
            blob_service: RwLock::new(None),
            #[cfg(feature = "queue")]
            queue_service: RwLock::new(None),
            #[cfg(feature = "cosmos")]
            cosmos: RwLock::new(None),
            #[cfg(feature = "keyvault")]
            keyvault: RwLock::new(None),
        };

        let services = Arc::new(services);

        // Pre-initialize enabled services
        services.initialize_enabled_services().await?;

        Ok(services)
    }

    /// Build Azure credential.
    #[cfg(feature = "auth")]
    async fn build_credential(
        config: &AzureConfig,
    ) -> Result<Arc<dyn azure_core::credentials::TokenCredential>> {
        use azure_identity::*;

        // The azure_identity 1.0 line removed the old `DefaultAzureCredential`,
        // `EnvironmentCredential` and `ImdsManagedIdentityCredential` types and
        // reworked the remaining constructors to return `Result<Arc<Self>>`.
        // `DeveloperToolsCredential` is the closest replacement for the previous
        // default credential chain.
        let credential: Arc<dyn azure_core::credentials::TokenCredential> =
            match &config.credentials {
                CredentialsSource::DefaultCredential => DeveloperToolsCredential::new(None)
                    .map_err(|e| AzureError::Auth(e.to_string()))?,
                CredentialsSource::Environment => DeveloperToolsCredential::new(None)
                    .map_err(|e| AzureError::Auth(e.to_string()))?,
                CredentialsSource::ManagedIdentity => ManagedIdentityCredential::new(None)
                    .map_err(|e| AzureError::Auth(e.to_string()))?,
                CredentialsSource::AzureCli => {
                    AzureCliCredential::new(None).map_err(|e| AzureError::Auth(e.to_string()))?
                }
                CredentialsSource::ServicePrincipal {
                    tenant_id,
                    client_id,
                    client_secret,
                } => ClientSecretCredential::new(
                    tenant_id.as_str(),
                    client_id.clone(),
                    client_secret.clone().into(),
                    None,
                )
                .map_err(|e| AzureError::Auth(e.to_string()))?,
                CredentialsSource::ConnectionString(_)
                | CredentialsSource::StorageAccountKey { .. } => {
                    // For storage-specific auth, we'll handle it at the client level.
                    DeveloperToolsCredential::new(None)
                        .map_err(|e| AzureError::Auth(e.to_string()))?
                }
            };

        Ok(credential)
    }

    /// Initialize all enabled services.
    async fn initialize_enabled_services(&self) -> Result<()> {
        for service in &self.config.enabled_services {
            match service.as_str() {
                #[cfg(feature = "blob")]
                "blob" => {
                    self.init_blob()?;
                }
                #[cfg(feature = "queue")]
                "queue" => {
                    self.init_queue()?;
                }
                #[cfg(feature = "cosmos")]
                "cosmos" => {
                    self.init_cosmos().await?;
                }
                #[cfg(feature = "keyvault")]
                "keyvault" => {
                    self.init_keyvault()?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Get the configuration.
    pub fn config(&self) -> &AzureConfig {
        &self.config
    }

    // Service initializers

    #[cfg(feature = "blob")]
    fn init_blob(&self) -> Result<()> {
        use azure_core::http::Url;
        use azure_storage_blob::BlobServiceClient;

        let mut client = self.blob_service.write();
        if client.is_none() {
            let account = self
                .config
                .storage_account
                .as_ref()
                .ok_or(AzureError::StorageAccountNotSpecified)?;

            // The new SDK (azure_storage_blob 1.0) is endpoint-URL + AAD-credential
            // based. It builds clients from a service URL plus an optional
            // `Arc<dyn TokenCredential>`; the old connection-string and shared-key
            // (account-key) authentication modes are no longer supported.
            let blob_client = if self.config.use_emulator {
                // Azurite emulator: HTTP endpoint, unauthenticated pipeline.
                let endpoint = Url::parse(&format!("http://127.0.0.1:10000/{account}"))
                    .map_err(|e| AzureError::Config(e.to_string()))?;
                BlobServiceClient::new(endpoint, None, None)
                    .map_err(|e| AzureError::Service(e.to_string()))?
            } else {
                match &self.config.credentials {
                    CredentialsSource::ConnectionString(_) => {
                        return Err(AzureError::Config(
                            "azure_storage_blob 1.0 requires AAD (Entra ID) token \
                             credentials; connection-string authentication is no longer \
                             supported by the new Azure SDK. Use a token credential source \
                             (DefaultCredential, ManagedIdentity, AzureCli, ServicePrincipal)."
                                .to_string(),
                        ));
                    }
                    CredentialsSource::StorageAccountKey { .. } => {
                        return Err(AzureError::Config(
                            "azure_storage_blob 1.0 requires AAD (Entra ID) token \
                             credentials; shared-key (storage account key) authentication is \
                             no longer supported by the new Azure SDK. Use a token credential \
                             source instead."
                                .to_string(),
                        ));
                    }
                    _ => {
                        let endpoint =
                            Url::parse(&format!("https://{account}.blob.core.windows.net/"))
                                .map_err(|e| AzureError::Config(e.to_string()))?;
                        BlobServiceClient::new(endpoint, Some(self.credential.clone()), None)
                            .map_err(|e| AzureError::Service(e.to_string()))?
                    }
                }
            };

            *client = Some(Arc::new(blob_client));
            info!(account = %account, "Blob Storage client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "queue")]
    fn init_queue(&self) -> Result<()> {
        use azure_core::http::Url;
        use azure_storage_queue::QueueServiceClient;

        let mut client = self.queue_service.write();
        if client.is_none() {
            let account = self
                .config
                .storage_account
                .as_ref()
                .ok_or(AzureError::StorageAccountNotSpecified)?;

            // The new SDK (azure_storage_queue 1.0) is endpoint-URL + AAD-credential
            // based; the old connection-string and shared-key (account-key) auth modes
            // are no longer supported.
            let queue_client = if self.config.use_emulator {
                // Azurite emulator: HTTP endpoint, unauthenticated pipeline.
                let endpoint = Url::parse(&format!("http://127.0.0.1:10001/{account}"))
                    .map_err(|e| AzureError::Config(e.to_string()))?;
                QueueServiceClient::new(endpoint, None, None)
                    .map_err(|e| AzureError::Service(e.to_string()))?
            } else {
                match &self.config.credentials {
                    CredentialsSource::ConnectionString(_) => {
                        return Err(AzureError::Config(
                            "azure_storage_queue 1.0 requires AAD (Entra ID) token \
                             credentials; connection-string authentication is no longer \
                             supported by the new Azure SDK. Use a token credential source \
                             (DefaultCredential, ManagedIdentity, AzureCli, ServicePrincipal)."
                                .to_string(),
                        ));
                    }
                    CredentialsSource::StorageAccountKey { .. } => {
                        return Err(AzureError::Config(
                            "azure_storage_queue 1.0 requires AAD (Entra ID) token \
                             credentials; shared-key (storage account key) authentication is \
                             no longer supported by the new Azure SDK. Use a token credential \
                             source instead."
                                .to_string(),
                        ));
                    }
                    _ => {
                        let endpoint =
                            Url::parse(&format!("https://{account}.queue.core.windows.net/"))
                                .map_err(|e| AzureError::Config(e.to_string()))?;
                        QueueServiceClient::new(endpoint, Some(self.credential.clone()), None)
                            .map_err(|e| AzureError::Service(e.to_string()))?
                    }
                }
            };

            *client = Some(Arc::new(queue_client));
            info!(account = %account, "Queue Storage client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "cosmos")]
    async fn init_cosmos(&self) -> Result<()> {
        use azure_data_cosmos::{AccountEndpoint, AccountReference, CosmosClient, RoutingStrategy};

        // Fast path: already initialized. The lock guard is intentionally dropped
        // before the `.await` below so the non-Send `parking_lot` guard is never
        // held across a suspension point.
        if self.cosmos.read().is_some() {
            return Ok(());
        }

        let endpoint =
            self.config.cosmos_endpoint.as_ref().ok_or_else(|| {
                AzureError::Config("Cosmos DB endpoint not specified".to_string())
            })?;

        // azure_data_cosmos 0.36 replaced `CosmosClient::new(...)` with a builder
        // that takes an `AccountReference` (endpoint + credential) and an explicit
        // `RoutingStrategy`. An empty preferred-region list defers region selection
        // to the account's default, preserving the previous endpoint-only behavior.
        let account_endpoint: AccountEndpoint = endpoint
            .parse()
            .map_err(|e: azure_data_cosmos::CosmosError| AzureError::Config(e.to_string()))?;
        let account = AccountReference::with_credential(account_endpoint, self.credential.clone());

        let cosmos_client = CosmosClient::builder()
            .build(account, RoutingStrategy::PreferredRegions(Vec::new()))
            .await
            .map_err(|e| AzureError::Service(e.to_string()))?;

        let mut client = self.cosmos.write();
        if client.is_none() {
            *client = Some(cosmos_client);
            info!(endpoint = %endpoint, "Cosmos DB client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "keyvault")]
    fn init_keyvault(&self) -> Result<()> {
        use azure_security_keyvault_secrets::SecretClient;

        let mut client = self.keyvault.write();
        if client.is_none() {
            let vault_url = self
                .config
                .keyvault_url
                .as_ref()
                .ok_or_else(|| AzureError::Config("Key Vault URL not specified".to_string()))?;

            // Key Vault has always used AAD credentials; the new SDK
            // (azure_security_keyvault_secrets 1.0) takes the vault endpoint plus an
            // `Arc<dyn TokenCredential>` and an optional options argument.
            let kv_client = SecretClient::new(vault_url, self.credential.clone(), None)
                .map_err(|e| AzureError::Config(e.to_string()))?;

            *client = Some(Arc::new(kv_client));
            info!(vault = %vault_url, "Key Vault client initialized");
        }
        Ok(())
    }

    // Service accessors

    /// Get the Blob Service client.
    #[cfg(feature = "blob")]
    pub fn blob_service(&self) -> Result<Arc<azure_storage_blob::BlobServiceClient>> {
        if !self.config.is_enabled("blob") {
            return Err(AzureError::not_configured("blob"));
        }

        self.blob_service
            .read()
            .clone()
            .ok_or_else(|| AzureError::Service("Blob client not initialized".to_string()))
    }

    #[cfg(not(feature = "blob"))]
    pub fn blob_service(&self) -> Result<()> {
        Err(AzureError::not_enabled("blob"))
    }

    /// Get the Queue Service client.
    #[cfg(feature = "queue")]
    pub fn queue_service(&self) -> Result<Arc<azure_storage_queue::QueueServiceClient>> {
        if !self.config.is_enabled("queue") {
            return Err(AzureError::not_configured("queue"));
        }

        self.queue_service
            .read()
            .clone()
            .ok_or_else(|| AzureError::Service("Queue client not initialized".to_string()))
    }

    #[cfg(not(feature = "queue"))]
    pub fn queue_service(&self) -> Result<()> {
        Err(AzureError::not_enabled("queue"))
    }

    /// Get the Cosmos DB client.
    #[cfg(feature = "cosmos")]
    pub fn cosmos(&self) -> Result<azure_data_cosmos::CosmosClient> {
        if !self.config.is_enabled("cosmos") {
            return Err(AzureError::not_configured("cosmos"));
        }

        self.cosmos
            .read()
            .clone()
            .ok_or_else(|| AzureError::Service("Cosmos client not initialized".to_string()))
    }

    #[cfg(not(feature = "cosmos"))]
    pub fn cosmos(&self) -> Result<()> {
        Err(AzureError::not_enabled("cosmos"))
    }

    /// Get the Key Vault client.
    #[cfg(feature = "keyvault")]
    pub fn keyvault(&self) -> Result<Arc<azure_security_keyvault_secrets::SecretClient>> {
        if !self.config.is_enabled("keyvault") {
            return Err(AzureError::not_configured("keyvault"));
        }

        self.keyvault
            .read()
            .clone()
            .ok_or_else(|| AzureError::Service("Key Vault client not initialized".to_string()))
    }

    #[cfg(not(feature = "keyvault"))]
    pub fn keyvault(&self) -> Result<()> {
        Err(AzureError::not_enabled("keyvault"))
    }
}
