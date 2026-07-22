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

    #[cfg(feature = "servicebus")]
    servicebus: RwLock<Option<crate::servicebus::ServiceBusClient>>,

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
            #[cfg(feature = "servicebus")]
            servicebus: RwLock::new(None),
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
                #[cfg(feature = "servicebus")]
                "servicebus" => {
                    self.init_servicebus()?;
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
                let endpoint = Url::parse(&format!("https://{account}.blob.core.windows.net/"))
                    .map_err(|e| AzureError::Config(e.to_string()))?;
                BlobServiceClient::new(endpoint, Some(self.credential.clone()), None)
                    .map_err(|e| AzureError::Service(e.to_string()))?
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
                let endpoint = Url::parse(&format!("https://{account}.queue.core.windows.net/"))
                    .map_err(|e| AzureError::Config(e.to_string()))?;
                QueueServiceClient::new(endpoint, Some(self.credential.clone()), None)
                    .map_err(|e| AzureError::Service(e.to_string()))?
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

    #[cfg(feature = "servicebus")]
    fn init_servicebus(&self) -> Result<()> {
        use crate::servicebus::{ServiceBusClient, ServiceBusServiceConfig};

        let mut client = self.servicebus.write();
        if client.is_some() {
            return Ok(());
        }

        // Service Bus (azure_messaging_servicebus 0.21) is SAS-based: it needs a
        // shared-access policy name + key, resolved from either a connection
        // string or the per-service `service_config("servicebus")` block.
        let sb = if let Some(conn) = &self.config.servicebus_connection_string {
            ServiceBusClient::from_connection_string(conn)?
        } else if let Some(cfg) = self
            .config
            .service_config::<ServiceBusServiceConfig>("servicebus")
        {
            let namespace = cfg
                .namespace
                .or_else(|| self.config.servicebus_namespace.clone())
                .ok_or_else(|| {
                    AzureError::Config(
                        "Service Bus namespace not specified (set servicebus_namespace or \
                         include it in service_config(\"servicebus\"))"
                            .to_string(),
                    )
                })?;
            ServiceBusClient::new(namespace, cfg.policy_name, cfg.shared_access_key)
        } else {
            return Err(AzureError::Config(
                "Service Bus requires SAS credentials: set servicebus_connection_string, or a \
                 service_config(\"servicebus\") with policy_name + shared_access_key. The \
                 azure_messaging_servicebus 0.21 SDK is SAS-based and has no token-credential \
                 path."
                    .to_string(),
            ));
        };

        let namespace = sb.namespace().to_string();
        *client = Some(sb);
        info!(namespace = %namespace, "Service Bus client initialized");
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

    /// Get a Cosmos DB [`DatabaseClient`](azure_data_cosmos::DatabaseClient) for
    /// the configured `cosmos_database`.
    #[cfg(feature = "cosmos")]
    pub fn cosmos_database(&self) -> Result<azure_data_cosmos::DatabaseClient> {
        let database = self.config.cosmos_database.as_ref().ok_or_else(|| {
            AzureError::Config(
                "Cosmos database not specified. Call cosmos_database(..) on the config builder."
                    .to_string(),
            )
        })?;
        Ok(self.cosmos()?.database_client(database))
    }

    #[cfg(not(feature = "cosmos"))]
    pub fn cosmos_database(&self) -> Result<()> {
        Err(AzureError::not_enabled("cosmos"))
    }

    /// Get the Service Bus client.
    #[cfg(feature = "servicebus")]
    pub fn servicebus(&self) -> Result<crate::servicebus::ServiceBusClient> {
        if !self.config.is_enabled("servicebus") {
            return Err(AzureError::not_configured("servicebus"));
        }

        self.servicebus
            .read()
            .clone()
            .ok_or_else(|| AzureError::Service("Service Bus client not initialized".to_string()))
    }

    #[cfg(not(feature = "servicebus"))]
    pub fn servicebus(&self) -> Result<()> {
        Err(AzureError::not_enabled("servicebus"))
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

#[cfg(all(test, feature = "servicebus"))]
mod servicebus_tests {
    use crate::{AzureConfig, AzureServices};
    use serde_json::json;

    #[tokio::test]
    async fn servicebus_initializes_from_connection_string() {
        let config = AzureConfig::builder()
            .servicebus_connection_string(
                "Endpoint=sb://mybus.servicebus.windows.net/;\
                 SharedAccessKeyName=policy;SharedAccessKey=a2V5",
            )
            .enable_servicebus()
            .build();

        let services = AzureServices::new(config).await.expect("init services");
        let sb = services.servicebus().expect("servicebus accessor");
        assert_eq!(sb.namespace(), "mybus");
        assert!(sb.queue("orders").is_ok());
        assert!(sb.topic("events").is_ok());
    }

    #[tokio::test]
    async fn servicebus_initializes_from_service_config() {
        // Proves service_configs is actually read by an initializer.
        let config = AzureConfig::builder()
            .servicebus_namespace("mybus")
            .service_config(
                "servicebus",
                json!({ "policy_name": "policy", "shared_access_key": "a2V5" }),
            )
            .enable_servicebus()
            .build();

        let services = AzureServices::new(config).await.expect("init services");
        let sb = services.servicebus().expect("servicebus accessor");
        assert_eq!(sb.namespace(), "mybus");
    }

    #[tokio::test]
    async fn servicebus_without_sas_credentials_errors() {
        let config = AzureConfig::builder().enable_servicebus().build();
        assert!(AzureServices::new(config).await.is_err());
    }
}

#[cfg(all(test, feature = "cosmos"))]
mod cosmos_tests {
    use crate::{AzureConfig, AzureServices};

    fn err_string(result: crate::Result<azure_data_cosmos::DatabaseClient>) -> String {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    #[tokio::test]
    async fn cosmos_database_requires_a_database_name() {
        // cosmos not enabled -> no client construction / no network.
        let config = AzureConfig::builder().build();
        let services = AzureServices::new(config).await.expect("init services");
        let err = err_string(services.cosmos_database());
        assert!(err.contains("database not specified"), "got: {err}");
    }

    #[tokio::test]
    async fn cosmos_database_reads_the_configured_name() {
        // Database IS set, so the accessor advances past the name check (proving
        // the field is read) and only then fails because cosmos is not enabled.
        let config = AzureConfig::builder().cosmos_database("mydb").build();
        let services = AzureServices::new(config).await.expect("init services");
        let err = err_string(services.cosmos_database());
        assert!(
            err.contains("not configured") || err.contains("not initialized"),
            "got: {err}"
        );
    }
}
