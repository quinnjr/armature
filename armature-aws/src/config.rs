//! AWS configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Credentials source for AWS authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum CredentialsSource {
    /// Use environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY).
    Environment,
    /// Use AWS profile from ~/.aws/credentials.
    Profile(String),
    /// Use IAM role (for EC2, ECS, Lambda).
    IamRole,
    /// Use explicit credentials.
    Explicit {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
    },
    /// Use web identity token (for EKS).
    WebIdentity,
    /// Auto-detect credentials (default AWS SDK behavior).
    #[default]
    Auto,
}

/// AWS service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsConfig {
    /// AWS region.
    pub region: Option<String>,
    /// Credentials source.
    #[serde(default)]
    pub credentials: CredentialsSource,
    /// Custom endpoint URL (for LocalStack, MinIO, etc.).
    pub endpoint_url: Option<String>,
    /// Enabled services.
    #[serde(default)]
    pub enabled_services: HashSet<String>,
    /// Service-specific configurations.
    #[serde(default)]
    pub service_configs: std::collections::HashMap<String, serde_json::Value>,
}

impl Default for AwsConfig {
    fn default() -> Self {
        Self {
            region: None,
            credentials: CredentialsSource::Auto,
            endpoint_url: None,
            enabled_services: HashSet::new(),
            service_configs: std::collections::HashMap::new(),
        }
    }
}

impl AwsConfig {
    /// Create a new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder.
    pub fn builder() -> AwsConfigBuilder {
        AwsConfigBuilder::new()
    }

    /// Load configuration from environment variables.
    ///
    /// The following variables are read:
    ///
    /// * `AWS_REGION` / `AWS_DEFAULT_REGION` — the region (the former wins).
    /// * `AWS_ENDPOINT_URL` — a custom endpoint (LocalStack, MinIO, ...).
    /// * Credential source, in precedence order (mirrors the AWS SDK default
    ///   credential chain):
    ///   * `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` — selects
    ///     [`CredentialsSource::Environment`].
    ///   * `AWS_WEB_IDENTITY_TOKEN_FILE` — selects
    ///     [`CredentialsSource::WebIdentity`] (EKS/OIDC).
    ///   * `AWS_PROFILE` (when non-empty) — selects
    ///     [`CredentialsSource::Profile`].
    ///   * otherwise the source is left as [`CredentialsSource::Auto`].
    /// * `ARMATURE_AWS_SERVICES` — a comma/whitespace-separated list of
    ///   services to enable (e.g. `"s3,dynamodb,sqs"`).
    ///
    /// Returns a builder so callers can override or add to any of the above
    /// (for example chaining `.enable_s3()`).
    pub fn from_env() -> AwsConfigBuilder {
        let mut builder = AwsConfigBuilder::new();

        if let Ok(region) = std::env::var("AWS_REGION") {
            builder = builder.region(region);
        } else if let Ok(region) = std::env::var("AWS_DEFAULT_REGION") {
            builder = builder.region(region);
        }

        if let Ok(endpoint) = std::env::var("AWS_ENDPOINT_URL") {
            builder = builder.endpoint_url(endpoint);
        }

        // Resolve the credential source from the environment, mirroring the AWS
        // SDK default credential chain: static env credentials first, then
        // web-identity, then a named profile (an empty `AWS_PROFILE` is treated
        // as unset).
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok()
            && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok()
        {
            builder = builder.credentials(CredentialsSource::Environment);
        } else if std::env::var("AWS_WEB_IDENTITY_TOKEN_FILE").is_ok() {
            builder = builder.credentials(CredentialsSource::WebIdentity);
        } else if let Ok(profile) = std::env::var("AWS_PROFILE")
            && !profile.is_empty()
        {
            builder = builder.credentials(CredentialsSource::Profile(profile));
        }

        // Enable services listed in `ARMATURE_AWS_SERVICES`.
        if let Ok(services) = std::env::var("ARMATURE_AWS_SERVICES") {
            for service in services.split([',', ' ', '\t']) {
                let service = service.trim();
                if !service.is_empty() {
                    builder = builder.enable(service);
                }
            }
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

/// Builder for AWS configuration.
#[derive(Default)]
pub struct AwsConfigBuilder {
    config: AwsConfig,
}

impl AwsConfigBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the AWS region.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.config.region = Some(region.into());
        self
    }

    /// Set the credentials source.
    pub fn credentials(mut self, credentials: CredentialsSource) -> Self {
        self.config.credentials = credentials;
        self
    }

    /// Use explicit credentials.
    pub fn explicit_credentials(
        mut self,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Self {
        self.config.credentials = CredentialsSource::Explicit {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
        };
        self
    }

    /// Use a named profile.
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.config.credentials = CredentialsSource::Profile(profile.into());
        self
    }

    /// Set a custom endpoint URL (for LocalStack, MinIO, etc.).
    pub fn endpoint_url(mut self, url: impl Into<String>) -> Self {
        self.config.endpoint_url = Some(url.into());
        self
    }

    /// Configure for LocalStack.
    pub fn localstack(self) -> Self {
        self.endpoint_url("http://localhost:4566")
    }

    /// Enable a service.
    pub fn enable(mut self, service: impl Into<String>) -> Self {
        self.config.enabled_services.insert(service.into());
        self
    }

    /// Enable S3.
    pub fn enable_s3(self) -> Self {
        self.enable("s3")
    }

    /// Enable DynamoDB.
    pub fn enable_dynamodb(self) -> Self {
        self.enable("dynamodb")
    }

    /// Enable SQS.
    pub fn enable_sqs(self) -> Self {
        self.enable("sqs")
    }

    /// Enable SNS.
    pub fn enable_sns(self) -> Self {
        self.enable("sns")
    }

    /// Enable SES.
    pub fn enable_ses(self) -> Self {
        self.enable("ses")
    }

    /// Enable Lambda.
    pub fn enable_lambda(self) -> Self {
        self.enable("lambda")
    }

    /// Enable Secrets Manager.
    pub fn enable_secrets_manager(self) -> Self {
        self.enable("secrets-manager")
    }

    /// Enable SSM Parameter Store.
    pub fn enable_ssm(self) -> Self {
        self.enable("ssm")
    }

    /// Enable CloudWatch.
    pub fn enable_cloudwatch(self) -> Self {
        self.enable("cloudwatch")
    }

    /// Enable Kinesis.
    pub fn enable_kinesis(self) -> Self {
        self.enable("kinesis")
    }

    /// Enable KMS.
    pub fn enable_kms(self) -> Self {
        self.enable("kms")
    }

    /// Enable Cognito.
    pub fn enable_cognito(self) -> Self {
        self.enable("cognito")
    }

    /// Enable all storage services.
    pub fn enable_storage(self) -> Self {
        self.enable_s3().enable_dynamodb()
    }

    /// Enable all messaging services.
    pub fn enable_messaging(self) -> Self {
        self.enable_sqs().enable_sns().enable_kinesis()
    }

    /// Add service-specific configuration.
    pub fn service_config(mut self, service: &str, config: serde_json::Value) -> Self {
        self.config
            .service_configs
            .insert(service.to_string(), config);
        self
    }

    /// Build the configuration.
    pub fn build(self) -> AwsConfig {
        self.config
    }
}
