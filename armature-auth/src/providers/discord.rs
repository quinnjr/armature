//! Discord OAuth2 Provider

use crate::error::AuthError;
use crate::oauth2::OAuth2Config;
use serde::{Deserialize, Serialize};

const AUTH_URL: &str = "https://discord.com/api/oauth2/authorize";
const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
const USER_INFO_URL: &str = "https://discord.com/api/users/@me";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
    pub verified: Option<bool>,
}

pub struct DiscordProvider;

impl DiscordProvider {
    /// Create a new Discord OAuth2 configuration
    pub fn config(client_id: String, client_secret: String, redirect_url: String) -> OAuth2Config {
        OAuth2Config::new(
            client_id,
            client_secret,
            AUTH_URL.to_string(),
            TOKEN_URL.to_string(),
            redirect_url,
        )
        .with_scopes(vec!["identify".to_string(), "email".to_string()])
        .with_user_info_url(USER_INFO_URL.to_string())
    }

    pub async fn get_user_info(access_token: &str) -> Result<DiscordUser, AuthError> {
        let client = crate::providers::shared_http_client();

        let user: DiscordUser = client
            .get(USER_INFO_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| AuthError::HttpRequest(e.to_string()))?
            .json()
            .await
            .map_err(|e| AuthError::InvalidResponse(e.to_string()))?;

        Ok(user)
    }
}
