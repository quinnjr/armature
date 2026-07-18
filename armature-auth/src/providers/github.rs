//! GitHub OAuth2 Provider

use crate::error::AuthError;
use crate::oauth2::OAuth2Config;
use serde::{Deserialize, Serialize};

const AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_INFO_URL: &str = "https://api.github.com/user";
const USER_EMAIL_URL: &str = "https://api.github.com/user/emails";

/// GitHub user information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub id: u64,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
}

/// GitHub email information
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

/// GitHub OAuth2 provider
pub struct GitHubProvider;

impl GitHubProvider {
    /// Create a new GitHub OAuth2 configuration
    pub fn config(client_id: String, client_secret: String, redirect_url: String) -> OAuth2Config {
        OAuth2Config::new(
            client_id,
            client_secret,
            AUTH_URL.to_string(),
            TOKEN_URL.to_string(),
            redirect_url,
        )
        .with_scopes(vec!["user:email".to_string()])
        .with_user_info_url(USER_INFO_URL.to_string())
    }

    /// Fetch user info from GitHub
    pub async fn get_user_info(access_token: &str) -> Result<GitHubUser, AuthError> {
        let client = crate::providers::shared_http_client();

        // Get user profile
        let mut user: GitHubUser = client
            .get(USER_INFO_URL)
            .header("Authorization", format!("token {}", access_token))
            .header("User-Agent", "Armature-Auth")
            .send()
            .await
            .map_err(|e| AuthError::HttpRequest(e.to_string()))?
            .json()
            .await
            .map_err(|e| AuthError::InvalidResponse(e.to_string()))?;

        // If email is not in profile, fetch from emails endpoint
        if user.email.is_none() {
            let emails: Vec<GitHubEmail> = client
                .get(USER_EMAIL_URL)
                .header("Authorization", format!("token {}", access_token))
                .header("User-Agent", "Armature-Auth")
                .send()
                .await
                .map_err(|e| AuthError::HttpRequest(e.to_string()))?
                .json()
                .await
                .map_err(|e| AuthError::InvalidResponse(e.to_string()))?;

            // Find primary verified email
            user.email = emails
                .iter()
                .find(|e| e.primary && e.verified)
                .or_else(|| emails.first())
                .map(|e| e.email.clone());
        }

        Ok(user)
    }
}
