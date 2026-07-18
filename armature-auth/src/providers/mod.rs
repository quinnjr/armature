// OAuth2 provider implementations

use std::sync::OnceLock;

/// Shared `reqwest::Client` for the stateless (unit-struct) providers.
///
/// Building a `reqwest::Client` allocates a connection pool and loads the TLS
/// root store, so constructing one per request is wasteful. These providers
/// expose only static async methods and hold no state, so they share a single
/// lazily-initialized client and clone the cheap `Arc`-backed handle per call.
pub(crate) fn shared_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

pub mod auth0;
pub mod aws_cognito;
pub mod discord;
pub mod github;
pub mod gitlab;
pub mod google;
pub mod linkedin;
pub mod microsoft;
pub mod okta;

pub use auth0::{Auth0Config, Auth0Provider};
pub use aws_cognito::{AwsCognitoConfig, AwsCognitoProvider};
pub use discord::{DiscordProvider, DiscordUser};
pub use github::{GitHubProvider, GitHubUser};
pub use gitlab::{GitLabProvider, GitLabUser};
pub use google::{GoogleConfig, GoogleProvider};
pub use linkedin::{LinkedInProvider, LinkedInUser};
pub use microsoft::{MicrosoftEntraConfig, MicrosoftEntraProvider};
pub use okta::{OktaConfig, OktaProvider};
