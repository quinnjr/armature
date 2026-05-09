//! LinkedIn OAuth2 Provider

use crate::error::AuthError;
use crate::oauth2::OAuth2Config;
use serde::{Deserialize, Serialize};

const AUTH_URL: &str = "https://www.linkedin.com/oauth/v2/authorization";
const TOKEN_URL: &str = "https://www.linkedin.com/oauth/v2/accessToken";
const USER_INFO_URL: &str = "https://api.linkedin.com/v2/me";
const EMAIL_URL: &str =
    "https://api.linkedin.com/v2/emailAddress?q=members&projection=(elements*(handle~))";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInUser {
    pub id: String,
    #[serde(rename = "localizedFirstName")]
    pub first_name: String,
    #[serde(rename = "localizedLastName")]
    pub last_name: String,
    pub email: Option<String>,
}

pub struct LinkedInProvider;

impl LinkedInProvider {
    /// Create a new LinkedIn OAuth2 configuration
    pub fn config(client_id: String, client_secret: String, redirect_url: String) -> OAuth2Config {
        OAuth2Config::new(
            client_id,
            client_secret,
            AUTH_URL.to_string(),
            TOKEN_URL.to_string(),
            redirect_url,
        )
        .with_scopes(vec![
            "r_liteprofile".to_string(),
            "r_emailaddress".to_string(),
        ])
        .with_user_info_url(USER_INFO_URL.to_string())
    }

    pub async fn get_user_info(access_token: &str) -> Result<LinkedInUser, AuthError> {
        let client = reqwest::Client::new();

        let mut user: LinkedInUser = client
            .get(USER_INFO_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| AuthError::HttpRequest(e.to_string()))?
            .json()
            .await
            .map_err(|e| AuthError::InvalidResponse(e.to_string()))?;

        // Fetch email separately
        #[derive(Deserialize)]
        struct EmailResponse {
            elements: Vec<EmailElement>,
        }

        #[derive(Deserialize)]
        struct EmailElement {
            #[serde(rename = "handle~")]
            handle: EmailHandle,
        }

        #[derive(Deserialize)]
        struct EmailHandle {
            #[serde(rename = "emailAddress")]
            email_address: String,
        }

        if let Ok(email_resp) = client
            .get(EMAIL_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            && let Ok(email_data) = email_resp.json::<EmailResponse>().await
        {
            user.email = email_data
                .elements
                .first()
                .map(|e| e.handle.email_address.clone());
        }

        Ok(user)
    }
}
