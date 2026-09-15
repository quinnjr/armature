//! Job definitions

use serde::{Deserialize, Serialize};

/// Email job payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailJob {
    pub to: String,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub html: bool,
}

/// Notification job payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationJob {
    pub user_id: String,
    pub message: String,
    pub channel: NotificationChannel,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationChannel {
    Push,
    Sms,
    InApp,
    Slack,
    Webhook,
}

impl Default for NotificationChannel {
    fn default() -> Self {
        Self::InApp
    }
}

/// Data processing job payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessDataJob {
    pub data_id: String,
    pub operation: String,
    #[serde(default)]
    pub params: serde_json::Value,
}
