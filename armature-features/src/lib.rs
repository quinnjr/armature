//! Feature Flags for Armature
//!
//! Comprehensive feature flag system with runtime toggling, A/B testing,
//! gradual rollout, and optional LaunchDarkly integration.
//!
//! # Features
//!
//! - 🚀 **Feature Flags** - Toggle features at runtime
//! - 🎯 **Targeting Rules** - User-based feature activation
//! - 📊 **A/B Testing** - Experiment framework with multivariate support
//! - 🎲 **Gradual Rollout** - Percentage-based feature rollout
//! - 🔌 **LaunchDarkly** - Optional LaunchDarkly integration
//!
//! # Quick Start
//!
//! ```
//! use armature_features::*;
//!
//! // Create a simple boolean flag
//! let flag = FeatureFlag::boolean("new-ui", true);
//!
//! // Evaluate for a user
//! let context = EvaluationContext::new().with_user_id("user-123");
//! let enabled = flag.evaluate(&context).as_bool().unwrap_or(false);
//!
//! if enabled {
//!     // Show new UI
//! }
//! ```
//!
//! # Targeting Rules
//!
//! ```
//! use armature_features::*;
//!
//! // Create rule for beta users
//! let rule = TargetingRule::new(Variation::boolean(true))
//!     .with_condition(Condition::new(
//!         "email",
//!         Operator::EndsWith,
//!         vec!["@company.com".to_string()],
//!     ));
//!
//! let flag = FeatureFlag::boolean("beta-feature", false)
//!     .with_rule(rule);
//! ```
//!
//! # Gradual Rollout
//!
//! ```
//! use armature_features::*;
//!
//! // Roll out to 25% of users
//! let rollout = Rollout::new(25, Variation::boolean(true));
//! let flag = FeatureFlag::boolean("new-algorithm", false)
//!     .with_rollout(rollout);
//! ```
//!
//! # A/B Testing
//!
//! ```
//! use armature_features::*;
//!
//! // Create multivariate flag for A/B test
//! let flag = FeatureFlag::multivariate(
//!     "button-color",
//!     vec![
//!         Variation::string("red"),
//!         Variation::string("blue"),
//!         Variation::string("green"),
//!     ],
//! );
//! ```

pub mod flag;

/// Optional LaunchDarkly integration (enabled by the `launchdarkly` feature).
#[cfg(feature = "launchdarkly")]
pub mod launchdarkly;

pub use flag::{
    Condition, EvaluationContext, FeatureFlag, Operator, Rollout, TargetingRule, Variation,
};
