//! Apple Push Notification Service (APNS) provider.

use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::debug;

use crate::{Notification, Platform, Priority, PushError, PushProvider, Result};

/// APNS environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApnsEnvironment {
    /// Development/sandbox environment.
    Development,
    /// Production environment.
    #[default]
    Production,
}

impl ApnsEnvironment {
    fn endpoint(&self) -> &str {
        match self {
            Self::Development => "https://api.sandbox.push.apple.com",
            Self::Production => "https://api.push.apple.com",
        }
    }
}

/// APNS configuration.
#[derive(Debug, Clone)]
pub struct ApnsConfig {
    /// Team ID.
    pub team_id: String,
    /// Key ID.
    pub key_id: String,
    /// Private key (P8 format).
    pub private_key: String,
    /// Bundle ID (topic).
    pub bundle_id: String,
    /// Environment.
    pub environment: ApnsEnvironment,
}

impl ApnsConfig {
    /// Create a new APNS configuration.
    pub fn new(
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        private_key: impl Into<String>,
        bundle_id: impl Into<String>,
    ) -> Self {
        Self {
            team_id: team_id.into(),
            key_id: key_id.into(),
            private_key: private_key.into(),
            bundle_id: bundle_id.into(),
            environment: ApnsEnvironment::Production,
        }
    }

    /// Load private key from a P8 file.
    pub fn from_p8_file(
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        p8_path: impl AsRef<std::path::Path>,
        bundle_id: impl Into<String>,
    ) -> Result<Self> {
        let private_key = std::fs::read_to_string(p8_path)?;
        Ok(Self::new(team_id, key_id, private_key, bundle_id))
    }

    /// Set the environment.
    pub fn environment(mut self, env: ApnsEnvironment) -> Self {
        self.environment = env;
        self
    }

    /// Use development environment.
    pub fn development(self) -> Self {
        self.environment(ApnsEnvironment::Development)
    }

    /// Use production environment.
    pub fn production(self) -> Self {
        self.environment(ApnsEnvironment::Production)
    }
}

/// APNS provider.
pub struct ApnsProvider {
    config: ApnsConfig,
    client: Client,
    access_token: RwLock<Option<AccessToken>>,
}

struct AccessToken {
    token: String,
    expires_at: Instant,
}

impl ApnsProvider {
    /// Create a new APNS provider.
    pub async fn new(config: ApnsConfig) -> Result<Self> {
        let client = Client::builder()
            .http2_prior_knowledge()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| PushError::Config(e.to_string()))?;

        Ok(Self {
            config,
            client,
            access_token: RwLock::new(None),
        })
    }

    /// Get or create the JWT token.
    fn get_token(&self) -> Result<String> {
        // Check if we have a valid token
        {
            let token = self.access_token.read().unwrap();
            if let Some(t) = token.as_ref() {
                // APNS tokens are valid for 1 hour, refresh after 50 mins
                if t.expires_at > Instant::now() {
                    return Ok(t.token.clone());
                }
            }
        }

        // Create a new token
        self.create_token()
    }

    /// Create a new JWT token.
    fn create_token(&self) -> Result<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        #[derive(Serialize)]
        struct Claims {
            iss: String,
            iat: i64,
        }

        let claims = Claims {
            iss: self.config.team_id.clone(),
            iat: now,
        };

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());

        let key = EncodingKey::from_ec_pem(self.config.private_key.as_bytes())
            .map_err(|e| PushError::Config(format!("Invalid private key: {}", e)))?;

        let token = encode(&header, &claims, &key)
            .map_err(|e| PushError::Config(format!("JWT encoding failed: {}", e)))?;

        // Cache the token (valid for ~50 minutes)
        let access_token = AccessToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(3000),
        };

        *self.access_token.write().unwrap() = Some(access_token);

        Ok(token)
    }

    /// Build APNS payload.
    fn build_payload(&self, notification: &Notification) -> ApnsPayload {
        let mut aps = ApnsAps {
            alert: None,
            badge: notification.badge,
            sound: notification.sound.clone(),
            content_available: if notification.content_available {
                Some(1)
            } else {
                None
            },
            mutable_content: if notification.mutable_content {
                Some(1)
            } else {
                None
            },
            category: notification.click_action.clone(),
            thread_id: notification.tag.clone(),
        };

        // Add alert if not silent
        if !notification.silent {
            aps.alert = Some(ApnsAlert {
                title: Some(notification.title.clone()),
                body: Some(notification.body.clone()),
                subtitle: None,
            });
        }

        let mut payload = ApnsPayload {
            aps,
            custom: std::collections::HashMap::new(),
        };

        // Add custom data
        for (key, value) in &notification.data {
            payload
                .custom
                .insert(key.clone(), serde_json::Value::String(value.clone()));
        }

        payload
    }
}

#[async_trait]
impl PushProvider for ApnsProvider {
    async fn send(&self, token: &str, notification: &Notification) -> Result<()> {
        let jwt = self.get_token()?;
        let payload = self.build_payload(notification);

        let url = format!("{}/3/device/{}", self.config.environment.endpoint(), token);

        debug!(token = %token, "Sending APNS notification");

        let mut request = self
            .client
            .post(&url)
            .header("authorization", format!("bearer {}", jwt))
            .header("apns-topic", &self.config.bundle_id)
            .header(
                "apns-push-type",
                if notification.silent {
                    "background"
                } else {
                    "alert"
                },
            );

        // Set priority
        request = request.header(
            "apns-priority",
            match notification.priority {
                Priority::High => "10",
                Priority::Normal => "5",
            },
        );

        // Set TTL (expiration)
        if let Some(ttl) = notification.ttl {
            let expiration = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + ttl as u64;
            request = request.header("apns-expiration", expiration.to_string());
        }

        // Set collapse ID
        if let Some(collapse_key) = &notification.collapse_key {
            request = request.header("apns-collapse-id", collapse_key);
        }

        let response = request.json(&payload).send().await?;

        let status = response.status();

        if status.is_success() {
            debug!("APNS notification sent successfully");
            Ok(())
        } else if status.as_u16() == 410 {
            Err(PushError::Unregistered(token.to_string()))
        } else if status.as_u16() == 400 {
            let body = response.text().await.unwrap_or_default();
            if body.contains("BadDeviceToken") || body.contains("DeviceTokenNotForTopic") {
                Err(PushError::InvalidSubscription(token.to_string()))
            } else {
                Err(PushError::Provider(format!("APNS error: {}", body)))
            }
        } else if status.as_u16() == 429 {
            Err(PushError::RateLimited(60))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(PushError::Provider(format!(
                "APNS error {}: {}",
                status, body
            )))
        }
    }

    fn platform(&self) -> Platform {
        Platform::Ios
    }
}

// APNS payload types

#[derive(Serialize)]
struct ApnsPayload {
    aps: ApnsAps,
    #[serde(flatten)]
    custom: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct ApnsAps {
    #[serde(skip_serializing_if = "Option::is_none")]
    alert: Option<ApnsAlert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    badge: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<String>,
    #[serde(rename = "content-available", skip_serializing_if = "Option::is_none")]
    content_available: Option<u8>,
    #[serde(rename = "mutable-content", skip_serializing_if = "Option::is_none")]
    mutable_content: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(rename = "thread-id", skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
}

#[derive(Serialize)]
struct ApnsAlert {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
}
