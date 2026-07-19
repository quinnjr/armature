// OAuth2 and OIDC provider support

use crate::{AuthError, Result};
use async_trait::async_trait;
use oauth2::basic::BasicTokenType;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointSet, ExtraTokenFields,
    RedirectUrl, Scope, StandardErrorResponse, StandardRevocableToken,
    StandardTokenIntrospectionResponse, StandardTokenResponse, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

/// Type alias for a fully configured OAuth2 client
type ConfiguredClient = oauth2::Client<
    StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
    StandardTokenResponse<OidcExtraFields, BasicTokenType>,
    StandardTokenIntrospectionResponse<OidcExtraFields, BasicTokenType>,
    StandardRevocableToken,
    StandardErrorResponse<oauth2::RevocationErrorResponseType>,
    EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    EndpointSet,
>;

/// OAuth2 provider trait
#[async_trait]
pub trait OAuth2Provider: Send + Sync {
    /// Get the provider name
    fn name(&self) -> &str;

    /// Get authorization URL
    fn authorization_url(&self) -> Result<(Url, CsrfToken)>;

    /// Exchange authorization code for token
    async fn exchange_code(&self, code: String) -> Result<OAuth2Token>;

    /// Get user info from token
    async fn get_user_info(&self, token: &OAuth2Token) -> Result<OAuth2UserInfo>;

    /// Refresh access token
    async fn refresh_token(&self, refresh_token: String) -> Result<OAuth2Token>;
}

/// OAuth2 token response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub id_token: Option<String>, // For OIDC
}

/// Extra token-endpoint fields beyond the OAuth2 base response, needed to
/// surface the OIDC `id_token` that a plain `BasicClient` response discards.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OidcExtraFields {
    #[serde(default)]
    pub id_token: Option<String>,
}

impl ExtraTokenFields for OidcExtraFields {}

impl From<StandardTokenResponse<OidcExtraFields, oauth2::basic::BasicTokenType>> for OAuth2Token {
    fn from(token: StandardTokenResponse<OidcExtraFields, oauth2::basic::BasicTokenType>) -> Self {
        let id_token = token.extra_fields().id_token.clone();
        Self {
            access_token: token.access_token().secret().clone(),
            token_type: token.token_type().as_ref().to_string(),
            expires_in: token.expires_in().map(|d| d.as_secs()),
            refresh_token: token.refresh_token().map(|t| t.secret().clone()),
            scope: token.scopes().map(|s| {
                s.iter()
                    .map(|scope| scope.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
            id_token,
        }
    }
}

/// OAuth2 user information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2UserInfo {
    pub sub: String, // Subject (user ID)
    pub email: Option<String>,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub picture: Option<String>,
    pub email_verified: Option<bool>,
    #[serde(flatten)]
    pub additional: HashMap<String, serde_json::Value>,
}

/// Generic OAuth2 client configuration
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
    pub user_info_url: Option<String>,
}

impl OAuth2Config {
    pub fn new(
        client_id: String,
        client_secret: String,
        auth_url: String,
        token_url: String,
        redirect_url: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            auth_url,
            token_url,
            redirect_url,
            scopes: Vec::new(),
            user_info_url: None,
        }
    }

    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    pub fn with_user_info_url(mut self, url: String) -> Self {
        self.user_info_url = Some(url);
        self
    }
}

/// Generic OAuth2 provider implementation
pub struct GenericOAuth2Provider {
    name: String,
    client: ConfiguredClient,
    config: OAuth2Config,
    /// HTTP client used for token endpoint requests (oauth2's reqwest, v0.12).
    ///
    /// Built once at construction and reused for every call. Constructing a
    /// `reqwest::Client` allocates a connection pool and loads the TLS root
    /// store, so it must not be rebuilt per request.
    oauth_http_client: oauth2::reqwest::Client,
}

impl GenericOAuth2Provider {
    /// The configured userinfo endpoint URL, if any.
    pub(crate) fn user_info_url(&self) -> Option<&str> {
        self.config.user_info_url.as_deref()
    }

    pub fn new(name: String, config: OAuth2Config) -> Result<Self> {
        let client = oauth2::Client::<
            StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
            StandardTokenResponse<OidcExtraFields, BasicTokenType>,
            StandardTokenIntrospectionResponse<OidcExtraFields, BasicTokenType>,
            StandardRevocableToken,
            StandardErrorResponse<oauth2::RevocationErrorResponseType>,
        >::new(ClientId::new(config.client_id.clone()))
        .set_client_secret(ClientSecret::new(config.client_secret.clone()))
        .set_auth_uri(
            AuthUrl::new(config.auth_url.clone())
                .map_err(|e| AuthError::AuthenticationFailed(e.to_string()))?,
        )
        .set_token_uri(
            TokenUrl::new(config.token_url.clone())
                .map_err(|e| AuthError::AuthenticationFailed(e.to_string()))?,
        )
        .set_redirect_uri(
            RedirectUrl::new(config.redirect_url.clone())
                .map_err(|e| AuthError::AuthenticationFailed(e.to_string()))?,
        );

        Ok(Self {
            name,
            client,
            config,
            oauth_http_client: oauth2::reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| oauth2::reqwest::Client::new()),
        })
    }
}

#[async_trait]
impl OAuth2Provider for GenericOAuth2Provider {
    fn name(&self) -> &str {
        &self.name
    }

    fn authorization_url(&self) -> Result<(Url, CsrfToken)> {
        let mut auth_request = self.client.authorize_url(CsrfToken::new_random);

        for scope in &self.config.scopes {
            auth_request = auth_request.add_scope(Scope::new(scope.clone()));
        }

        let (url, csrf_token) = auth_request.url();
        Ok((url, csrf_token))
    }

    async fn exchange_code(&self, code: String) -> Result<OAuth2Token> {
        let token = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .request_async(&self.oauth_http_client)
            .await
            .map_err(|e| {
                AuthError::AuthenticationFailed(format!("Token exchange failed: {}", e))
            })?;

        Ok(token.into())
    }

    async fn get_user_info(&self, token: &OAuth2Token) -> Result<OAuth2UserInfo> {
        let user_info_url =
            self.config.user_info_url.as_ref().ok_or_else(|| {
                AuthError::AuthenticationFailed("No user info URL configured".into())
            })?;

        let response = crate::providers::shared_http_client()
            .get(user_info_url)
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(|e| {
                AuthError::AuthenticationFailed(format!("User info request failed: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(AuthError::AuthenticationFailed(format!(
                "User info request failed with status: {}",
                response.status()
            )));
        }

        let user_info: OAuth2UserInfo = response.json().await.map_err(|e| {
            AuthError::AuthenticationFailed(format!("Failed to parse user info: {}", e))
        })?;

        Ok(user_info)
    }

    async fn refresh_token(&self, refresh_token: String) -> Result<OAuth2Token> {
        let token = self
            .client
            .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token))
            .request_async(&self.oauth_http_client)
            .await
            .map_err(|e| AuthError::AuthenticationFailed(format!("Token refresh failed: {}", e)))?;

        Ok(token.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth2_config() {
        let config = OAuth2Config::new(
            "client_id".to_string(),
            "client_secret".to_string(),
            "https://example.com/auth".to_string(),
            "https://example.com/token".to_string(),
            "https://example.com/callback".to_string(),
        )
        .with_scopes(vec!["openid".to_string(), "profile".to_string()])
        .with_user_info_url("https://example.com/userinfo".to_string());

        assert_eq!(config.client_id, "client_id");
        assert_eq!(config.scopes.len(), 2);
        assert!(config.user_info_url.is_some());
    }

    #[tokio::test]
    async fn exchange_code_surfaces_oidc_id_token() {
        let server = armature_testkit::StubServer::builder()
            .route(
                "POST",
                "/token",
                armature_testkit::StubResponse::json(
                    200,
                    r#"{
                        "access_token": "at-123",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                        "id_token": "header.payload.signature"
                    }"#,
                ),
            )
            .start()
            .await;

        let config = OAuth2Config::new(
            "client_id".to_string(),
            "client_secret".to_string(),
            format!("{}/authorize", server.url()),
            format!("{}/token", server.url()),
            "https://example.com/callback".to_string(),
        );
        let provider = GenericOAuth2Provider::new("test".to_string(), config).unwrap();

        let token = provider
            .exchange_code("some-code".to_string())
            .await
            .unwrap();

        assert_eq!(token.access_token, "at-123");
        assert_eq!(token.id_token.as_deref(), Some("header.payload.signature"));
    }
}
