// Microsoft Entra (Azure AD) OAuth2 provider

use crate::oauth2::{
    GenericOAuth2Provider, OAuth2Config, OAuth2Provider, OAuth2Token, OAuth2UserInfo,
};
use crate::{AuthError, Result};
use async_trait::async_trait;
use oauth2::CsrfToken;
use serde::Deserialize;
use std::collections::HashMap;
use url::Url;

/// Shape of the Microsoft Graph `/me` response.
///
/// Graph does not use OIDC-standard claim names (`id` rather than `sub`,
/// `mail`/`userPrincipalName` rather than `email`, `displayName` rather than
/// `name`), so it is deserialized separately and mapped into
/// [`OAuth2UserInfo`].
#[derive(Debug, Clone, Deserialize)]
struct GraphUser {
    id: String,
    #[serde(default)]
    mail: Option<String>,
    #[serde(default, rename = "userPrincipalName")]
    user_principal_name: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(flatten)]
    additional: HashMap<String, serde_json::Value>,
}

impl From<GraphUser> for OAuth2UserInfo {
    fn from(graph: GraphUser) -> Self {
        Self {
            sub: graph.id,
            email: graph.mail.or(graph.user_principal_name),
            name: graph.display_name,
            given_name: None,
            family_name: None,
            picture: None,
            email_verified: None,
            additional: graph.additional,
        }
    }
}

/// Microsoft Entra provider configuration
#[derive(Debug, Clone)]
pub struct MicrosoftEntraConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    pub tenant_id: String, // "common", "organizations", "consumers", or specific tenant ID
    pub scopes: Vec<String>,
}

impl MicrosoftEntraConfig {
    pub fn new(
        client_id: String,
        client_secret: String,
        redirect_url: String,
        tenant_id: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            redirect_url,
            tenant_id,
            scopes: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ],
        }
    }

    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Create config for common tenant (any Azure AD account)
    pub fn common(client_id: String, client_secret: String, redirect_url: String) -> Self {
        Self::new(client_id, client_secret, redirect_url, "common".to_string())
    }

    /// Create config for organization accounts only
    pub fn organizations(client_id: String, client_secret: String, redirect_url: String) -> Self {
        Self::new(
            client_id,
            client_secret,
            redirect_url,
            "organizations".to_string(),
        )
    }

    /// Create config for personal Microsoft accounts
    pub fn consumers(client_id: String, client_secret: String, redirect_url: String) -> Self {
        Self::new(
            client_id,
            client_secret,
            redirect_url,
            "consumers".to_string(),
        )
    }
}

/// Microsoft Entra (Azure AD) provider
pub struct MicrosoftEntraProvider {
    inner: GenericOAuth2Provider,
}

impl MicrosoftEntraProvider {
    pub fn new(config: MicrosoftEntraConfig) -> Result<Self> {
        let auth_url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
            config.tenant_id
        );
        let token_url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            config.tenant_id
        );

        let oauth2_config = OAuth2Config::new(
            config.client_id,
            config.client_secret,
            auth_url,
            token_url,
            config.redirect_url,
        )
        .with_scopes(config.scopes)
        .with_user_info_url("https://graph.microsoft.com/v1.0/me".to_string());

        let inner = GenericOAuth2Provider::new("microsoft-entra".to_string(), oauth2_config)?;

        Ok(Self { inner })
    }
}

#[async_trait]
impl OAuth2Provider for MicrosoftEntraProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn authorization_url(&self) -> Result<(Url, CsrfToken)> {
        self.inner.authorization_url()
    }

    async fn exchange_code(&self, code: String) -> Result<OAuth2Token> {
        self.inner.exchange_code(code).await
    }

    async fn get_user_info(&self, token: &OAuth2Token) -> Result<OAuth2UserInfo> {
        // Microsoft Graph's `/me` response doesn't use OIDC claim names (`id`
        // rather than `sub`, `mail`/`userPrincipalName` rather than `email`,
        // `displayName` rather than `name`), so it's fetched and deserialized
        // into a Graph-specific shape rather than `OAuth2UserInfo` directly.
        let user_info_url = self
            .inner
            .user_info_url()
            .ok_or_else(|| AuthError::AuthenticationFailed("No user info URL configured".into()))?;

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

        let graph_user: GraphUser = response.json().await.map_err(|e| {
            AuthError::AuthenticationFailed(format!("Failed to parse user info: {}", e))
        })?;

        Ok(graph_user.into())
    }

    async fn refresh_token(&self, refresh_token: String) -> Result<OAuth2Token> {
        self.inner.refresh_token(refresh_token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_microsoft_config() {
        let config = MicrosoftEntraConfig::common(
            "client_id".to_string(),
            "client_secret".to_string(),
            "http://localhost:3000/callback".to_string(),
        );

        assert_eq!(config.tenant_id, "common");
        assert_eq!(config.scopes.len(), 3);
    }

    #[test]
    fn test_microsoft_organizations() {
        let config = MicrosoftEntraConfig::organizations(
            "client_id".to_string(),
            "client_secret".to_string(),
            "http://localhost:3000/callback".to_string(),
        );

        assert_eq!(config.tenant_id, "organizations");
    }

    fn test_provider(user_info_url: String) -> MicrosoftEntraProvider {
        let oauth2_config = OAuth2Config::new(
            "client_id".to_string(),
            "client_secret".to_string(),
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string(),
            "https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string(),
            "http://localhost:3000/callback".to_string(),
        )
        .with_user_info_url(user_info_url);
        let inner = GenericOAuth2Provider::new("microsoft-entra".to_string(), oauth2_config)
            .expect("provider config should be valid");
        MicrosoftEntraProvider { inner }
    }

    fn dummy_token() -> OAuth2Token {
        OAuth2Token {
            access_token: "at-123".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: None,
            refresh_token: None,
            scope: None,
            id_token: None,
        }
    }

    #[tokio::test]
    async fn get_user_info_maps_graph_fields_to_oauth2_user_info() {
        let server = armature_testkit::StubServer::builder()
            .route(
                "GET",
                "/me",
                armature_testkit::StubResponse::json(
                    200,
                    r#"{
                        "id": "abc-123",
                        "mail": "jane@example.com",
                        "userPrincipalName": "jane@contoso.onmicrosoft.com",
                        "displayName": "Jane Doe"
                    }"#,
                ),
            )
            .start()
            .await;

        let provider = test_provider(format!("{}/me", server.url()));
        let user_info = provider.get_user_info(&dummy_token()).await.unwrap();

        assert_eq!(user_info.sub, "abc-123");
        assert_eq!(user_info.email.as_deref(), Some("jane@example.com"));
        assert_eq!(user_info.name.as_deref(), Some("Jane Doe"));
    }

    #[tokio::test]
    async fn get_user_info_falls_back_to_user_principal_name_when_mail_absent() {
        let server = armature_testkit::StubServer::builder()
            .route(
                "GET",
                "/me",
                armature_testkit::StubResponse::json(
                    200,
                    r#"{
                        "id": "abc-456",
                        "userPrincipalName": "jane@contoso.onmicrosoft.com",
                        "displayName": "Jane Doe"
                    }"#,
                ),
            )
            .start()
            .await;

        let provider = test_provider(format!("{}/me", server.url()));
        let user_info = provider.get_user_info(&dummy_token()).await.unwrap();

        assert_eq!(user_info.sub, "abc-456");
        assert_eq!(
            user_info.email.as_deref(),
            Some("jane@contoso.onmicrosoft.com")
        );
    }
}
