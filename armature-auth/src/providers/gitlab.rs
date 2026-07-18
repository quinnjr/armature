//! GitLab OAuth2 Provider

use crate::error::AuthError;
use crate::oauth2::OAuth2Config;
use serde::{Deserialize, Serialize};

const AUTH_URL: &str = "https://gitlab.com/oauth/authorize";
const TOKEN_URL: &str = "https://gitlab.com/oauth/token";
const USER_INFO_URL: &str = "https://gitlab.com/api/v4/user";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabUser {
    pub id: u64,
    pub username: String,
    pub name: String,
    pub email: String,
    pub avatar_url: Option<String>,
    pub state: String,
}

pub struct GitLabProvider;

impl GitLabProvider {
    /// Create a new GitLab OAuth2 configuration
    pub fn config(client_id: String, client_secret: String, redirect_url: String) -> OAuth2Config {
        OAuth2Config::new(
            client_id,
            client_secret,
            AUTH_URL.to_string(),
            TOKEN_URL.to_string(),
            redirect_url,
        )
        .with_scopes(vec!["read_user".to_string()])
        .with_user_info_url(USER_INFO_URL.to_string())
    }

    pub async fn get_user_info(access_token: &str) -> Result<GitLabUser, AuthError> {
        let client = crate::providers::shared_http_client();

        let user: GitLabUser = client
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
