//! Feature Flag Core
//!
//! Defines feature flags, targeting rules, and evaluation logic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Maximum number of distinct patterns memoized before the cache is dropped
/// and rebuilt.
///
/// Patterns reach this cache from targeting rules, which can be pushed by a
/// remote flag service — i.e. their cardinality is not bounded by anything in
/// this process. A hard cap turns "remote config grows memory forever" into
/// "worst case, we recompile after a burst of novel patterns". Real rule sets
/// are far below this, so steady-state behaviour is an unconditional hit.
const REGEX_CACHE_CAPACITY: usize = 256;

/// Process-wide cache of compiled regexes keyed by pattern string.
///
/// `Operator::Matches` previously recompiled the DFA on every flag evaluation
/// (µs–ms on the request path); memoizing the compiled `Regex` makes repeat
/// evaluations effectively free. `None` memoizes patterns that failed to
/// compile, so an invalid pattern still yields no match — identical to the old
/// `Regex::new(..).map(..).unwrap_or(false)`.
///
/// Process-wide rather than thread-local so the memory cost is paid once
/// instead of once per worker thread.
static REGEX_CACHE: LazyLock<RwLock<HashMap<String, Option<regex::Regex>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Test `value` against `pattern`, compiling the regex once and reusing the
/// cached compilation thereafter. Invalid patterns match nothing.
fn regex_matches(pattern: &str, value: &str) -> bool {
    // A poisoned lock only means some other evaluation panicked mid-update;
    // the map itself is still a valid cache, so recover rather than propagate.
    {
        let cache = REGEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some(compiled) = cache.get(pattern) {
            return compiled.as_ref().is_some_and(|re| re.is_match(value));
        }
    }

    // Compile outside the lock: a pathological pattern must not stall every
    // other evaluation in the process.
    let compiled = regex::Regex::new(pattern).ok();
    let matched = compiled.as_ref().is_some_and(|re| re.is_match(value));

    let mut cache = REGEX_CACHE.write().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= REGEX_CACHE_CAPACITY && !cache.contains_key(pattern) {
        cache.clear();
    }
    cache.insert(pattern.to_string(), compiled);

    matched
}

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

    /// Whether this flag distributes users uniformly across `variations`
    /// (multivariate experiment / A/B test). When `true` and no targeting
    /// rule or rollout applies, each context is deterministically bucketed
    /// into one of the available variations.
    #[serde(default)]
    pub experiment: bool,
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
            experiment: false,
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
            experiment: true,
        }
    }

    /// Enable or disable multivariate experiment bucketing.
    ///
    /// When enabled, and no targeting rule or rollout applies, `evaluate`
    /// deterministically distributes each context across all `variations`.
    pub fn with_experiment(mut self, experiment: bool) -> Self {
        self.experiment = experiment;
        self
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

        // Multivariate experiment: deterministically distribute the context
        // uniformly across all available variations.
        if self.experiment
            && self.variations.len() > 1
            && let Some(variation) = self.bucket_variation(context)
        {
            return variation;
        }

        // Return default variation
        self.default_variation.clone()
    }

    /// Deterministically select one of the flag's `variations` for `context`
    /// by hashing the bucketing attribute across the full variation set.
    ///
    /// Returns `None` when no bucketing attribute is present on the context,
    /// in which case the caller falls back to the default variation.
    fn bucket_variation(&self, context: &EvaluationContext) -> Option<Variation> {
        let n = self.variations.len();
        if n == 0 {
            return None;
        }

        // Prefer an explicit bucketing attribute, else the user id.
        let bucket_value = context
            .get("bucket_by")
            .or_else(|| context.get("user_id"))?;

        // Hash into a wide, uniform space then map to a variation index.
        let hash = Rollout::hash_to_u64(&self.key, bucket_value);
        let index = (hash % n as u64) as usize;
        self.variations.get(index).cloned()
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
            Operator::Matches => attr_value
                .map(|v| self.values.iter().any(|pattern| regex_matches(pattern, v)))
                .unwrap_or(false),
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

    /// Hash `(flag_key, value)` into a uniformly distributed 64-bit value.
    ///
    /// Uses the first 8 bytes of a SHA-256 digest, which gives a far larger
    /// and more uniform space than a single byte before the caller reduces it
    /// modulo the desired number of buckets.
    fn hash_to_u64(flag_key: &str, value: &str) -> u64 {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(flag_key.as_bytes());
        // Separator prevents ("ab", "c") and ("a", "bc") from colliding.
        hasher.update(b":");
        hasher.update(value.as_bytes());
        let result = hasher.finalize();

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&result[..8]);
        u64::from_be_bytes(bytes)
    }

    fn calculate_bucket(flag_key: &str, value: &str) -> u8 {
        // Reduce a wide, uniform hash modulo 100. Using more hash bytes plus a
        // modulo avoids the bias of mapping a single 0-255 byte onto 0-99.
        (Self::hash_to_u64(flag_key, value) % 100) as u8
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

    /// Iterate over all `(name, value)` attribute pairs.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the regex memo was unbounded, so patterns arriving from a
    /// remote flag service could grow it without limit. It must stay capped
    /// while still answering correctly across the overflow.
    #[test]
    fn test_regex_cache_is_bounded() {
        for i in 0..(REGEX_CACHE_CAPACITY * 2) {
            let pattern = format!("^bounded-cache-probe-{i}$");
            assert!(regex_matches(&pattern, &format!("bounded-cache-probe-{i}")));
            assert!(!regex_matches(&pattern, "no-match"));
        }

        let len = REGEX_CACHE.read().unwrap().len();
        assert!(
            len <= REGEX_CACHE_CAPACITY,
            "regex cache grew to {len}, above the {REGEX_CACHE_CAPACITY} cap"
        );
    }

    #[test]
    fn test_invalid_regex_pattern_never_matches() {
        assert!(!regex_matches("([unclosed", "([unclosed"));
    }

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

    /// Regression: `Operator::Matches` previously always returned `false`, so a
    /// `Matches` rule could never fire. It must now match via the regex crate.
    #[test]
    fn test_matches_operator_fires() {
        let rule = TargetingRule::new(Variation::boolean(true)).with_condition(Condition::new(
            "email",
            Operator::Matches,
            vec![r"^[a-z]+@example\.(com|org)$".to_string()],
        ));

        let flag = FeatureFlag::boolean("test-flag", false).with_rule(rule);

        let matching = EvaluationContext::new()
            .with_user_id("u")
            .with_attribute("email", "alice@example.org");
        assert_eq!(flag.evaluate(&matching).as_bool(), Some(true));

        let non_matching = EvaluationContext::new()
            .with_user_id("u")
            .with_attribute("email", "alice@other.net");
        assert_eq!(flag.evaluate(&non_matching).as_bool(), Some(false));
    }

    #[test]
    fn test_matches_condition_directly() {
        let cond = Condition::new(
            "version",
            Operator::Matches,
            vec![r"^v\d+\.\d+$".to_string()],
        );
        let ctx = EvaluationContext::new().with_attribute("version", "v12.4");
        assert!(cond.matches(&ctx));

        let ctx_bad = EvaluationContext::new().with_attribute("version", "12.4");
        assert!(!cond.matches(&ctx_bad));
    }

    #[test]
    fn test_matches_invalid_regex_does_not_match() {
        let cond = Condition::new("x", Operator::Matches, vec!["(".to_string()]);
        let ctx = EvaluationContext::new().with_attribute("x", "anything(");
        assert!(!cond.matches(&ctx));
    }

    #[test]
    fn test_operator_in_and_not_in() {
        let ctx = EvaluationContext::new().with_attribute("plan", "pro");

        let in_cond = Condition::new(
            "plan",
            Operator::In,
            vec!["pro".to_string(), "enterprise".to_string()],
        );
        assert!(in_cond.matches(&ctx));

        let not_in_cond = Condition::new("plan", Operator::NotIn, vec!["free".to_string()]);
        assert!(not_in_cond.matches(&ctx));

        // Missing attribute: In is false, NotIn is true.
        let empty = EvaluationContext::new();
        assert!(!in_cond.matches(&empty));
        assert!(not_in_cond.matches(&empty));
    }

    #[test]
    fn test_operator_contains_startswith_endswith() {
        let ctx = EvaluationContext::new().with_attribute("email", "bob@example.com");

        assert!(
            Condition::new("email", Operator::Contains, vec!["@exam".to_string()]).matches(&ctx)
        );
        assert!(
            Condition::new("email", Operator::StartsWith, vec!["bob".to_string()]).matches(&ctx)
        );
        assert!(
            Condition::new("email", Operator::EndsWith, vec![".com".to_string()]).matches(&ctx)
        );
        assert!(
            !Condition::new("email", Operator::Contains, vec!["zzz".to_string()]).matches(&ctx)
        );
    }

    /// Regression: a multivariate flag previously always returned its default
    /// (first) variation, so every user got the same variant. It must now
    /// distribute users across all variations.
    #[test]
    fn test_multivariate_distributes_across_variants() {
        let flag = FeatureFlag::multivariate(
            "button-color",
            vec![
                Variation::string("red"),
                Variation::string("blue"),
                Variation::string("green"),
            ],
        );

        let mut counts: HashMap<String, usize> = HashMap::new();
        for i in 0..600 {
            let ctx = EvaluationContext::new().with_user_id(format!("user-{i}"));
            let variant = flag.evaluate(&ctx).as_string().unwrap().to_string();
            *counts.entry(variant).or_default() += 1;
        }

        // All three variants must appear (fails against the old code, which
        // only ever returned "red").
        assert_eq!(counts.len(), 3, "expected all 3 variants, got {counts:?}");
        for variant in ["red", "blue", "green"] {
            let c = counts.get(variant).copied().unwrap_or(0);
            // Roughly even: each ~200/600, allow generous slack.
            assert!(
                (100..=300).contains(&c),
                "variant {variant} count {c} outside expected band: {counts:?}"
            );
        }
    }

    #[test]
    fn test_multivariate_is_deterministic() {
        let flag = FeatureFlag::multivariate(
            "layout",
            vec![Variation::string("a"), Variation::string("b")],
        );
        let ctx = EvaluationContext::new().with_user_id("stable-user");
        let first = flag.evaluate(&ctx).as_string().unwrap().to_string();
        let second = flag.evaluate(&ctx).as_string().unwrap().to_string();
        assert_eq!(first, second);
    }

    /// `with_bucket_by` must change *which* context attribute drives bucketing:
    /// with the same rollout, holding `org_id` fixed keeps the outcome stable
    /// even as `user_id` varies, and two contexts differing only in `org_id`
    /// can land in different buckets.
    #[test]
    fn test_rollout_with_bucket_by_changes_bucketing_attribute() {
        let rollout = Rollout::new(50, Variation::boolean(true)).with_bucket_by("org_id");
        assert_eq!(rollout.bucket_by.as_deref(), Some("org_id"));

        let flag = FeatureFlag::boolean("bucket-by-flag", false).with_rollout(rollout);

        // Determinism for a fixed bucket-by value, regardless of user_id.
        let ctx_a = EvaluationContext::new()
            .with_user_id("user-1")
            .with_attribute("org_id", "org-42");
        let ctx_b = EvaluationContext::new()
            .with_user_id("user-99999") // different user...
            .with_attribute("org_id", "org-42"); // ...same org.
        assert_eq!(
            flag.evaluate(&ctx_a).as_bool(),
            flag.evaluate(&ctx_b).as_bool(),
            "org_id-bucketed flag must ignore user_id"
        );

        // Two contexts differing only in the bucket-by attribute must be able to
        // land in different buckets. Scan a handful of org ids to find a pair.
        let mut saw_true = false;
        let mut saw_false = false;
        for i in 0..100 {
            let ctx = EvaluationContext::new()
                .with_user_id("fixed-user") // held constant on purpose
                .with_attribute("org_id", format!("org-{i}"));
            match flag.evaluate(&ctx).as_bool() {
                Some(true) => saw_true = true,
                _ => saw_false = true,
            }
        }
        assert!(
            saw_true && saw_false,
            "varying only org_id must produce both included and excluded buckets"
        );
    }

    /// The default bucket-by (`user_id`) and an explicit `with_bucket_by` on a
    /// different attribute can disagree for the same context, proving the knob
    /// actually redirects which attribute is hashed.
    #[test]
    fn test_rollout_bucket_by_differs_from_default() {
        let ctx = EvaluationContext::new()
            .with_user_id("alice")
            .with_attribute("country", "NZ");

        // Find a rollout percentage where user_id and country buckets disagree.
        let user_bucket = Rollout::calculate_bucket("flag", "alice");
        let country_bucket = Rollout::calculate_bucket("flag", "NZ");
        assert_ne!(
            user_bucket, country_bucket,
            "test precondition: the two attributes hash to different buckets"
        );
        let pct = user_bucket.min(country_bucket) + 1;

        let by_user = FeatureFlag::boolean("flag", false)
            .with_rollout(Rollout::new(pct, Variation::boolean(true)));
        let by_country = FeatureFlag::boolean("flag", false)
            .with_rollout(Rollout::new(pct, Variation::boolean(true)).with_bucket_by("country"));

        // Same context, different bucket-by attribute -> different outcome.
        assert_ne!(
            by_user.evaluate(&ctx).as_bool(),
            by_country.evaluate(&ctx).as_bool()
        );
    }

    #[test]
    fn test_calculate_bucket_uniform_and_in_range() {
        // Bucketing must be (a) in range and (b) *flat* across all 100 buckets.
        //
        // The old mapping folded a single hash byte onto 0..99 via
        // `(byte * 100) / 256`. That is monotonic and still reaches every
        // integer 0..99 — so a mere "all buckets hit" check passes against it
        // too and never detects the ~50% skew it induces (56 buckets receive 3
        // source byte-values, 44 receive only 2, a 3:2 count ratio). The uniform
        // `hash_to_u64 % 100` fix removes that skew. To *discriminate* we bucket
        // a large deterministic sample and assert distribution flatness with a
        // chi-square goodness-of-fit statistic against the uniform expectation.
        const N: usize = 200_000;
        const BUCKETS: usize = 100;

        let mut counts = [0usize; BUCKETS];
        for i in 0..N {
            let b = Rollout::calculate_bucket("flag", &format!("user{i}"));
            assert!((b as usize) < BUCKETS, "bucket {b} out of range");
            counts[b as usize] += 1;
        }

        // Every bucket must be hit (necessary but, alone, insufficient).
        assert!(
            counts.iter().all(|&c| c > 0),
            "some buckets never hit: {counts:?}"
        );

        // Chi-square over 99 degrees of freedom. Expected per bucket = N/100.
        // For a genuinely uniform mapping this lands near 99 (± ~14). The old
        // single-byte mapping produces a chi-square in the thousands (each "3"
        // bucket ~ (3N/256 - N/100)^2 / (N/100) ≈ 59, each "2" bucket ≈ 96, over
        // 100 buckets ≈ 7500). A threshold of 200 (≈7σ above the uniform mean,
        // p ≪ 1e-6) passes the fix comfortably yet fails the biased mapping by
        // ~37×, so the test now measures flatness rather than mere coverage.
        let expected = N as f64 / BUCKETS as f64;
        let chi_square: f64 = counts
            .iter()
            .map(|&c| {
                let diff = c as f64 - expected;
                diff * diff / expected
            })
            .sum();

        assert!(
            chi_square < 200.0,
            "distribution not flat: chi-square {chi_square:.1} exceeds 200 \
             (uniform expects ~99); counts={counts:?}"
        );
    }
}
