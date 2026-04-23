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
        services.initialize_enabled_services().await;

        Ok(services)
    }

    /// Build AWS SDK configuration.
    async fn build_sdk_config(config: &AwsConfig) -> Result<aws_config::SdkConfig> {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        // Set region
        if let Some(region) = &config.region {
            loader = loader.region(aws_config::Region::new(region.clone()));
        }

        // Set credentials
        match &config.credentials {
            CredentialsSource::Profile(profile) => {
                loader = loader.profile_name(profile);
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
            _ => {
                // Use default credential chain
            }
        }

        // Set custom endpoint
        if let Some(endpoint) = &config.endpoint_url {
            loader = loader.endpoint_url(endpoint);
        }

        Ok(loader.load().await)
    }

    /// Initialize all enabled services.
    async fn initialize_enabled_services(&self) {
        for service in &self.config.enabled_services {
            match service.as_str() {
                #[cfg(feature = "s3")]
                "s3" => {
                    let _ = self.s3();
                }
                #[cfg(feature = "dynamodb")]
                "dynamodb" => {
                    let _ = self.dynamodb();
                }
                #[cfg(feature = "sqs")]
                "sqs" => {
                    let _ = self.sqs();
                }
                #[cfg(feature = "sns")]
                "sns" => {
                    let _ = self.sns();
                }
                #[cfg(feature = "ses")]
                "ses" => {
                    let _ = self.ses();
                }
                #[cfg(feature = "lambda")]
                "lambda" => {
                    let _ = self.lambda();
                }
                #[cfg(feature = "secrets-manager")]
                "secrets-manager" => {
                    let _ = self.secrets_manager();
                }
                #[cfg(feature = "ssm")]
                "ssm" => {
                    let _ = self.ssm();
                }
                #[cfg(feature = "cloudwatch")]
                "cloudwatch" => {
                    let _ = self.cloudwatch();
                }
                #[cfg(feature = "kinesis")]
                "kinesis" => {
                    let _ = self.kinesis();
                }
                #[cfg(feature = "kms")]
                "kms" => {
                    let _ = self.kms();
                }
                #[cfg(feature = "cognito")]
                "cognito" => {
                    let _ = self.cognito();
                }
                _ => {}
            }
        }
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

        let mut client = self.s3.write();
        if client.is_none() {
            let mut config = aws_sdk_s3::config::Builder::from(&self.sdk_config);
            if self.config.endpoint_url.is_some() {
                config = config.force_path_style(true);
            }
            *client = Some(aws_sdk_s3::Client::from_conf(config.build()));
            info!("S3 client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.dynamodb.write();
        if client.is_none() {
            *client = Some(aws_sdk_dynamodb::Client::new(&self.sdk_config));
            info!("DynamoDB client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.sqs.write();
        if client.is_none() {
            *client = Some(aws_sdk_sqs::Client::new(&self.sdk_config));
            info!("SQS client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.sns.write();
        if client.is_none() {
            *client = Some(aws_sdk_sns::Client::new(&self.sdk_config));
            info!("SNS client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.ses.write();
        if client.is_none() {
            *client = Some(aws_sdk_sesv2::Client::new(&self.sdk_config));
            info!("SES client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.lambda.write();
        if client.is_none() {
            *client = Some(aws_sdk_lambda::Client::new(&self.sdk_config));
            info!("Lambda client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.secrets_manager.write();
        if client.is_none() {
            *client = Some(aws_sdk_secretsmanager::Client::new(&self.sdk_config));
            info!("Secrets Manager client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.ssm.write();
        if client.is_none() {
            *client = Some(aws_sdk_ssm::Client::new(&self.sdk_config));
            info!("SSM client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.cloudwatch.write();
        if client.is_none() {
            *client = Some(aws_sdk_cloudwatch::Client::new(&self.sdk_config));
            info!("CloudWatch client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.kinesis.write();
        if client.is_none() {
            *client = Some(aws_sdk_kinesis::Client::new(&self.sdk_config));
            info!("Kinesis client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.kms.write();
        if client.is_none() {
            *client = Some(aws_sdk_kms::Client::new(&self.sdk_config));
            info!("KMS client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
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

        let mut client = self.cognito.write();
        if client.is_none() {
            *client = Some(aws_sdk_cognito_idp::Client::new(&self.sdk_config));
            info!("Cognito client initialized");
        }
        Ok(client.as_ref().unwrap().clone())
    }

    #[cfg(not(feature = "cognito"))]
    pub fn cognito(&self) -> Result<()> {
        Err(AwsError::not_enabled("cognito"))
    }
}
