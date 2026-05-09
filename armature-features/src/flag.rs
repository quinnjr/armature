//! Feature Flag Core
//!
//! Defines feature flags, targeting rules, and evaluation logic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Feature flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    /// Flag key/name
    pub key: String,

    /// Flag description
    pub description: Option<String>,

    /// Whether flag is enabled globally
    pub enabled: bool,

    /// Targeting rules
    pub targeting: Vec<TargetingRule>,

    /// Default variation when no rules match
    pub default_variation: Variation,

    /// All available variations
    pub variations: Vec<Variation>,

    /// Rollout configuration
    pub rollout: Option<Rollout>,
}

impl FeatureFlag {
    /// Create a new simple boolean feature flag
    ///
    /// The `default_value` parameter specifies what value to return when no targeting rules match.
    /// The flag is enabled by default (can be disabled with `flag.enabled = false`).
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_features::FeatureFlag;
    ///
    /// let flag = FeatureFlag::boolean("new-ui", true);
    /// ```
    pub fn boolean(key: impl Into<String>, default_value: bool) -> Self {
        Self {
            key: key.into(),
            description: None,
            enabled: true, // Flag is enabled by default
            targeting: Vec::new(),
            default_variation: Variation::boolean(default_value),
            variations: vec![Variation::boolean(false), Variation::boolean(true)],
            rollout: None,
        }
    }

    /// Create a new multivariate feature flag
    pub fn multivariate(key: impl Into<String>, variations: Vec<Variation>) -> Self {
        let default_variation = variations
            .first()
            .cloned()
            .unwrap_or(Variation::Boolean(false));

        Self {
            key: key.into(),
            description: None,
            enabled: true,
            targeting: Vec::new(),
            default_variation,
            variations,
            rollout: None,
        }
    }

    /// Set description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add targeting rule
    pub fn with_rule(mut self, rule: TargetingRule) -> Self {
        self.targeting.push(rule);
        self
    }

    /// Set rollout configuration
    pub fn with_rollout(mut self, rollout: Rollout) -> Self {
        self.rollout = Some(rollout);
        self
    }

    /// Evaluate flag for a context
    pub fn evaluate(&self, context: &EvaluationContext) -> Variation {
        // If flag is disabled globally, return off variation
        if !self.enabled {
            return self
                .variations
                .first()
                .cloned()
                .unwrap_or(Variation::Boolean(false));
        }

        // Check targeting rules in order
        for rule in &self.targeting {
            if rule.matches(context) {
                return rule.variation.clone();
            }
        }

        // Check rollout
        if let Some(ref rollout) = self.rollout
            && let Some(variation) = rollout.evaluate(context, &self.key)
        {
            return variation;
        }

        // Return default variation
        self.default_variation.clone()
    }
}

/// Feature flag variation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Variation {
    Boolean(bool),
    String(String),
    Number(f64),
    Json(serde_json::Value),
}

impl Variation {
    pub fn boolean(value: bool) -> Self {
        Self::Boolean(value)
    }

    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn number(value: f64) -> Self {
        Self::Number(value)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// Targeting rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetingRule {
    /// Rule conditions (all must match)
    pub conditions: Vec<Condition>,

    /// Variation to return if rule matches
    pub variation: Variation,
}

impl TargetingRule {
    pub fn new(variation: Variation) -> Self {
        Self {
            conditions: Vec::new(),
            variation,
        }
    }

    pub fn with_condition(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    pub fn matches(&self, context: &EvaluationContext) -> bool {
        self.conditions.iter().all(|c| c.matches(context))
    }
}

/// Targeting condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    /// Attribute to check
    pub attribute: String,

    /// Operator
    pub operator: Operator,

    /// Values to compare against
    pub values: Vec<String>,
}

impl Condition {
    pub fn new(attribute: impl Into<String>, operator: Operator, values: Vec<String>) -> Self {
        Self {
            attribute: attribute.into(),
            operator,
            values,
        }
    }

    pub fn matches(&self, context: &EvaluationContext) -> bool {
        let attr_value = context.get(&self.attribute);

        match self.operator {
            Operator::In => attr_value
                .map(|v| self.values.contains(&v.to_string()))
                .unwrap_or(false),
            Operator::NotIn => attr_value
                .map(|v| !self.values.contains(&v.to_string()))
                .unwrap_or(true),
            Operator::Contains => attr_value
                .map(|v| self.values.iter().any(|val| v.contains(val)))
                .unwrap_or(false),
            Operator::StartsWith => attr_value
                .map(|v| self.values.iter().any(|val| v.starts_with(val)))
                .unwrap_or(false),
            Operator::EndsWith => attr_value
                .map(|v| self.values.iter().any(|val| v.ends_with(val)))
                .unwrap_or(false),
            Operator::Matches => {
                // Regex matching would go here
                false
            }
        }
    }
}

/// Comparison operator
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operator {
    In,
    NotIn,
    Contains,
    StartsWith,
    EndsWith,
    Matches,
}

/// Gradual rollout configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    /// Percentage (0-100)
    pub percentage: u8,

    /// Variation to serve to included users
    pub variation: Variation,

    /// Attribute to use for bucketing (default: user_id)
    pub bucket_by: Option<String>,
}

impl Rollout {
    pub fn new(percentage: u8, variation: Variation) -> Self {
        Self {
            percentage: percentage.min(100),
            variation,
            bucket_by: None,
        }
    }

    pub fn with_bucket_by(mut self, attribute: impl Into<String>) -> Self {
        self.bucket_by = Some(attribute.into());
        self
    }

    pub fn evaluate(&self, context: &EvaluationContext, flag_key: &str) -> Option<Variation> {
        // Get bucketing attribute
        let bucket_attr = self.bucket_by.as_deref().unwrap_or("user_id");
        let bucket_value = context.get(bucket_attr)?;

        // Calculate hash bucket (0-99)
        let bucket = Self::calculate_bucket(flag_key, bucket_value);

        // Check if user is in rollout
        if bucket < self.percentage {
            Some(self.variation.clone())
        } else {
            None
        }
    }

    fn calculate_bucket(flag_key: &str, value: &str) -> u8 {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(flag_key.as_bytes());
        hasher.update(value.as_bytes());
        let result = hasher.finalize();

        // Use first byte for bucket (0-255) and map to 0-99
        ((result[0] as u16 * 100) / 256) as u8
    }
}

/// Evaluation context (user attributes)
#[derive(Debug, Clone, Default)]
pub struct EvaluationContext {
    attributes: HashMap<String, String>,
}

impl EvaluationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.attributes
            .insert("user_id".to_string(), user_id.into());
        self
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(|s| s.as_str())
    }

    pub fn user_id(&self) -> Option<&str> {
        self.get("user_id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boolean_flag() {
        let flag = FeatureFlag::boolean("test-flag", true);
        let context = EvaluationContext::new().with_user_id("user-1");

        let result = flag.evaluate(&context);
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_disabled_flag() {
        let mut flag = FeatureFlag::boolean("test-flag", true);
        flag.enabled = false;

        let context = EvaluationContext::new().with_user_id("user-1");
        let result = flag.evaluate(&context);
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_targeting_rule() {
        let rule = TargetingRule::new(Variation::boolean(true)).with_condition(Condition::new(
            "email",
            Operator::EndsWith,
            vec!["@example.com".to_string()],
        ));

        let flag = FeatureFlag::boolean("test-flag", false).with_rule(rule);

        let context = EvaluationContext::new()
            .with_user_id("user-1")
            .with_attribute("email", "user@example.com");

        let result = flag.evaluate(&context);
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_rollout() {
        let rollout = Rollout::new(50, Variation::boolean(true));
        let flag = FeatureFlag::boolean("test-flag", false).with_rollout(rollout);

        // Test multiple users
        let mut enabled_count = 0;
        for i in 0..100 {
            let context = EvaluationContext::new().with_user_id(format!("user-{}", i));
            if flag.evaluate(&context).as_bool() == Some(true) {
                enabled_count += 1;
            }
        }

        // Should be close to 50%
        assert!((40..=60).contains(&enabled_count));
    }

    #[test]
    fn test_multivariate_flag() {
        let flag = FeatureFlag::multivariate(
            "color-scheme",
            vec![
                Variation::string("red"),
                Variation::string("blue"),
                Variation::string("green"),
            ],
        );

        let context = EvaluationContext::new().with_user_id("user-1");
        let result = flag.evaluate(&context);
        assert!(result.as_string().is_some());
    }
}
