//! GCP services container with dynamic loading.

#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use std::sync::Arc;
use tracing::info;

use crate::{GcpConfig, GcpError, Result};

/// Generate the eager initializer, the typed accessor, and the feature-absent
/// fallback for a REST-backed service. These three follow an identical shape;
/// only the feature, service key, field, client type, method names, and log
/// label differ, so a declarative macro removes the near-verbatim triplication.
macro_rules! rest_service {
    (
        feature = $feature:literal,
        service = $service:literal,
        field = $field:ident,
        client = $client:ty,
        init = $init:ident,
        accessor = $accessor:ident,
        label = $label:literal $(,)?
    ) => {
        #[cfg(feature = $feature)]
        async fn $init(&self) -> Result<()> {
            if self.$field.read().is_some() {
                return Ok(());
            }
            let built = <$client>::new(&self.config).await?;
            let mut client = self.$field.write();
            if client.is_none() {
                *client = Some(built);
                info!("{} client initialized", $label);
            }
            Ok(())
        }

        #[cfg(feature = $feature)]
        #[doc = concat!("Get the ", $label, " client.")]
        pub fn $accessor(&self) -> Result<$client> {
            if !self.config.is_enabled($service) {
                return Err(GcpError::not_configured($service));
            }

            self.$field.read().clone().ok_or_else(|| {
                GcpError::Service(concat!($label, " client not initialized").to_string())
            })
        }

        #[cfg(not(feature = $feature))]
        pub fn $accessor(&self) -> Result<()> {
            Err(GcpError::not_enabled($service))
        }
    };
}

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

    // REST-backed services: initializer + accessor + feature-absent fallback,
    // generated from a single shared shape (see `rest_service!`).
    rest_service! {
        feature = "secret-manager",
        service = "secret-manager",
        field = secret_manager,
        client = crate::rest::SecretManagerClient,
        init = init_secret_manager,
        accessor = secret_manager,
        label = "Secret Manager",
    }

    rest_service! {
        feature = "cloud-run",
        service = "cloud-run",
        field = cloud_run,
        client = crate::rest::CloudRunClient,
        init = init_cloud_run,
        accessor = cloud_run,
        label = "Cloud Run",
    }

    rest_service! {
        feature = "cloud-functions",
        service = "cloud-functions",
        field = cloud_functions,
        client = crate::rest::CloudFunctionsClient,
        init = init_cloud_functions,
        accessor = cloud_functions,
        label = "Cloud Functions",
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

    // The Secret Manager / Cloud Run / Cloud Functions accessors (and their
    // feature-absent fallbacks) are generated by the `rest_service!` invocations
    // above.
}
