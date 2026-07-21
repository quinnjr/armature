//! Pluralization Rules
//!
//! Implements CLDR plural rules for correct pluralization across languages.
//! Different languages have different plural forms - English has 2 (one, other),
//! while Russian has 4 and Arabic has 6.

use crate::{I18nError, Locale, Result};

/// CLDR plural categories.
///
/// Not all languages use all categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    /// Zero items (Arabic)
    Zero,
    /// One item (most languages)
    One,
    /// Two items (Arabic, Welsh)
    Two,
    /// Few items (Slavic languages)
    Few,
    /// Many items (Slavic languages, Arabic)
    Many,
    /// All other cases
    Other,
}

impl PluralCategory {
    /// Parse from string.
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "zero" => Ok(Self::Zero),
            "one" => Ok(Self::One),
            "two" => Ok(Self::Two),
            "few" => Ok(Self::Few),
            "many" => Ok(Self::Many),
            "other" => Ok(Self::Other),
            _ => Err(I18nError::InvalidPluralCategory(s.to_string())),
        }
    }

    /// Convert to string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }
}

impl std::str::FromStr for PluralCategory {
    type Err = I18nError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for PluralCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Plural rules for a specific language.
pub trait PluralRules {
    /// Get the plural category for a number.
    fn category(&self, n: f64) -> PluralCategory;

    /// Get all categories used by this language.
    fn categories(&self) -> &[PluralCategory];
}

/// Get the plural category for a number in a locale.
///
/// # Example
///
/// ```
/// use armature_i18n::{plural_category, PluralCategory, Locale};
///
/// assert_eq!(plural_category(1, &Locale::en()), PluralCategory::One);
/// assert_eq!(plural_category(2, &Locale::en()), PluralCategory::Other);
/// assert_eq!(plural_category(0, &Locale::en()), PluralCategory::Other);
/// ```
pub fn plural_category(n: impl Into<f64>, locale: &Locale) -> PluralCategory {
    let n = n.into();
    let rules = get_plural_rules(&locale.language);
    rules.category(n)
}

/// Get plural rules for a language.
///
/// The rule implementations are zero-sized, stateless unit structs, so we
/// return references to const-promoted `'static` instances rather than
/// heap-allocating a fresh boxed trait object on every call. This keeps
/// [`plural_category`] allocation-free and lets callers cache the result.
fn get_plural_rules(language: &str) -> &'static dyn PluralRules {
    match language {
        // East Asian languages - no plural forms
        "ja" | "ko" | "zh" | "vi" | "th" | "id" | "ms" => &NoPlurals,

        // Romance languages (French special case for 0/1)
        "fr" => &FrenchPlurals,

        // Slavic languages with complex plurals
        "ru" | "uk" | "be" => &RussianPlurals,
        "pl" => &PolishPlurals,
        "cs" | "sk" => &CzechPlurals,

        // Celtic languages
        "cy" => &WelshPlurals,

        // Arabic
        "ar" => &ArabicPlurals,

        // Germanic, Romance (except French), and most others
        _ => &DefaultPlurals,
    }
}

// ============================================================================
// Plural Rule Implementations
// ============================================================================

/// Default pluralization (English-like): 1 = one, else other.
struct DefaultPlurals;

impl PluralRules for DefaultPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        let i = n.abs() as i64;
        if i == 1 && n.fract() == 0.0 {
            PluralCategory::One
        } else {
            PluralCategory::Other
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[PluralCategory::One, PluralCategory::Other]
    }
}

/// No plural forms (Chinese, Japanese, Korean, etc.).
struct NoPlurals;

impl PluralRules for NoPlurals {
    fn category(&self, _n: f64) -> PluralCategory {
        PluralCategory::Other
    }

    fn categories(&self) -> &[PluralCategory] {
        &[PluralCategory::Other]
    }
}

/// French pluralization: 0 and 1 = one, else other.
struct FrenchPlurals;

impl PluralRules for FrenchPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        let i = n.abs() as i64;
        if (i == 0 || i == 1) && n.fract() == 0.0 {
            PluralCategory::One
        } else {
            PluralCategory::Other
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[PluralCategory::One, PluralCategory::Other]
    }
}

/// Russian pluralization (also Ukrainian, Belarusian).
///
/// - one: 1, 21, 31, 41, 51, 61, 71, 81, 101, 1001, ...
/// - few: 2-4, 22-24, 32-34, ...
/// - many: 0, 5-20, 25-30, 35-40, ...
struct RussianPlurals;

impl PluralRules for RussianPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        if n.fract() != 0.0 {
            return PluralCategory::Other;
        }

        let i = n.abs() as i64;
        let mod10 = i % 10;
        let mod100 = i % 100;

        if mod10 == 1 && mod100 != 11 {
            PluralCategory::One
        } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
            PluralCategory::Few
        } else {
            PluralCategory::Many
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[
            PluralCategory::One,
            PluralCategory::Few,
            PluralCategory::Many,
        ]
    }
}

/// Polish pluralization.
///
/// - one: 1
/// - few: 2-4, 22-24, 32-34, ...
/// - many: 0, 5-21, 25-31, ...
struct PolishPlurals;

impl PluralRules for PolishPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        if n.fract() != 0.0 {
            return PluralCategory::Other;
        }

        let i = n.abs() as i64;

        if i == 1 {
            return PluralCategory::One;
        }

        let mod10 = i % 10;
        let mod100 = i % 100;

        if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
            PluralCategory::Few
        } else {
            PluralCategory::Many
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[
            PluralCategory::One,
            PluralCategory::Few,
            PluralCategory::Many,
        ]
    }
}

/// Czech/Slovak pluralization.
///
/// - one: 1
/// - few: 2-4
/// - many: fractions
/// - other: 0, 5+
struct CzechPlurals;

impl PluralRules for CzechPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        if n.fract() != 0.0 {
            return PluralCategory::Many;
        }

        let i = n.abs() as i64;

        match i {
            1 => PluralCategory::One,
            2..=4 => PluralCategory::Few,
            _ => PluralCategory::Other,
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[
            PluralCategory::One,
            PluralCategory::Few,
            PluralCategory::Many,
            PluralCategory::Other,
        ]
    }
}

/// Welsh pluralization.
///
/// - zero: 0
/// - one: 1
/// - two: 2
/// - few: 3
/// - many: 6
/// - other: rest
struct WelshPlurals;

impl PluralRules for WelshPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        if n.fract() != 0.0 {
            return PluralCategory::Other;
        }

        let i = n.abs() as i64;

        match i {
            0 => PluralCategory::Zero,
            1 => PluralCategory::One,
            2 => PluralCategory::Two,
            3 => PluralCategory::Few,
            6 => PluralCategory::Many,
            _ => PluralCategory::Other,
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[
            PluralCategory::Zero,
            PluralCategory::One,
            PluralCategory::Two,
            PluralCategory::Few,
            PluralCategory::Many,
            PluralCategory::Other,
        ]
    }
}

/// Arabic pluralization (most complex).
///
/// - zero: 0
/// - one: 1
/// - two: 2
/// - few: 3-10, 103-110, ...
/// - many: 11-99, 111-199, ...
/// - other: 100-102, 200-202, ...
struct ArabicPlurals;

impl PluralRules for ArabicPlurals {
    fn category(&self, n: f64) -> PluralCategory {
        if n.fract() != 0.0 {
            return PluralCategory::Other;
        }

        let i = n.abs() as i64;
        let mod100 = i % 100;

        match i {
            0 => PluralCategory::Zero,
            1 => PluralCategory::One,
            2 => PluralCategory::Two,
            _ if (3..=10).contains(&mod100) => PluralCategory::Few,
            _ if (11..=99).contains(&mod100) => PluralCategory::Many,
            _ => PluralCategory::Other,
        }
    }

    fn categories(&self) -> &[PluralCategory] {
        &[
            PluralCategory::Zero,
            PluralCategory::One,
            PluralCategory::Two,
            PluralCategory::Few,
            PluralCategory::Many,
            PluralCategory::Other,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_english_plurals() {
        let en = Locale::en();
        assert_eq!(plural_category(0, &en), PluralCategory::Other);
        assert_eq!(plural_category(1, &en), PluralCategory::One);
        assert_eq!(plural_category(2, &en), PluralCategory::Other);
        assert_eq!(plural_category(100, &en), PluralCategory::Other);
    }

    #[test]
    fn test_french_plurals() {
        let fr = Locale::fr();
        assert_eq!(plural_category(0, &fr), PluralCategory::One);
        assert_eq!(plural_category(1, &fr), PluralCategory::One);
        assert_eq!(plural_category(2, &fr), PluralCategory::Other);
    }

    #[test]
    fn test_russian_plurals() {
        let ru = Locale::new("ru", None::<&str>);
        assert_eq!(plural_category(1, &ru), PluralCategory::One);
        assert_eq!(plural_category(2, &ru), PluralCategory::Few);
        assert_eq!(plural_category(5, &ru), PluralCategory::Many);
        assert_eq!(plural_category(11, &ru), PluralCategory::Many);
        assert_eq!(plural_category(21, &ru), PluralCategory::One);
        assert_eq!(plural_category(22, &ru), PluralCategory::Few);
        assert_eq!(plural_category(25, &ru), PluralCategory::Many);
    }

    #[test]
    fn test_japanese_plurals() {
        let ja = Locale::ja();
        assert_eq!(plural_category(0, &ja), PluralCategory::Other);
        assert_eq!(plural_category(1, &ja), PluralCategory::Other);
        assert_eq!(plural_category(100, &ja), PluralCategory::Other);
    }

    #[test]
    fn test_arabic_plurals() {
        let ar = Locale::new("ar", None::<&str>);
        assert_eq!(plural_category(0, &ar), PluralCategory::Zero);
        assert_eq!(plural_category(1, &ar), PluralCategory::One);
        assert_eq!(plural_category(2, &ar), PluralCategory::Two);
        assert_eq!(plural_category(5, &ar), PluralCategory::Few);
        assert_eq!(plural_category(11, &ar), PluralCategory::Many);
        assert_eq!(plural_category(100, &ar), PluralCategory::Other);
    }

    #[test]
    fn test_plural_rules_are_cached() {
        // Regression: `get_plural_rules` must return the same const-promoted
        // 'static instance on every call instead of boxing a fresh trait
        // object (which would yield a distinct heap pointer each time).
        let a = get_plural_rules("en");
        let b = get_plural_rules("en");
        assert!(
            std::ptr::eq(a as *const _ as *const (), b as *const _ as *const ()),
            "plural rules should be cached, not re-allocated per call"
        );

        let fr1 = get_plural_rules("fr");
        let fr2 = get_plural_rules("fr");
        assert!(std::ptr::eq(
            fr1 as *const _ as *const (),
            fr2 as *const _ as *const ()
        ));
    }

    #[test]
    fn test_plural_category_parse() {
        assert_eq!(PluralCategory::parse("one").unwrap(), PluralCategory::One);
        assert_eq!(
            PluralCategory::parse("OTHER").unwrap(),
            PluralCategory::Other
        );
        assert!(PluralCategory::parse("invalid").is_err());
    }
}
