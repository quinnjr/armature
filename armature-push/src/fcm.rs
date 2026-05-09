//! Firebase Cloud Messaging (FCM) provider.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::debug;

use crate::{Notification, Platform, Priority, PushError, PushProvider, Result};

/// FCM configuration.
#[derive(Debug, Clone)]
pub struct FcmConfig {
    /// Project ID.
    pub project_id: String,
    /// Service account credentials (JSON).
    pub credentials: FcmCredentials,
}

/// FCM service account credentials.
#[derive(Debug, Clone, Deserialize)]
pub struct FcmCredentials {
    /// Client email.
    pub client_email: String,
    /// Private key (PEM format).
    pub private_key: String,
    /// Token URI.
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

impl FcmConfig {
    /// Create config from a service account JSON file.
    pub fn from_service_account(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_service_account_json(&content)
    }

    /// Create config from service account JSON string.
    pub fn from_service_account_json(json: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct ServiceAccount {
            project_id: String,
            client_email: String,
            private_key: String,
            #[serde(default = "default_token_uri")]
            token_uri: String,
        }

        let sa: ServiceAccount =
            serde_json::from_str(json).map_err(|e| PushError::Config(e.to_string()))?;

        Ok(Self {
            project_id: sa.project_id,
            credentials: FcmCredentials {
                client_email: sa.client_email,
                private_key: sa.private_key,
                token_uri: sa.token_uri,
            },
        })
    }

    /// Create with explicit credentials.
    pub fn new(
        project_id: impl Into<String>,
        client_email: impl Into<String>,
        private_key: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            credentials: FcmCredentials {
                client_email: client_email.into(),
                private_key: private_key.into(),
                token_uri: default_token_uri(),
            },
        }
    }
}

/// FCM provider.
pub struct FcmProvider {
    config: FcmConfig,
    client: Client,
    access_token: RwLock<Option<AccessToken>>,
}

struct AccessToken {
    token: String,
    expires_at: Instant,
}

impl FcmProvider {
    /// Create a new FCM provider.
    pub async fn new(config: FcmConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| PushError::Config(e.to_string()))?;

        let provider = Self {
            config,
            client,
            access_token: RwLock::new(None),
        };

        // Get initial token
        provider.refresh_token().await?;

        Ok(provider)
    }

    /// Get or refresh the access token.
    async fn get_access_token(&self) -> Result<String> {
        // Check if we have a valid token
        {
            let token = self.access_token.read().unwrap();
            if let Some(t) = token.as_ref()
                && t.expires_at > Instant::now() + Duration::from_secs(60)
            {
                return Ok(t.token.clone());
            }
        }

        // Refresh the token
        self.refresh_token().await
    }

    /// Refresh the access token.
    async fn refresh_token(&self) -> Result<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        #[derive(Serialize)]
        struct Claims {
            iss: String,
            scope: String,
            aud: String,
            iat: i64,
            exp: i64,
        }

        let claims = Claims {
            iss: self.config.credentials.client_email.clone(),
            scope: "https://www.googleapis.com/auth/firebase.messaging".to_string(),
            aud: self.config.credentials.token_uri.clone(),
            iat: now,
            exp: now + 3600,
        };

        let key = EncodingKey::from_rsa_pem(self.config.credentials.private_key.as_bytes())
            .map_err(|e| PushError::Config(format!("Invalid private key: {}", e)))?;

        let jwt = encode(&Header::new(Algorithm::RS256), &claims, &key)
            .map_err(|e| PushError::Config(format!("JWT encoding failed: {}", e)))?;

        // Exchange JWT for access token
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        let response: TokenResponse = self
            .client
            .post(&self.config.credentials.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await?
            .json()
            .await?;

        let token = AccessToken {
            token: response.access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(response.expires_in),
        };

        *self.access_token.write().unwrap() = Some(token);

        Ok(response.access_token)
    }

    /// Build FCM message payload.
    fn build_payload(&self, token: &str, notification: &Notification) -> FcmMessage {
        let mut message = FcmMessage {
            token: token.to_string(),
            notification: None,
            data: None,
            android: None,
            apns: None,
            webpush: None,
        };

        // Add notification if not silent
        if !notification.silent {
            message.notification = Some(FcmNotification {
                title: Some(notification.title.clone()),
                body: Some(notification.body.clone()),
                image: notification.image.clone(),
            });
        }

        // Add data if present
        if !notification.data.is_empty() {
            message.data = Some(notification.data.clone());
        }

        // Android-specific config
        message.android = Some(FcmAndroidConfig {
            priority: match notification.priority {
                Priority::High => "high".to_string(),
                Priority::Normal => "normal".to_string(),
            },
            ttl: notification.ttl.map(|t| format!("{}s", t)),
            collapse_key: notification.collapse_key.clone(),
            notification: Some(FcmAndroidNotification {
                icon: notification.icon.clone(),
                color: None,
                sound: notification.sound.clone(),
                tag: notification.tag.clone(),
                click_action: notification.click_action.clone(),
            }),
        });

        message
    }
}

#[async_trait]
impl PushProvider for FcmProvider {
    async fn send(&self, token: &str, notification: &Notification) -> Result<()> {
        let access_token = self.get_access_token().await?;
        let payload = self.build_payload(token, notification);

        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.config.project_id
        );

        debug!(token = %token, "Sending FCM notification");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&FcmRequest { message: payload })
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            debug!("FCM notification sent successfully");
            Ok(())
        } else if status.as_u16() == 404 {
            Err(PushError::Unregistered(token.to_string()))
        } else if status.as_u16() == 429 {
            Err(PushError::RateLimited(60))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(PushError::Provider(format!(
                "FCM error {}: {}",
                status, body
            )))
        }
    }

    fn platform(&self) -> Platform {
        Platform::Android
    }
}

// FCM API types

#[derive(Serialize)]
struct FcmRequest {
    message: FcmMessage,
}

#[derive(Serialize)]
struct FcmMessage {
    token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<FcmNotification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    android: Option<FcmAndroidConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    apns: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    webpush: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct FcmNotification {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

#[derive(Serialize)]
struct FcmAndroidConfig {
    priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collapse_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<FcmAndroidNotification>,
}

#[derive(Serialize)]
struct FcmAndroidNotification {
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    click_action: Option<String>,
}
