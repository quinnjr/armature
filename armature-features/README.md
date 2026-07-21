# armature-features

Feature flags and A/B testing for the Armature framework.

## Features

- **Feature Flags** - Toggle features on/off at runtime
- **Targeting Rules** - Activate features for users matching conditions
- **Gradual Rollout** - Deterministic percentage-based rollouts
- **A/B Testing** - Multivariate flags that distribute users across variations
- **LaunchDarkly** - Optional integration behind the `launchdarkly` feature

## Installation

```toml
[dependencies]
armature-features = "0.1"
```

## Quick Start

The core type is [`FeatureFlag`]. You build a flag, then call `evaluate` with an
`EvaluationContext` describing the current user; it returns a `Variation`.

```rust
use armature_features::{FeatureFlag, EvaluationContext};

// A simple boolean flag with a default value of `false`.
let flag = FeatureFlag::boolean("new-checkout", false);

let context = EvaluationContext::new()
    .with_user_id("user-123")
    .with_attribute("email", "alice@example.com");

let enabled = flag.evaluate(&context).as_bool().unwrap_or(false);
if enabled {
    // New checkout flow
}
```

## Targeting Rules

A rule serves its variation when **all** of its conditions match. Conditions
support the `In`, `NotIn`, `Contains`, `StartsWith`, `EndsWith`, and `Matches`
(regex) operators.

```rust
use armature_features::{FeatureFlag, TargetingRule, Condition, Operator, Variation, EvaluationContext};

let rule = TargetingRule::new(Variation::boolean(true))
    .with_condition(Condition::new(
        "email",
        Operator::EndsWith,
        vec!["@company.com".to_string()],
    ));

let flag = FeatureFlag::boolean("beta-feature", false).with_rule(rule);

let ctx = EvaluationContext::new()
    .with_user_id("u1")
    .with_attribute("email", "dev@company.com");
assert_eq!(flag.evaluate(&ctx).as_bool(), Some(true));
```

Use `Operator::Matches` for regex conditions:

```rust
use armature_features::{Condition, Operator};

let cond = Condition::new(
    "version",
    Operator::Matches,
    vec![r"^v\d+\.\d+$".to_string()],
);
```

## Gradual Rollout

`Rollout` deterministically buckets users (by `user_id`, or a custom attribute
via `with_bucket_by`) into a percentage band, so the same user always gets the
same answer.

```rust
use armature_features::{FeatureFlag, Rollout, Variation};

// Roll out to 25% of users.
let rollout = Rollout::new(25, Variation::boolean(true));
let flag = FeatureFlag::boolean("new-algorithm", false).with_rollout(rollout);
```

## A/B Testing (Multivariate)

A multivariate flag distributes users uniformly across all of its variations.

```rust
use armature_features::{FeatureFlag, Variation, EvaluationContext};

let flag = FeatureFlag::multivariate(
    "button-color",
    vec![
        Variation::string("red"),
        Variation::string("blue"),
        Variation::string("green"),
    ],
);

let ctx = EvaluationContext::new().with_user_id("user-123");
let color = flag.evaluate(&ctx).as_string().unwrap_or("red").to_string();
```

## LaunchDarkly (optional)

Enable the `launchdarkly` feature to evaluate flags managed in LaunchDarkly
through the same `EvaluationContext`/`Variation` types:

```toml
[dependencies]
armature-features = { version = "0.1", features = ["launchdarkly"] }
```

```rust,ignore
use armature_features::{EvaluationContext, Variation};
use armature_features::launchdarkly::LaunchDarklyProvider;

let provider = LaunchDarklyProvider::new("sdk-key-123")?;
provider.start(); // begin streaming flag data

let ctx = EvaluationContext::new().with_user_id("user-42");
let variation = provider.evaluate("new-checkout", &ctx, &Variation::boolean(false))?;
let enabled = variation.as_bool().unwrap_or(false);
# Ok::<(), armature_features::launchdarkly::LaunchDarklyError>(())
```

## License

MIT OR Apache-2.0
