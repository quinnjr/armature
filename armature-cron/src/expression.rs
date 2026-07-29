//! Cron expression parsing and evaluation.

use crate::error::{CronError, CronResult};
use chrono::{DateTime, Utc};
use cron::Schedule;
use std::str::FromStr;

/// Parsed cron expression.
#[derive(Debug, Clone)]
pub struct CronExpression {
    schedule: Schedule,
    expression: String,
}

impl CronExpression {
    /// Parse a cron expression.
    ///
    /// Supports standard cron format with 6 fields:
    /// - Second (0-59)
    /// - Minute (0-59)
    /// - Hour (0-23)
    /// - Day of month (1-31)
    /// - Month (1-12)
    /// - Day of week (0-6, Sunday = 0)
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_cron::CronExpression;
    ///
    /// // Every minute
    /// let expr = CronExpression::parse("0 * * * * *").unwrap();
    ///
    /// // Every day at midnight
    /// let expr = CronExpression::parse("0 0 0 * * *").unwrap();
    ///
    /// // Every Monday at 9 AM
    /// let expr = CronExpression::parse("0 0 9 * * MON").unwrap();
    /// ```
    pub fn parse(expression: &str) -> CronResult<Self> {
        let schedule = Schedule::from_str(expression)
            .map_err(|e| CronError::InvalidExpression(format!("{}: {}", expression, e)))?;

        Ok(Self {
            schedule,
            expression: expression.to_string(),
        })
    }

    /// Get the next execution time after the given time.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.schedule.after(&after).next()
    }

    /// Get the next execution time from now.
    pub fn next(&self) -> Option<DateTime<Utc>> {
        self.next_after(Utc::now())
    }

    /// Get the expression string.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Check if the expression would execute at the given time.
    ///
    /// This inspects `time` itself (truncated to whole seconds, since cron fires
    /// on whole-second boundaries) rather than the next upcoming fire from *now*,
    /// so it works for past, present, and future instants.
    pub fn matches(&self, time: DateTime<Utc>) -> bool {
        // Truncate to whole seconds; cron only fires on second boundaries.
        let time = DateTime::from_timestamp(time.timestamp(), 0).unwrap_or(time);
        // `after` is exclusive, so the first fire strictly after `time - 1s` is
        // `time` itself iff `time` is a scheduled fire time.
        self.schedule
            .after(&(time - chrono::Duration::seconds(1)))
            .next()
            == Some(time)
    }
}

/// Common cron expression presets.
pub struct CronPresets;

impl CronPresets {
    /// Every second
    pub const EVERY_SECOND: &'static str = "* * * * * *";

    /// Every minute
    pub const EVERY_MINUTE: &'static str = "0 * * * * *";

    /// Every 5 minutes
    pub const EVERY_5_MINUTES: &'static str = "0 */5 * * * *";

    /// Every 15 minutes
    pub const EVERY_15_MINUTES: &'static str = "0 */15 * * * *";

    /// Every 30 minutes
    pub const EVERY_30_MINUTES: &'static str = "0 */30 * * * *";

    /// Every hour
    pub const EVERY_HOUR: &'static str = "0 0 * * * *";

    /// Every day at midnight
    pub const DAILY: &'static str = "0 0 0 * * *";

    /// Every week on Sunday at midnight
    pub const WEEKLY: &'static str = "0 0 0 * * SUN";

    /// Every month on the 1st at midnight
    pub const MONTHLY: &'static str = "0 0 0 1 * *";

    /// Every year on January 1st at midnight
    pub const YEARLY: &'static str = "0 0 0 1 1 *";

    /// Every weekday (Monday-Friday) at 9 AM
    pub const WEEKDAYS_9AM: &'static str = "0 0 9 * * MON-FRI";

    /// Every weekend (Saturday-Sunday) at 10 AM
    pub const WEEKENDS_10AM: &'static str = "0 0 10 * * SAT,SUN";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_expression() {
        let expr = CronExpression::parse("0 * * * * *");
        assert!(expr.is_ok());
    }

    #[test]
    fn test_parse_invalid_expression() {
        let expr = CronExpression::parse("invalid");
        assert!(expr.is_err());
    }

    #[test]
    fn test_presets() {
        assert!(CronExpression::parse(CronPresets::EVERY_MINUTE).is_ok());
        assert!(CronExpression::parse(CronPresets::DAILY).is_ok());
        assert!(CronExpression::parse(CronPresets::WEEKLY).is_ok());
    }

    #[test]
    fn test_next_execution() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let next = expr.next();
        assert!(next.is_some());
    }

    #[test]
    fn test_preset_every_minute() {
        let expr = CronExpression::parse(CronPresets::EVERY_MINUTE);
        assert!(expr.is_ok());
    }

    #[test]
    fn test_preset_every_hour() {
        let expr = CronExpression::parse(CronPresets::EVERY_HOUR);
        assert!(expr.is_ok());
    }

    #[test]
    fn test_preset_daily() {
        let expr = CronExpression::parse(CronPresets::DAILY);
        assert!(expr.is_ok());
    }

    #[test]
    fn test_preset_weekly() {
        let expr = CronExpression::parse(CronPresets::WEEKLY);
        assert!(expr.is_ok());
    }

    #[test]
    fn test_preset_monthly() {
        let expr = CronExpression::parse(CronPresets::MONTHLY);
        assert!(expr.is_ok());
    }

    #[test]
    fn test_preset_yearly() {
        let expr = CronExpression::parse(CronPresets::YEARLY);
        assert!(expr.is_ok());
    }

    #[test]
    fn test_custom_expression_6_fields() {
        let expr = CronExpression::parse("0 0 0 * * *");
        assert!(expr.is_ok());
    }

    #[test]
    fn test_custom_expression_with_ranges() {
        let expr = CronExpression::parse("0 0-5 0 * * *");
        assert!(expr.is_ok());
    }

    #[test]
    fn test_custom_expression_with_steps() {
        let expr = CronExpression::parse("0 */2 0 * * *");
        assert!(expr.is_ok());
    }

    #[test]
    fn test_custom_expression_with_lists() {
        let expr = CronExpression::parse("0 1,2,3 0 * * *");
        assert!(expr.is_ok());
    }

    #[test]
    fn test_invalid_expression_too_few_fields() {
        let expr = CronExpression::parse("0 * *");
        assert!(expr.is_err());
    }

    #[test]
    fn test_invalid_expression_too_many_fields() {
        let expr = CronExpression::parse("0 * * * * * * *");
        assert!(expr.is_err());
    }

    #[test]
    fn test_invalid_expression_bad_value() {
        let expr = CronExpression::parse("60 * * * *");
        assert!(expr.is_err());
    }

    #[test]
    fn test_expression_clone() {
        let expr1 = CronExpression::parse("0 * * * * *").unwrap();
        let expr2 = expr1.clone();

        let next1 = expr1.next();
        let next2 = expr2.next();

        assert_eq!(next1.is_some(), next2.is_some());
    }

    #[test]
    fn test_next_execution_multiple_calls() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();

        let next1 = expr.next();
        let next2 = expr.next();

        assert!(next1.is_some());
        assert!(next2.is_some());
    }

    #[test]
    fn test_preset_constants_valid() {
        // Just verify all preset constants are valid cron expressions
        let presets = vec![
            CronPresets::EVERY_MINUTE,
            CronPresets::EVERY_HOUR,
            CronPresets::DAILY,
            CronPresets::WEEKLY,
            CronPresets::MONTHLY,
            CronPresets::YEARLY,
        ];

        for preset in presets {
            assert!(CronExpression::parse(preset).is_ok());
        }
    }

    #[test]
    fn test_expression_with_weekday() {
        let expr = CronExpression::parse("0 0 0 * * MON");
        assert!(expr.is_ok());
    }

    #[test]
    fn test_expression_with_month_name() {
        let expr = CronExpression::parse("0 0 0 1 JAN *");
        assert!(expr.is_ok());
    }

    // 2023-01-01 00:00:00 UTC — a whole midnight (multiple of 86400).
    const MIDNIGHT_2023: i64 = 1_672_531_200;

    #[test]
    fn test_matches_returns_true_for_past_firing_time() {
        // Daily at midnight. This instant is in the past yet is a valid fire.
        let expr = CronExpression::parse("0 0 0 * * *").unwrap();
        let midnight = DateTime::from_timestamp(MIDNIGHT_2023, 0).unwrap();
        assert!(
            expr.matches(midnight),
            "a past midnight should match a daily-at-midnight schedule"
        );
    }

    #[test]
    fn test_matches_returns_false_for_non_firing_time() {
        let expr = CronExpression::parse("0 0 0 * * *").unwrap();
        // Noon on the same day is not a fire for a daily-midnight schedule.
        let noon = DateTime::from_timestamp(MIDNIGHT_2023 + 43_200, 0).unwrap();
        assert!(
            !expr.matches(noon),
            "noon should not match a midnight schedule"
        );
    }

    #[test]
    fn test_matches_truncates_subsecond() {
        let expr = CronExpression::parse("0 0 0 * * *").unwrap();
        // Half a second past midnight still counts as the midnight fire.
        let midnight_plus = DateTime::from_timestamp(MIDNIGHT_2023, 500_000_000).unwrap();
        assert!(expr.matches(midnight_plus));
    }

    #[test]
    fn test_matches_every_second_matches_any_whole_second() {
        let expr = CronExpression::parse("* * * * * *").unwrap();
        let t = DateTime::from_timestamp(MIDNIGHT_2023 + 17, 0).unwrap();
        assert!(expr.matches(t));
    }
}
