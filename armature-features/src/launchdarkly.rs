//! LaunchDarkly integration (optional, `launchdarkly` feature).
//!
//! This module adapts armature-features' [`EvaluationContext`] and
//! [`Variation`] types onto the official [`launchdarkly-server-sdk`] client so
//! that flags managed in LaunchDarkly can be evaluated through the same shapes
//! used by the local flag engine.
//!
//! ```no_run
//! # #[cfg(feature = "launchdarkly")]
//! # fn demo() -> Result<(), armature_features::launchdarkly::LaunchDarklyError> {
//! use armature_features::{EvaluationContext, Variation};
//! use armature_features::launchdarkly::LaunchDarklyProvider;
//!
//! let provider = LaunchDarklyProvider::new("sdk-key-123")?;
//! provider.start(); // begins the background stream connection
//!
//! let ctx = EvaluationContext::new()
//!     .with_user_id("user-42")
//!     .with_attribute("plan", "pro");
//!
//! let variation = provider.evaluate("new-checkout", &ctx, &Variation::boolean(false))?;
//! let enabled = variation.as_bool().unwrap_or(false);
//! # let _ = enabled;
//! # Ok(())
//! # }
//! ```

use crate::flag::{EvaluationContext, Variation};
use launchdarkly_server_sdk::{AttributeValue, Client, ConfigBuilder, Context, ContextBuilder};

/// Errors surfaced by the LaunchDarkly adapter.
#[derive(Debug, thiserror::Error)]
pub enum LaunchDarklyError {
    /// The armature context could not be mapped to an LD [`Context`].
    #[error("failed to build LaunchDarkly context: {0}")]
    Context(String),

    /// The LD client configuration could not be built.
    #[error("failed to build LaunchDarkly config: {0}")]
    Config(String),

    /// The LD client could not be constructed.
    #[error("failed to build LaunchDarkly client: {0}")]
    Client(String),
}

/// The context `kind` used when mapping armature contexts into LaunchDarkly.
const DEFAULT_KIND: &str = "user";

/// The key used for a LaunchDarkly context when no `user_id` is present.
const ANONYMOUS_KEY: &str = "anonymous";

/// Evaluation adapter that resolves armature-features flags via a LaunchDarkly
/// [`Client`].
///
/// The provider owns an LD client and translates each evaluation request from
/// armature's [`EvaluationContext`]/[`Variation`] vocabulary into the SDK's
/// typed `*_variation` calls, picking the typed call from the shape of the
/// supplied default variation.
pub struct LaunchDarklyProvider {
    client: Client,
}

impl LaunchDarklyProvider {
    /// Build a provider against LaunchDarkly's live streaming service using the
    /// given server-side SDK key.
    ///
    /// Constructing the provider does not open a network connection; call
    /// [`LaunchDarklyProvider::start`] to begin streaming flag data.
    pub fn new(sdk_key: &str) -> Result<Self, LaunchDarklyError> {
        Self::build(sdk_key, false)
    }

    /// Build a provider in **offline** mode.
    ///
    /// In offline mode the client never contacts LaunchDarkly and every
    /// evaluation returns the supplied default. This is primarily useful for
    /// tests and for gracefully degrading when LD is unreachable.
    pub fn offline(sdk_key: &str) -> Result<Self, LaunchDarklyError> {
        Self::build(sdk_key, true)
    }

    fn build(sdk_key: &str, offline: bool) -> Result<Self, LaunchDarklyError> {
        let config = ConfigBuilder::new(sdk_key)
            .offline(offline)
            .build()
            .map_err(|e| LaunchDarklyError::Config(e.to_string()))?;
        let client = Client::build(config).map_err(|e| LaunchDarklyError::Client(e.to_string()))?;
        Ok(Self { client })
    }

    /// Wrap an already-constructed LaunchDarkly [`Client`].
    pub fn from_client(client: Client) -> Self {
        Self { client }
    }

    /// Begin the client's background data source connection (non-blocking).
    pub fn start(&self) {
        self.client.start_with_default_executor();
    }

    /// Whether the underlying client has received an initial flag payload.
    pub fn initialized(&self) -> bool {
        self.client.initialized()
    }

    /// Access the underlying LaunchDarkly [`Client`].
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Evaluate `flag_key` for `context`, returning a [`Variation`] whose type
    /// matches `default`.
    ///
    /// The variant of `default` selects the LaunchDarkly typed evaluation used:
    /// [`Variation::Boolean`] → `bool_variation`, [`Variation::String`] →
    /// `str_variation`, [`Variation::Number`] → `float_variation`, and
    /// [`Variation::Json`] → `json_variation`. If LaunchDarkly returns no value
    /// (unknown flag, offline, uninitialized), the default is returned.
    pub fn evaluate(
        &self,
        flag_key: &str,
        context: &EvaluationContext,
        default: &Variation,
    ) -> Result<Variation, LaunchDarklyError> {
        let ld_ctx = to_ld_context(context)?;

        let variation = match default {
            Variation::Boolean(d) => {
                Variation::Boolean(self.client.bool_variation(&ld_ctx, flag_key, *d))
            }
            Variation::String(d) => {
                Variation::String(self.client.str_variation(&ld_ctx, flag_key, d.clone()))
            }
            Variation::Number(d) => {
                Variation::Number(self.client.float_variation(&ld_ctx, flag_key, *d))
            }
            Variation::Json(d) => {
                Variation::Json(self.client.json_variation(&ld_ctx, flag_key, d.clone()))
            }
        };

        Ok(variation)
    }
}

/// Map an armature-features [`EvaluationContext`] onto a LaunchDarkly
/// [`Context`].
///
/// The `user_id` attribute becomes the LaunchDarkly context key (falling back
/// to `"anonymous"` when absent). Every other attribute is copied as a string
/// custom attribute. The context `kind` is always `"user"`.
pub fn to_ld_context(context: &EvaluationContext) -> Result<Context, LaunchDarklyError> {
    let key = context.user_id().unwrap_or(ANONYMOUS_KEY);
    let mut builder = ContextBuilder::new(key);
    builder.kind(DEFAULT_KIND);

    for (name, value) in context.attributes() {
        // The key is already carried by `user_id`; don't also set it as a
        // custom attribute.
        if name == "user_id" {
            continue;
        }
        builder.set_value(name, AttributeValue::String(value.to_string()));
    }

    builder.build().map_err(LaunchDarklyError::Context)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Adapter-wiring unit test that runs by default (no LaunchDarkly account,
    // no network). It exercises the context mapping and the offline evaluation
    // path end-to-end through the real SDK types.

    #[test]
    fn context_mapping_uses_user_id_as_key() {
        let ctx = EvaluationContext::new()
            .with_user_id("user-42")
            .with_attribute("plan", "pro");

        let ld_ctx = to_ld_context(&ctx).expect("context should build");
        assert_eq!(ld_ctx.key(), "user-42");
        assert_eq!(ld_ctx.kind().to_string(), "user");
    }

    #[test]
    fn context_mapping_defaults_key_when_no_user_id() {
        let ctx = EvaluationContext::new().with_attribute("region", "eu");
        let ld_ctx = to_ld_context(&ctx).expect("context should build");
        assert_eq!(ld_ctx.key(), "anonymous");
    }

    #[test]
    fn offline_provider_returns_defaults_for_each_type() {
        let provider = LaunchDarklyProvider::offline("sdk-test").expect("offline client builds");
        let ctx = EvaluationContext::new().with_user_id("user-1");

        // Offline mode: unknown flags resolve to the supplied default, proving
        // the type-directed dispatch selects the right typed evaluation.
        let b = provider
            .evaluate("flag-bool", &ctx, &Variation::boolean(true))
            .unwrap();
        assert_eq!(b.as_bool(), Some(true));

        let s = provider
            .evaluate("flag-str", &ctx, &Variation::string("fallback"))
            .unwrap();
        assert_eq!(s.as_string(), Some("fallback"));

        let n = provider
            .evaluate("flag-num", &ctx, &Variation::number(4.2))
            .unwrap();
        assert_eq!(n.as_number(), Some(4.2));

        let j = provider
            .evaluate(
                "flag-json",
                &ctx,
                &Variation::Json(serde_json::json!({"k": "v"})),
            )
            .unwrap();
        match j {
            Variation::Json(v) => assert_eq!(v, serde_json::json!({"k": "v"})),
            other => panic!("expected json variation, got {other:?}"),
        }
    }

    // Live-integration smoke test: requires a real LaunchDarkly SDK key /
    // relay and network access, so it is ignored by default.
    #[test]
    #[ignore = "requires a live LaunchDarkly account/relay and network access"]
    fn live_evaluation_smoke() {
        let sdk_key = std::env::var("LAUNCHDARKLY_SDK_KEY")
            .expect("set LAUNCHDARKLY_SDK_KEY to run the live test");
        let provider = LaunchDarklyProvider::new(&sdk_key).expect("client builds");
        provider.start();
        // Give the stream a moment to initialize in a real environment.
        std::thread::sleep(std::time::Duration::from_secs(5));
        assert!(
            provider.initialized(),
            "client should initialize against LD"
        );

        let ctx = EvaluationContext::new().with_user_id("live-user");
        let result = provider
            .evaluate("test-flag", &ctx, &Variation::boolean(false))
            .expect("evaluation succeeds");
        // We can't assert the concrete value without knowing the LD project's
        // configuration, only that a boolean came back.
        assert!(result.as_bool().is_some());
    }
}
