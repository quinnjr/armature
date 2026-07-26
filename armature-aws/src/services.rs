//! AWS services container with dynamic loading.

#[cfg(any(
    feature = "s3",
    feature = "dynamodb",
    feature = "sqs",
    feature = "sns",
    feature = "ses",
    feature = "lambda",
    feature = "secrets-manager",
    feature = "ssm",
    feature = "cloudwatch",
    feature = "kinesis",
    feature = "kms",
    feature = "cognito"
))]
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::info;

use crate::{AwsConfig, AwsError, CredentialsSource, Result};

/// Double-checked lazy initializer for a service client behind a `RwLock`.
///
/// Takes the fast path under a read lock when the client already exists, and
/// otherwise initializes it under the write lock (re-checking so a racing
/// writer's client is reused). `$init` is evaluated at most once and only when
/// the slot is empty. Expands to the accessor's tail expression, returning a
/// clone of the client wrapped in `Ok`.
#[cfg(any(
    feature = "s3",
    feature = "dynamodb",
    feature = "sqs",
    feature = "sns",
    feature = "ses",
    feature = "lambda",
    feature = "secrets-manager",
    feature = "ssm",
    feature = "cloudwatch",
    feature = "kinesis",
    feature = "kms",
    feature = "cognito"
))]
macro_rules! get_or_init {
    ($lock:expr, $init:expr) => {{
        // Fast path: already initialized — only take a read lock.
        if let Some(client) = $lock.read().as_ref() {
            return Ok(client.clone());
        }

        // Slow path: initialize under the write lock (double-checked).
        let mut guard = $lock.write();
        if guard.is_none() {
            *guard = Some($init);
        }
        Ok(guard.as_ref().unwrap().clone())
    }};
}

/// Container for AWS service clients.
///
/// Services are loaded lazily based on configuration.
/// Only enabled services are initialized.
pub struct AwsServices {
    config: AwsConfig,
    sdk_config: aws_config::SdkConfig,

    #[cfg(feature = "s3")]
    s3: RwLock<Option<aws_sdk_s3::Client>>,

    #[cfg(feature = "dynamodb")]
    dynamodb: RwLock<Option<aws_sdk_dynamodb::Client>>,

    #[cfg(feature = "sqs")]
    sqs: RwLock<Option<aws_sdk_sqs::Client>>,

    #[cfg(feature = "sns")]
    sns: RwLock<Option<aws_sdk_sns::Client>>,

    #[cfg(feature = "ses")]
    ses: RwLock<Option<aws_sdk_sesv2::Client>>,

    #[cfg(feature = "lambda")]
    lambda: RwLock<Option<aws_sdk_lambda::Client>>,

    #[cfg(feature = "secrets-manager")]
    secrets_manager: RwLock<Option<aws_sdk_secretsmanager::Client>>,

    #[cfg(feature = "ssm")]
    ssm: RwLock<Option<aws_sdk_ssm::Client>>,

    #[cfg(feature = "cloudwatch")]
    cloudwatch: RwLock<Option<aws_sdk_cloudwatch::Client>>,

    #[cfg(feature = "kinesis")]
    kinesis: RwLock<Option<aws_sdk_kinesis::Client>>,

    #[cfg(feature = "kms")]
    kms: RwLock<Option<aws_sdk_kms::Client>>,

    #[cfg(feature = "cognito")]
    cognito: RwLock<Option<aws_sdk_cognito_idp::Client>>,
}

impl AwsServices {
    /// Create a new AWS services container.
    pub async fn new(config: AwsConfig) -> Result<Arc<Self>> {
        let sdk_config = Self::build_sdk_config(&config).await?;

        info!(
            region = ?sdk_config.region(),
            services = ?config.enabled_services,
            "AWS services initialized"
        );

        let services = Self {
            config,
            sdk_config,
            #[cfg(feature = "s3")]
            s3: RwLock::new(None),
            #[cfg(feature = "dynamodb")]
            dynamodb: RwLock::new(None),
            #[cfg(feature = "sqs")]
            sqs: RwLock::new(None),
            #[cfg(feature = "sns")]
            sns: RwLock::new(None),
            #[cfg(feature = "ses")]
            ses: RwLock::new(None),
            #[cfg(feature = "lambda")]
            lambda: RwLock::new(None),
            #[cfg(feature = "secrets-manager")]
            secrets_manager: RwLock::new(None),
            #[cfg(feature = "ssm")]
            ssm: RwLock::new(None),
            #[cfg(feature = "cloudwatch")]
            cloudwatch: RwLock::new(None),
            #[cfg(feature = "kinesis")]
            kinesis: RwLock::new(None),
            #[cfg(feature = "kms")]
            kms: RwLock::new(None),
            #[cfg(feature = "cognito")]
            cognito: RwLock::new(None),
        };

        // Pre-initialize enabled services
        let services = Arc::new(services);
        services.initialize_enabled_services().await?;

        Ok(services)
    }

    /// Build AWS SDK configuration.
    async fn build_sdk_config(config: &AwsConfig) -> Result<aws_config::SdkConfig> {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        // Set region
        if let Some(region) = &config.region {
            loader = loader.region(aws_config::Region::new(region.clone()));
        }

        // Set credentials.
        //
        // Each `CredentialsSource` variant selects a concrete provider from
        // `aws-config`, rather than silently delegating to the default chain:
        //
        // * `Environment` -> `EnvironmentVariableCredentialsProvider` (reads
        //   `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`).
        // * `Profile`     -> the named profile from `~/.aws/{config,credentials}`.
        // * `IamRole`     -> `ImdsCredentialsProvider` (EC2/ECS instance metadata).
        // * `WebIdentity` -> `WebIdentityTokenCredentialsProvider` (EKS/OIDC via
        //   `AWS_WEB_IDENTITY_TOKEN_FILE` + `AWS_ROLE_ARN`).
        // * `Explicit`    -> the supplied static credentials.
        // * `Auto`        -> the full default credential provider chain (which
        //   tries env, profile, web-identity, ECS, and IMDS in order).
        match &config.credentials {
            CredentialsSource::Environment => {
                loader = loader.credentials_provider(
                    aws_config::environment::EnvironmentVariableCredentialsProvider::new(),
                );
            }
            CredentialsSource::Profile(profile) => {
                loader = loader.profile_name(profile);
            }
            CredentialsSource::IamRole => {
                let provider = aws_config::imds::credentials::ImdsCredentialsProvider::builder()
                    .configure(&Self::provider_config(config))
                    .build();
                loader = loader.credentials_provider(provider);
            }
            CredentialsSource::WebIdentity => {
                let provider =
                    aws_config::web_identity_token::WebIdentityTokenCredentialsProvider::builder()
                        .configure(&Self::provider_config(config))
                        .build();
                loader = loader.credentials_provider(provider);
            }
            CredentialsSource::Explicit {
                access_key_id,
                secret_access_key,
                session_token,
            } => {
                let creds = aws_credential_types::Credentials::new(
                    access_key_id,
                    secret_access_key,
                    session_token.clone(),
                    None,
                    "explicit",
                );
                loader = loader.credentials_provider(creds);
            }
            CredentialsSource::Auto => {
                // Use the SDK default credential provider chain.
            }
        }

        // Set custom endpoint
        if let Some(endpoint) = &config.endpoint_url {
            loader = loader.endpoint_url(endpoint);
        }

        Ok(loader.load().await)
    }

    /// Build a `ProviderConfig` for the IMDS / web-identity credential
    /// providers so they resolve STS/metadata endpoints against the configured
    /// region instead of falling back to their own default region lookup.
    fn provider_config(config: &AwsConfig) -> aws_config::provider_config::ProviderConfig {
        let region = config
            .region
            .as_ref()
            .map(|r| aws_config::Region::new(r.clone()));
        aws_config::provider_config::ProviderConfig::without_region().with_region(region)
    }

    /// Initialize all enabled services.
    async fn initialize_enabled_services(&self) -> Result<()> {
        for service in &self.config.enabled_services {
            match service.as_str() {
                #[cfg(feature = "s3")]
                "s3" => {
                    self.s3()?;
                }
                #[cfg(feature = "dynamodb")]
                "dynamodb" => {
                    self.dynamodb()?;
                }
                #[cfg(feature = "sqs")]
                "sqs" => {
                    self.sqs()?;
                }
                #[cfg(feature = "sns")]
                "sns" => {
                    self.sns()?;
                }
                #[cfg(feature = "ses")]
                "ses" => {
                    self.ses()?;
                }
                #[cfg(feature = "lambda")]
                "lambda" => {
                    self.lambda()?;
                }
                #[cfg(feature = "secrets-manager")]
                "secrets-manager" => {
                    self.secrets_manager()?;
                }
                #[cfg(feature = "ssm")]
                "ssm" => {
                    self.ssm()?;
                }
                #[cfg(feature = "cloudwatch")]
                "cloudwatch" => {
                    self.cloudwatch()?;
                }
                #[cfg(feature = "kinesis")]
                "kinesis" => {
                    self.kinesis()?;
                }
                #[cfg(feature = "kms")]
                "kms" => {
                    self.kms()?;
                }
                #[cfg(feature = "cognito")]
                "cognito" => {
                    self.cognito()?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Get the configuration.
    pub fn config(&self) -> &AwsConfig {
        &self.config
    }

    /// Get the SDK configuration.
    pub fn sdk_config(&self) -> &aws_config::SdkConfig {
        &self.sdk_config
    }

    /// Get the configured region.
    pub fn region(&self) -> Option<&aws_config::Region> {
        self.sdk_config.region()
    }

    // Service accessors with lazy initialization

    /// Get the S3 client.
    #[cfg(feature = "s3")]
    pub fn s3(&self) -> Result<aws_sdk_s3::Client> {
        if !self.config.is_enabled("s3") {
            return Err(AwsError::not_configured("s3"));
        }

        get_or_init!(self.s3, {
            let mut config = aws_sdk_s3::config::Builder::from(&self.sdk_config);
            if self.config.endpoint_url.is_some() {
                config = config.force_path_style(true);
            }
            let client = aws_sdk_s3::Client::from_conf(config.build());
            info!("S3 client initialized");
            client
        })
    }

    #[cfg(not(feature = "s3"))]
    pub fn s3(&self) -> Result<()> {
        Err(AwsError::not_enabled("s3"))
    }

    /// Get the DynamoDB client.
    #[cfg(feature = "dynamodb")]
    pub fn dynamodb(&self) -> Result<aws_sdk_dynamodb::Client> {
        if !self.config.is_enabled("dynamodb") {
            return Err(AwsError::not_configured("dynamodb"));
        }

        get_or_init!(self.dynamodb, {
            let client = aws_sdk_dynamodb::Client::new(&self.sdk_config);
            info!("DynamoDB client initialized");
            client
        })
    }

    #[cfg(not(feature = "dynamodb"))]
    pub fn dynamodb(&self) -> Result<()> {
        Err(AwsError::not_enabled("dynamodb"))
    }

    /// Get the SQS client.
    #[cfg(feature = "sqs")]
    pub fn sqs(&self) -> Result<aws_sdk_sqs::Client> {
        if !self.config.is_enabled("sqs") {
            return Err(AwsError::not_configured("sqs"));
        }

        get_or_init!(self.sqs, {
            let client = aws_sdk_sqs::Client::new(&self.sdk_config);
            info!("SQS client initialized");
            client
        })
    }

    #[cfg(not(feature = "sqs"))]
    pub fn sqs(&self) -> Result<()> {
        Err(AwsError::not_enabled("sqs"))
    }

    /// Get the SNS client.
    #[cfg(feature = "sns")]
    pub fn sns(&self) -> Result<aws_sdk_sns::Client> {
        if !self.config.is_enabled("sns") {
            return Err(AwsError::not_configured("sns"));
        }

        get_or_init!(self.sns, {
            let client = aws_sdk_sns::Client::new(&self.sdk_config);
            info!("SNS client initialized");
            client
        })
    }

    #[cfg(not(feature = "sns"))]
    pub fn sns(&self) -> Result<()> {
        Err(AwsError::not_enabled("sns"))
    }

    /// Get the SES client.
    #[cfg(feature = "ses")]
    pub fn ses(&self) -> Result<aws_sdk_sesv2::Client> {
        if !self.config.is_enabled("ses") {
            return Err(AwsError::not_configured("ses"));
        }

        get_or_init!(self.ses, {
            let client = aws_sdk_sesv2::Client::new(&self.sdk_config);
            info!("SES client initialized");
            client
        })
    }

    #[cfg(not(feature = "ses"))]
    pub fn ses(&self) -> Result<()> {
        Err(AwsError::not_enabled("ses"))
    }

    /// Get the Lambda client.
    #[cfg(feature = "lambda")]
    pub fn lambda(&self) -> Result<aws_sdk_lambda::Client> {
        if !self.config.is_enabled("lambda") {
            return Err(AwsError::not_configured("lambda"));
        }

        get_or_init!(self.lambda, {
            let client = aws_sdk_lambda::Client::new(&self.sdk_config);
            info!("Lambda client initialized");
            client
        })
    }

    #[cfg(not(feature = "lambda"))]
    pub fn lambda(&self) -> Result<()> {
        Err(AwsError::not_enabled("lambda"))
    }

    /// Get the Secrets Manager client.
    #[cfg(feature = "secrets-manager")]
    pub fn secrets_manager(&self) -> Result<aws_sdk_secretsmanager::Client> {
        if !self.config.is_enabled("secrets-manager") {
            return Err(AwsError::not_configured("secrets-manager"));
        }

        get_or_init!(self.secrets_manager, {
            let client = aws_sdk_secretsmanager::Client::new(&self.sdk_config);
            info!("Secrets Manager client initialized");
            client
        })
    }

    #[cfg(not(feature = "secrets-manager"))]
    pub fn secrets_manager(&self) -> Result<()> {
        Err(AwsError::not_enabled("secrets-manager"))
    }

    /// Get the SSM client.
    #[cfg(feature = "ssm")]
    pub fn ssm(&self) -> Result<aws_sdk_ssm::Client> {
        if !self.config.is_enabled("ssm") {
            return Err(AwsError::not_configured("ssm"));
        }

        get_or_init!(self.ssm, {
            let client = aws_sdk_ssm::Client::new(&self.sdk_config);
            info!("SSM client initialized");
            client
        })
    }

    #[cfg(not(feature = "ssm"))]
    pub fn ssm(&self) -> Result<()> {
        Err(AwsError::not_enabled("ssm"))
    }

    /// Get the CloudWatch client.
    #[cfg(feature = "cloudwatch")]
    pub fn cloudwatch(&self) -> Result<aws_sdk_cloudwatch::Client> {
        if !self.config.is_enabled("cloudwatch") {
            return Err(AwsError::not_configured("cloudwatch"));
        }

        get_or_init!(self.cloudwatch, {
            let client = aws_sdk_cloudwatch::Client::new(&self.sdk_config);
            info!("CloudWatch client initialized");
            client
        })
    }

    #[cfg(not(feature = "cloudwatch"))]
    pub fn cloudwatch(&self) -> Result<()> {
        Err(AwsError::not_enabled("cloudwatch"))
    }

    /// Get the Kinesis client.
    #[cfg(feature = "kinesis")]
    pub fn kinesis(&self) -> Result<aws_sdk_kinesis::Client> {
        if !self.config.is_enabled("kinesis") {
            return Err(AwsError::not_configured("kinesis"));
        }

        get_or_init!(self.kinesis, {
            let client = aws_sdk_kinesis::Client::new(&self.sdk_config);
            info!("Kinesis client initialized");
            client
        })
    }

    #[cfg(not(feature = "kinesis"))]
    pub fn kinesis(&self) -> Result<()> {
        Err(AwsError::not_enabled("kinesis"))
    }

    /// Get the KMS client.
    #[cfg(feature = "kms")]
    pub fn kms(&self) -> Result<aws_sdk_kms::Client> {
        if !self.config.is_enabled("kms") {
            return Err(AwsError::not_configured("kms"));
        }

        get_or_init!(self.kms, {
            let client = aws_sdk_kms::Client::new(&self.sdk_config);
            info!("KMS client initialized");
            client
        })
    }

    #[cfg(not(feature = "kms"))]
    pub fn kms(&self) -> Result<()> {
        Err(AwsError::not_enabled("kms"))
    }

    /// Get the Cognito client.
    #[cfg(feature = "cognito")]
    pub fn cognito(&self) -> Result<aws_sdk_cognito_idp::Client> {
        if !self.config.is_enabled("cognito") {
            return Err(AwsError::not_configured("cognito"));
        }

        get_or_init!(self.cognito, {
            let client = aws_sdk_cognito_idp::Client::new(&self.sdk_config);
            info!("Cognito client initialized");
            client
        })
    }

    #[cfg(not(feature = "cognito"))]
    pub fn cognito(&self) -> Result<()> {
        Err(AwsError::not_enabled("cognito"))
    }
}
