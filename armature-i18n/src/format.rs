//! Date and Number Formatting
//!
//! Provides locale-aware formatting for numbers, dates, and currencies.

use crate::Locale;

/// Date formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateStyle {
    /// Full date (e.g., "Monday, January 1, 2024")
    Full,
    /// Long date (e.g., "January 1, 2024")
    Long,
    /// Medium date (e.g., "Jan 1, 2024")
    #[default]
    Medium,
    /// Short date (e.g., "1/1/24")
    Short,
}

/// Time formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeStyle {
    /// Full time with timezone (e.g., "12:00:00 PM Eastern Standard Time")
    Full,
    /// Long time with timezone abbrev (e.g., "12:00:00 PM EST")
    Long,
    /// Medium time (e.g., "12:00:00 PM")
    #[default]
    Medium,
    /// Short time (e.g., "12:00 PM")
    Short,
}

// ============================================================================
// Number Formatting
// ============================================================================

/// Number formatting configuration.
#[derive(Debug, Clone)]
pub struct NumberFormatter {
    /// Minimum integer digits
    pub min_integer_digits: usize,
    /// Minimum fraction digits
    pub min_fraction_digits: usize,
    /// Maximum fraction digits
    pub max_fraction_digits: usize,
    /// Use grouping separators
    pub use_grouping: bool,
}

impl Default for NumberFormatter {
    fn default() -> Self {
        Self {
            min_integer_digits: 1,
            min_fraction_digits: 0,
            max_fraction_digits: 3,
            use_grouping: true,
        }
    }
}

impl NumberFormatter {
    /// Create a new number formatter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set minimum integer digits (zero-pads the integer part).
    pub fn min_integer_digits(mut self, digits: usize) -> Self {
        self.min_integer_digits = digits;
        self
    }

    /// Set minimum fraction digits.
    pub fn min_fraction_digits(mut self, digits: usize) -> Self {
        self.min_fraction_digits = digits;
        self
    }

    /// Set maximum fraction digits.
    pub fn max_fraction_digits(mut self, digits: usize) -> Self {
        self.max_fraction_digits = digits;
        self
    }

    /// Set whether to use grouping separators.
    pub fn use_grouping(mut self, use_grouping: bool) -> Self {
        self.use_grouping = use_grouping;
        self
    }

    /// Format a number for the given locale.
    ///
    /// Fraction handling: the value is rounded to `max_fraction_digits`, then
    /// trailing zeros are trimmed down to `min_fraction_digits`. So with the
    /// defaults (`min = 0`, `max = 3`), `1.5` renders as `"1.5"` (not
    /// `"1.50"`), while `1.0` renders as `"1"`. To force trailing zeros (e.g.
    /// currency cents) raise `min_fraction_digits` — with `min = max = 2`,
    /// `1.5` renders as `"1.50"` and `1.0` as `"1.00"`.
    ///
    /// The integer part is zero-padded on the left to `min_integer_digits`.
    pub fn format(&self, n: f64, locale: &Locale) -> String {
        let (decimal_sep, group_sep) = get_number_separators(locale);

        // Preserve the sign ourselves so grouping/padding operate on digits.
        let negative = n.is_sign_negative() && n != 0.0;
        let abs = n.abs();

        // Round to the maximum precision (never below the minimum).
        let precision = self.max_fraction_digits.max(self.min_fraction_digits);
        let formatted = format!("{:.*}", precision, abs);

        let (integer_part, frac_full) = match formatted.split_once('.') {
            Some((i, f)) => (i.to_string(), f.to_string()),
            None => (formatted, String::new()),
        };

        // Trim trailing zeros from the fraction, but keep at least
        // `min_fraction_digits` digits.
        let mut frac: Vec<char> = frac_full.chars().collect();
        while frac.len() > self.min_fraction_digits && frac.last() == Some(&'0') {
            frac.pop();
        }
        let fraction: String = frac.into_iter().collect();

        // Zero-pad the integer part to the requested minimum width.
        let integer_part = if integer_part.len() < self.min_integer_digits {
            let pad = self.min_integer_digits - integer_part.len();
            format!("{}{}", "0".repeat(pad), integer_part)
        } else {
            integer_part
        };

        // Add grouping separators to the (padded) integer part.
        let grouped_integer = if self.use_grouping {
            add_grouping(&integer_part, group_sep)
        } else {
            integer_part
        };

        let mut result = String::new();
        if negative {
            result.push('-');
        }
        result.push_str(&grouped_integer);
        if !fraction.is_empty() {
            result.push_str(decimal_sep);
            result.push_str(&fraction);
        }
        result
    }
}

/// Format a number for a locale.
///
/// # Example
///
/// ```
/// use armature_i18n::{format_number, Locale};
///
/// assert_eq!(format_number(1234567.89, &Locale::en_us()), "1,234,567.89");
/// assert_eq!(format_number(1234567.89, &Locale::de_de()), "1.234.567,89");
/// assert_eq!(format_number(1234567.89, &Locale::fr_fr()), "1 234 567,89");
/// ```
pub fn format_number(n: f64, locale: &Locale) -> String {
    NumberFormatter::default()
        .max_fraction_digits(2)
        .format(n, locale)
}

/// Format a percentage for a locale.
///
/// # Example
///
/// ```
/// use armature_i18n::{format_percent, Locale};
///
/// assert_eq!(format_percent(0.75, &Locale::en_us()), "75%");
/// assert_eq!(format_percent(0.125, &Locale::de_de()), "12,5%");
/// ```
pub fn format_percent(n: f64, locale: &Locale) -> String {
    let value = n * 100.0;
    let formatted = NumberFormatter::new()
        .max_fraction_digits(1)
        .format(value, locale);
    format!("{}%", formatted)
}

// ============================================================================
// Currency Formatting
// ============================================================================

/// Currency formatting configuration.
#[derive(Debug, Clone)]
pub struct CurrencyFormatter {
    /// Currency code (ISO 4217)
    pub currency_code: String,
    /// Show currency symbol instead of code
    pub use_symbol: bool,
}

impl CurrencyFormatter {
    /// Create a new currency formatter.
    pub fn new(currency_code: impl Into<String>) -> Self {
        Self {
            currency_code: currency_code.into().to_uppercase(),
            use_symbol: true,
        }
    }

    /// Set whether to use symbol.
    pub fn use_symbol(mut self, use_symbol: bool) -> Self {
        self.use_symbol = use_symbol;
        self
    }

    /// Format a currency amount.
    pub fn format(&self, amount: f64, locale: &Locale) -> String {
        let (symbol, before) = get_currency_symbol(&self.currency_code, locale);

        let formatted = NumberFormatter::new()
            .min_fraction_digits(2)
            .max_fraction_digits(2)
            .format(amount.abs(), locale);

        let sign = if amount < 0.0 { "-" } else { "" };

        if self.use_symbol {
            if before {
                format!("{}{}{}", sign, symbol, formatted)
            } else {
                format!("{}{} {}", sign, formatted, symbol)
            }
        } else {
            format!("{}{} {}", sign, formatted, self.currency_code)
        }
    }
}

/// Format a currency amount for a locale.
///
/// # Example
///
/// ```
/// use armature_i18n::{format_currency, Locale};
///
/// assert_eq!(format_currency(99.99, "USD", &Locale::en_us()), "$99.99");
/// assert_eq!(format_currency(99.99, "EUR", &Locale::de_de()), "99,99 €");
/// assert_eq!(format_currency(99.99, "GBP", &Locale::en_gb()), "£99.99");
/// ```
pub fn format_currency(amount: f64, currency_code: &str, locale: &Locale) -> String {
    CurrencyFormatter::new(currency_code).format(amount, locale)
}

// ============================================================================
// Date Formatting
// ============================================================================

/// A timezone descriptor used by the `Full`/`Long` time styles.
///
/// Formatting is calendar-only (it does not shift the wall-clock time), so the
/// timezone is supplied explicitly. `Full` appends the [`name`](Self::name)
/// (e.g. "Eastern Standard Time") and `Long` appends the
/// [`abbreviation`](Self::abbreviation) (e.g. "EST").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeZone {
    /// Full display name, e.g. "Eastern Standard Time".
    pub name: String,
    /// Short abbreviation, e.g. "EST".
    pub abbreviation: String,
}

impl TimeZone {
    /// Create a timezone from a full name and its abbreviation.
    pub fn new(name: impl Into<String>, abbreviation: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            abbreviation: abbreviation.into(),
        }
    }

    /// Coordinated Universal Time.
    pub fn utc() -> Self {
        Self::new("Coordinated Universal Time", "UTC")
    }
}

/// Date formatting configuration.
#[derive(Debug, Clone, Default)]
pub struct DateFormatter {
    /// Date style
    pub date_style: Option<DateStyle>,
    /// Time style
    pub time_style: Option<TimeStyle>,
    /// Timezone applied by the `Full`/`Long` time styles.
    pub timezone: Option<TimeZone>,
}

impl DateFormatter {
    /// Create a new date formatter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set date style.
    pub fn date_style(mut self, style: DateStyle) -> Self {
        self.date_style = Some(style);
        self
    }

    /// Set time style.
    pub fn time_style(mut self, style: TimeStyle) -> Self {
        self.time_style = Some(style);
        self
    }

    /// Set the timezone shown by the `Full`/`Long` time styles.
    pub fn timezone(mut self, tz: TimeZone) -> Self {
        self.timezone = Some(tz);
        self
    }

    /// Format a date (year, month, day).
    pub fn format_date(&self, year: i32, month: u32, day: u32, locale: &Locale) -> String {
        let style = self.date_style.unwrap_or(DateStyle::Medium);
        format_date_impl(year, month, day, style, locale)
    }

    /// Format a time (hour, minute, second).
    pub fn format_time(&self, hour: u32, minute: u32, second: u32, locale: &Locale) -> String {
        let style = self.time_style.unwrap_or(TimeStyle::Medium);
        format_time_impl(hour, minute, second, style, self.timezone.as_ref(), locale)
    }
}

/// Format a date for a locale.
///
/// # Example
///
/// ```
/// use armature_i18n::{format_date, DateStyle, Locale};
///
/// // Default medium style
/// let date = format_date(2024, 1, 15, &Locale::en_us());
/// assert!(date.contains("Jan") && date.contains("15") && date.contains("2024"));
/// ```
pub fn format_date(year: i32, month: u32, day: u32, locale: &Locale) -> String {
    format_date_impl(year, month, day, DateStyle::Medium, locale)
}

fn format_date_impl(year: i32, month: u32, day: u32, style: DateStyle, locale: &Locale) -> String {
    let month_idx = (month.saturating_sub(1) as usize).min(11);
    let (month_names_short, month_names_long) = month_names(locale);

    // Determine date order based on locale
    let is_dmy = matches!(locale.language.as_str(), "en" if locale.region.as_deref() == Some("GB"))
        || matches!(
            locale.language.as_str(),
            "fr" | "de" | "es" | "it" | "pt" | "ru" | "pl"
        );
    let is_ymd = matches!(locale.language.as_str(), "ja" | "zh" | "ko");

    match style {
        DateStyle::Full => {
            // Full includes the weekday, computed from the proleptic
            // Gregorian calendar via Sakamoto's algorithm.
            let weekday = weekday_name(year, month, day, locale);
            format!(
                "{}, {} {}, {}",
                weekday, month_names_long[month_idx], day, year
            )
        }
        DateStyle::Long => {
            format!("{} {}, {}", month_names_long[month_idx], day, year)
        }
        DateStyle::Medium => {
            if is_ymd {
                format!("{}/{}/{}", year, month, day)
            } else if is_dmy {
                format!("{} {} {}", day, month_names_short[month_idx], year)
            } else {
                format!("{} {}, {}", month_names_short[month_idx], day, year)
            }
        }
        DateStyle::Short => {
            if is_ymd {
                format!("{}/{}/{}", year % 100, month, day)
            } else if is_dmy {
                format!("{}/{}/{}", day, month, year % 100)
            } else {
                format!("{}/{}/{}", month, day, year % 100)
            }
        }
    }
}

fn format_time_impl(
    hour: u32,
    minute: u32,
    second: u32,
    style: TimeStyle,
    timezone: Option<&TimeZone>,
    locale: &Locale,
) -> String {
    // Determine if 12-hour format
    let use_12h = matches!(locale.language.as_str(), "en");

    // Base HH:MM:SS (with AM/PM for English) shared by Full/Long/Medium.
    let base = if use_12h {
        let (h, period) = if hour == 0 {
            (12, "AM")
        } else if hour < 12 {
            (hour, "AM")
        } else if hour == 12 {
            (12, "PM")
        } else {
            (hour - 12, "PM")
        };
        format!("{}:{:02}:{:02} {}", h, minute, second, period)
    } else {
        format!("{:02}:{:02}:{:02}", hour, minute, second)
    };

    match style {
        // Full appends the full timezone name; Long appends its abbreviation.
        // With no timezone supplied both degrade to the Medium rendering.
        TimeStyle::Full => match timezone {
            Some(tz) => format!("{} {}", base, tz.name),
            None => base,
        },
        TimeStyle::Long => match timezone {
            Some(tz) => format!("{} {}", base, tz.abbreviation),
            None => base,
        },
        TimeStyle::Medium => {
            if use_12h {
                let (h, period) = if hour == 0 {
                    (12, "AM")
                } else if hour < 12 {
                    (hour, "AM")
                } else if hour == 12 {
                    (12, "PM")
                } else {
                    (hour - 12, "PM")
                };
                format!("{}:{:02}:{:02} {}", h, minute, second, period)
            } else {
                format!("{:02}:{:02}:{:02}", hour, minute, second)
            }
        }
        TimeStyle::Short => {
            if use_12h {
                let (h, period) = if hour == 0 {
                    (12, "AM")
                } else if hour < 12 {
                    (hour, "AM")
                } else if hour == 12 {
                    (12, "PM")
                } else {
                    (hour - 12, "PM")
                };
                format!("{}:{:02} {}", h, minute, period)
            } else {
                format!("{:02}:{:02}", hour, minute)
            }
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get decimal and grouping separators for a locale.
fn get_number_separators(locale: &Locale) -> (&'static str, &'static str) {
    match locale.language.as_str() {
        // Comma decimal, period grouping
        "de" | "es" | "it" | "pt" | "nl" | "da" | "sv" | "no" | "fi" | "pl" | "cs" | "sk"
        | "hu" | "ro" | "bg" | "el" | "ru" | "uk" | "tr" | "id" | "vi" => (",", "."),

        // Comma decimal, space grouping (French-speaking)
        "fr" => (",", " "),

        // Period decimal, comma grouping (default English-like)
        _ => (".", ","),
    }
}

/// Add grouping separators to an integer string.
fn add_grouping(s: &str, sep: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    if len <= 3 {
        return s.to_string();
    }

    let mut result = String::with_capacity(len + (len - 1) / 3);

    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push_str(sep);
        }
        result.push(*c);
    }

    result
}

/// Get currency symbol and position for a locale.
fn get_currency_symbol(currency_code: &str, locale: &Locale) -> (String, bool) {
    // Symbol before amount (English-style)
    let symbol_before = !matches!(
        locale.language.as_str(),
        "de" | "fr"
            | "es"
            | "it"
            | "pt"
            | "nl"
            | "da"
            | "sv"
            | "no"
            | "fi"
            | "pl"
            | "cs"
            | "sk"
            | "hu"
            | "ro"
            | "bg"
            | "el"
            | "ru"
            | "uk"
            | "vi"
    );

    let symbol = match currency_code {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        "CNY" => "¥",
        "KRW" => "₩",
        "INR" => "₹",
        "RUB" => "₽",
        "BRL" => "R$",
        "CHF" => "CHF",
        "CAD" => "CA$",
        "AUD" => "A$",
        "HKD" => "HK$",
        "SGD" => "S$",
        "SEK" => "kr",
        "NOK" => "kr",
        "DKK" => "kr",
        "PLN" => "zł",
        "CZK" => "Kč",
        "MXN" => "MX$",
        "THB" => "฿",
        "TWD" => "NT$",
        _ => currency_code,
    };

    (symbol.to_string(), symbol_before)
}

/// Locale-aware (short, long) month name tables.
///
/// Falls back to English for languages without a bundled table.
fn month_names(locale: &Locale) -> (&'static [&'static str; 12], &'static [&'static str; 12]) {
    const EN_SHORT: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const EN_LONG: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const FR_SHORT: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    const FR_LONG: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    const DE_SHORT: [&str; 12] = [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.",
        "Dez.",
    ];
    const DE_LONG: [&str; 12] = [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ];
    const ES_SHORT: [&str; 12] = [
        "ene.", "feb.", "mar.", "abr.", "may.", "jun.", "jul.", "ago.", "sept.", "oct.", "nov.",
        "dic.",
    ];
    const ES_LONG: [&str; 12] = [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ];

    match locale.language.as_str() {
        "fr" => (&FR_SHORT, &FR_LONG),
        "de" => (&DE_SHORT, &DE_LONG),
        "es" => (&ES_SHORT, &ES_LONG),
        _ => (&EN_SHORT, &EN_LONG),
    }
}

/// Compute the weekday name for a proleptic-Gregorian date.
///
/// Uses Sakamoto's algorithm; `dow` is 0 = Sunday .. 6 = Saturday.
fn weekday_name(year: i32, month: u32, day: u32, locale: &Locale) -> &'static str {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let m = (month.clamp(1, 12)) as usize;
    let y = if m < 3 { year - 1 } else { year };
    let dow = (y + y / 4 - y / 100 + y / 400 + T[m - 1] + day as i32).rem_euclid(7) as usize;

    const EN: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const FR: [&str; 7] = [
        "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
    ];
    const DE: [&str; 7] = [
        "Sonntag",
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
    ];
    const ES: [&str; 7] = [
        "domingo",
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
    ];

    let table = match locale.language.as_str() {
        "fr" => &FR,
        "de" => &DE,
        "es" => &ES,
        _ => &EN,
    };
    table[dow]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number_us() {
        let locale = Locale::en_us();
        assert_eq!(format_number(1234567.89, &locale), "1,234,567.89");
        assert_eq!(format_number(1000.0, &locale), "1,000");
    }

    #[test]
    fn test_format_number_german() {
        let locale = Locale::de_de();
        assert_eq!(format_number(1234567.89, &locale), "1.234.567,89");
    }

    #[test]
    fn test_format_number_french() {
        let locale = Locale::fr_fr();
        assert_eq!(format_number(1234567.89, &locale), "1 234 567,89");
    }

    #[test]
    fn test_format_percent() {
        assert_eq!(format_percent(0.75, &Locale::en_us()), "75%");
        assert_eq!(format_percent(0.125, &Locale::de_de()), "12,5%");
    }

    #[test]
    fn test_format_currency() {
        assert_eq!(format_currency(99.99, "USD", &Locale::en_us()), "$99.99");
        assert_eq!(format_currency(99.99, "EUR", &Locale::de_de()), "99,99 €");
        assert_eq!(format_currency(99.99, "GBP", &Locale::en_gb()), "£99.99");
    }

    #[test]
    fn test_format_date() {
        let us = Locale::en_us();
        let date = format_date(2024, 1, 15, &us);
        assert!(date.contains("Jan"));
        assert!(date.contains("15"));
        assert!(date.contains("2024"));
    }

    #[test]
    fn test_format_date_short() {
        let us = Locale::en_us();
        let date = DateFormatter::new()
            .date_style(DateStyle::Short)
            .format_date(2024, 1, 15, &us);
        assert_eq!(date, "1/15/24");

        let gb = Locale::en_gb();
        let date = DateFormatter::new()
            .date_style(DateStyle::Short)
            .format_date(2024, 1, 15, &gb);
        assert_eq!(date, "15/1/24");
    }

    #[test]
    fn test_format_time() {
        let us = Locale::en_us();
        let time = DateFormatter::new()
            .time_style(TimeStyle::Short)
            .format_time(14, 30, 0, &us);
        assert_eq!(time, "2:30 PM");

        let de = Locale::de_de();
        let time = DateFormatter::new()
            .time_style(TimeStyle::Short)
            .format_time(14, 30, 0, &de);
        assert_eq!(time, "14:30");
    }

    #[test]
    fn test_min_integer_digits_zero_pads() {
        // Regression: `min_integer_digits` was never applied.
        let f = NumberFormatter::new()
            .min_integer_digits(3)
            .max_fraction_digits(0)
            .use_grouping(false);
        assert_eq!(f.format(5.0, &Locale::en_us()), "005");
        assert_eq!(f.format(42.0, &Locale::en_us()), "042");
        assert_eq!(f.format(1234.0, &Locale::en_us()), "1234");
    }

    #[test]
    fn test_max_fraction_digits_trims_trailing_zeros() {
        // Regression: 1.5 formatted "1.50" instead of "1.5".
        let f = NumberFormatter::new().max_fraction_digits(2);
        assert_eq!(f.format(1.5, &Locale::en_us()), "1.5");
        assert_eq!(f.format(1.0, &Locale::en_us()), "1");
        assert_eq!(f.format(1.25, &Locale::en_us()), "1.25");

        // With min == max, trailing zeros are intentionally kept.
        let padded = NumberFormatter::new()
            .min_fraction_digits(2)
            .max_fraction_digits(2);
        assert_eq!(padded.format(1.5, &Locale::en_us()), "1.50");
        assert_eq!(padded.format(1.0, &Locale::en_us()), "1.00");
    }

    #[test]
    fn test_time_full_long_include_timezone() {
        // Regression: Full/Long were identical to Medium (no timezone).
        let us = Locale::en_us();
        let tz = TimeZone::new("Eastern Standard Time", "EST");

        let full = DateFormatter::new()
            .time_style(TimeStyle::Full)
            .timezone(tz.clone())
            .format_time(12, 0, 0, &us);
        assert_eq!(full, "12:00:00 PM Eastern Standard Time");

        let long = DateFormatter::new()
            .time_style(TimeStyle::Long)
            .timezone(tz)
            .format_time(12, 0, 0, &us);
        assert_eq!(long, "12:00:00 PM EST");

        let medium = DateFormatter::new()
            .time_style(TimeStyle::Medium)
            .format_time(12, 0, 0, &us);
        assert_ne!(full, medium);
        assert_ne!(long, medium);
    }

    #[test]
    fn test_date_full_includes_weekday() {
        // Regression: DateStyle::Full omitted the weekday (TODO stub).
        let us = Locale::en_us();
        let date = DateFormatter::new()
            .date_style(DateStyle::Full)
            .format_date(2024, 1, 15, &us);
        // 2024-01-15 is a Monday.
        assert!(date.starts_with("Monday, "), "got {date:?}");
        assert!(date.contains("January 15, 2024"));
    }

    #[test]
    fn test_month_names_are_locale_aware() {
        // Regression: month names were hardcoded English for all locales.
        let fr = Locale::fr_fr();
        let date = DateFormatter::new()
            .date_style(DateStyle::Long)
            .format_date(2024, 1, 15, &fr);
        assert!(date.contains("janvier"), "got {date:?}");

        let de = Locale::de_de();
        let date = DateFormatter::new()
            .date_style(DateStyle::Long)
            .format_date(2024, 3, 1, &de);
        assert!(date.contains("März"), "got {date:?}");
    }

    #[test]
    fn test_add_grouping() {
        assert_eq!(add_grouping("1234567", ","), "1,234,567");
        assert_eq!(add_grouping("123", ","), "123");
        assert_eq!(add_grouping("1234", " "), "1 234");
    }
}
