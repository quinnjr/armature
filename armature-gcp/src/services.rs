//! GCP services container with dynamic loading.

#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use std::sync::Arc;
use tracing::info;

use crate::{GcpConfig, GcpError, Result};

/// Container for GCP service clients.
///
/// Which services are compiled in is controlled by Cargo feature flags; which
/// of those are actually constructed is controlled by the [`GcpConfig`]
/// `enable_*` flags. Clients are initialized **eagerly**: [`GcpServices::new`]
/// pre-initializes every enabled service's client before it returns, so the
/// accessor methods only ever hand back an already-built client (or a clear
/// error). Once created, a client is cached and shared on subsequent access.
pub struct GcpServices {
    config: GcpConfig,

    #[cfg(feature = "storage")]
    storage: RwLock<Option<google_cloud_storage::client::Storage>>,

    // The new Pub/Sub SDK splits the old unified client into purpose-specific
    // clients (TopicAdmin, SubscriptionAdmin, Publisher, Subscriber). We keep the
    // topic-administration client as the general-purpose handle.
    #[cfg(feature = "pubsub")]
    pubsub: RwLock<Option<google_cloud_pubsub::client::TopicAdmin>>,

    #[cfg(feature = "spanner")]
    spanner: RwLock<Option<google_cloud_spanner::client::Client>>,

    #[cfg(feature = "bigquery")]
    bigquery: RwLock<Option<gcloud_bigquery::client::Client>>,

    #[cfg(feature = "secret-manager")]
    secret_manager: RwLock<Option<crate::rest::SecretManagerClient>>,

    #[cfg(feature = "cloud-run")]
    cloud_run: RwLock<Option<crate::rest::CloudRunClient>>,

    #[cfg(feature = "cloud-functions")]
    cloud_functions: RwLock<Option<crate::rest::CloudFunctionsClient>>,
}

impl GcpServices {
    /// Create a new GCP services container.
    ///
    /// Every enabled service is initialized eagerly before this returns.
    pub async fn new(config: GcpConfig) -> Result<Arc<Self>> {
        info!(
            project = ?config.project_id,
            services = ?config.enabled_services,
            "GCP services initialized"
        );

        let services = Self {
            config,
            #[cfg(feature = "storage")]
            storage: RwLock::new(None),
            #[cfg(feature = "pubsub")]
            pubsub: RwLock::new(None),
            #[cfg(feature = "spanner")]
            spanner: RwLock::new(None),
            #[cfg(feature = "bigquery")]
            bigquery: RwLock::new(None),
            #[cfg(feature = "secret-manager")]
            secret_manager: RwLock::new(None),
            #[cfg(feature = "cloud-run")]
            cloud_run: RwLock::new(None),
            #[cfg(feature = "cloud-functions")]
            cloud_functions: RwLock::new(None),
        };

        let services = Arc::new(services);

        // Pre-initialize enabled services.
        services.initialize_enabled_services().await?;

        Ok(services)
    }

    /// Initialize all enabled services.
    async fn initialize_enabled_services(&self) -> Result<()> {
        for service in &self.config.enabled_services {
            match service.as_str() {
                #[cfg(feature = "storage")]
                "storage" => {
                    self.init_storage().await?;
                }
                #[cfg(feature = "pubsub")]
                "pubsub" => {
                    self.init_pubsub().await?;
                }
                #[cfg(feature = "spanner")]
                "spanner" => {
                    self.init_spanner().await?;
                }
                #[cfg(feature = "bigquery")]
                "bigquery" => {
                    self.init_bigquery().await?;
                }
                #[cfg(feature = "secret-manager")]
                "secret-manager" => {
                    self.init_secret_manager().await?;
                }
                #[cfg(feature = "cloud-run")]
                "cloud-run" => {
                    self.init_cloud_run().await?;
                }
                #[cfg(feature = "cloud-functions")]
                "cloud-functions" => {
                    self.init_cloud_functions().await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Get the configuration.
    pub fn config(&self) -> &GcpConfig {
        &self.config
    }

    /// Get the project ID.
    pub fn project_id(&self) -> Option<&str> {
        self.config.project_id.as_deref()
    }

    // Service initializers

    #[cfg(feature = "storage")]
    async fn init_storage(&self) -> Result<()> {
        use google_cloud_storage::client::Storage;

        if self.storage.read().is_some() {
            return Ok(());
        }

        // Thread the configured credentials and endpoint override (emulator or
        // per-service `service_configs.endpoint`) into the client builder.
        // Build the client without holding the lock across the await.
        let mut builder = Storage::builder();
        if let Some(cred) = crate::credentials::build_gcloud_credentials(&self.config.credentials)?
        {
            builder = builder.with_credentials(cred);
        }
        if let Some(endpoint) = crate::credentials::resolve_endpoint(&self.config, "storage") {
            builder = builder.with_endpoint(endpoint);
        }
        let storage_client = builder
            .build()
            .await
            .map_err(|e| GcpError::Auth(e.to_string()))?;

        let mut client = self.storage.write();
        if client.is_none() {
            *client = Some(storage_client);
            info!("Cloud Storage client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "pubsub")]
    async fn init_pubsub(&self) -> Result<()> {
        use google_cloud_pubsub::client::TopicAdmin;

        if self.pubsub.read().is_some() {
            return Ok(());
        }

        // The Pub/Sub SDK auto-detects the project from the credentials/ADC and
        // takes the project per-request via resource paths, so `project_id` is
        // not a required builder input here. Credentials and an optional
        // endpoint override (emulator / service_configs) are threaded in.
        let mut builder = TopicAdmin::builder();
        if let Some(cred) = crate::credentials::build_gcloud_credentials(&self.config.credentials)?
        {
            builder = builder.with_credentials(cred);
        }
        if let Some(endpoint) = crate::credentials::resolve_endpoint(&self.config, "pubsub") {
            builder = builder.with_endpoint(endpoint);
        }
        let pubsub_client = builder
            .build()
            .await
            .map_err(|e| GcpError::Auth(e.to_string()))?;

        let mut client = self.pubsub.write();
        if client.is_none() {
            *client = Some(pubsub_client);
            info!(project = ?self.config.project_id, "Pub/Sub client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "spanner")]
    async fn init_spanner(&self) -> Result<()> {
        use google_cloud_spanner::client::{Client, ClientConfig};

        if self.spanner.read().is_some() {
            return Ok(());
        }

        let project_id = self
            .config
            .project_id
            .as_ref()
            .ok_or(GcpError::ProjectNotSpecified)?;

        // Build the client without holding the lock across the await.
        let config = ClientConfig::default()
            .with_auth()
            .await
            .map_err(|e| GcpError::Auth(e.to_string()))?;

        let spanner_client = Client::new(project_id, config)
            .await
            .map_err(|e| GcpError::Service(e.to_string()))?;

        let mut client = self.spanner.write();
        if client.is_none() {
            *client = Some(spanner_client);
            info!(project = %project_id, "Spanner client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "bigquery")]
    async fn init_bigquery(&self) -> Result<()> {
        use gcloud_bigquery::client::{Client, ClientConfig};

        if self.bigquery.read().is_some() {
            return Ok(());
        }

        let project_id = self
            .config
            .project_id
            .as_ref()
            .ok_or(GcpError::ProjectNotSpecified)?;

        // Build the client without holding the lock across the await.
        let (config, _) = ClientConfig::new_with_auth()
            .await
            .map_err(|e| GcpError::Auth(e.to_string()))?;

        let bigquery_client = Client::new(config)
            .await
            .map_err(|e| GcpError::Service(e.to_string()))?;

        let mut client = self.bigquery.write();
        if client.is_none() {
            *client = Some(bigquery_client);
            info!(project = %project_id, "BigQuery client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "secret-manager")]
    async fn init_secret_manager(&self) -> Result<()> {
        if self.secret_manager.read().is_some() {
            return Ok(());
        }
        let built = crate::rest::SecretManagerClient::new(&self.config).await?;
        let mut client = self.secret_manager.write();
        if client.is_none() {
            *client = Some(built);
            info!("Secret Manager client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "cloud-run")]
    async fn init_cloud_run(&self) -> Result<()> {
        if self.cloud_run.read().is_some() {
            return Ok(());
        }
        let built = crate::rest::CloudRunClient::new(&self.config).await?;
        let mut client = self.cloud_run.write();
        if client.is_none() {
            *client = Some(built);
            info!("Cloud Run client initialized");
        }
        Ok(())
    }

    #[cfg(feature = "cloud-functions")]
    async fn init_cloud_functions(&self) -> Result<()> {
        if self.cloud_functions.read().is_some() {
            return Ok(());
        }
        let built = crate::rest::CloudFunctionsClient::new(&self.config).await?;
        let mut client = self.cloud_functions.write();
        if client.is_none() {
            *client = Some(built);
            info!("Cloud Functions client initialized");
        }
        Ok(())
    }

    // Service accessors

    /// Get the Cloud Storage client.
    #[cfg(feature = "storage")]
    pub fn storage(&self) -> Result<google_cloud_storage::client::Storage> {
        if !self.config.is_enabled("storage") {
            return Err(GcpError::not_configured("storage"));
        }

        self.storage
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("Storage client not initialized".to_string()))
    }

    #[cfg(not(feature = "storage"))]
    pub fn storage(&self) -> Result<()> {
        Err(GcpError::not_enabled("storage"))
    }

    /// Get the Pub/Sub client.
    #[cfg(feature = "pubsub")]
    pub fn pubsub(&self) -> Result<google_cloud_pubsub::client::TopicAdmin> {
        if !self.config.is_enabled("pubsub") {
            return Err(GcpError::not_configured("pubsub"));
        }

        self.pubsub
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("Pub/Sub client not initialized".to_string()))
    }

    #[cfg(not(feature = "pubsub"))]
    pub fn pubsub(&self) -> Result<()> {
        Err(GcpError::not_enabled("pubsub"))
    }

    /// Get the Spanner client.
    #[cfg(feature = "spanner")]
    pub fn spanner(&self) -> Result<google_cloud_spanner::client::Client> {
        if !self.config.is_enabled("spanner") {
            return Err(GcpError::not_configured("spanner"));
        }

        self.spanner
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("Spanner client not initialized".to_string()))
    }

    #[cfg(not(feature = "spanner"))]
    pub fn spanner(&self) -> Result<()> {
        Err(GcpError::not_enabled("spanner"))
    }

    /// Get the BigQuery client.
    #[cfg(feature = "bigquery")]
    pub fn bigquery(&self) -> Result<gcloud_bigquery::client::Client> {
        if !self.config.is_enabled("bigquery") {
            return Err(GcpError::not_configured("bigquery"));
        }

        self.bigquery
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("BigQuery client not initialized".to_string()))
    }

    #[cfg(not(feature = "bigquery"))]
    pub fn bigquery(&self) -> Result<()> {
        Err(GcpError::not_enabled("bigquery"))
    }

    /// Get the Secret Manager client.
    #[cfg(feature = "secret-manager")]
    pub fn secret_manager(&self) -> Result<crate::rest::SecretManagerClient> {
        if !self.config.is_enabled("secret-manager") {
            return Err(GcpError::not_configured("secret-manager"));
        }

        self.secret_manager
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("Secret Manager client not initialized".to_string()))
    }

    #[cfg(not(feature = "secret-manager"))]
    pub fn secret_manager(&self) -> Result<()> {
        Err(GcpError::not_enabled("secret-manager"))
    }

    /// Get the Cloud Run client.
    #[cfg(feature = "cloud-run")]
    pub fn cloud_run(&self) -> Result<crate::rest::CloudRunClient> {
        if !self.config.is_enabled("cloud-run") {
            return Err(GcpError::not_configured("cloud-run"));
        }

        self.cloud_run
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("Cloud Run client not initialized".to_string()))
    }

    #[cfg(not(feature = "cloud-run"))]
    pub fn cloud_run(&self) -> Result<()> {
        Err(GcpError::not_enabled("cloud-run"))
    }

    /// Get the Cloud Functions client.
    #[cfg(feature = "cloud-functions")]
    pub fn cloud_functions(&self) -> Result<crate::rest::CloudFunctionsClient> {
        if !self.config.is_enabled("cloud-functions") {
            return Err(GcpError::not_configured("cloud-functions"));
        }

        self.cloud_functions
            .read()
            .clone()
            .ok_or_else(|| GcpError::Service("Cloud Functions client not initialized".to_string()))
    }

    #[cfg(not(feature = "cloud-functions"))]
    pub fn cloud_functions(&self) -> Result<()> {
        Err(GcpError::not_enabled("cloud-functions"))
    }
}
