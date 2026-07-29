//! Internationalization (i18n) Support for Armature
//!
//! Provides comprehensive internationalization features:
//!
//! - **Message Translation**: Load and format localized messages
//! - **Locale Detection**: Parse Accept-Language headers
//! - **Pluralization**: Handle plural forms correctly per locale
//! - **Date/Number Formatting**: Locale-aware formatting
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use armature_i18n::{I18n, Locale};
//!
//! // Create i18n instance
//! let i18n = I18n::new()
//!     .with_default_locale(Locale::en_us())
//!     .with_fallback(Locale::en())
//!     .load_from_dir("locales/")?;
//!
//! // Get translation
//! let msg = i18n.t("hello", &Locale::es());  // "¡Hola!"
//!
//! // With arguments
//! let msg = i18n.t_args("greeting", &Locale::fr(), &[("name", "Alice")]);
//! // "Bonjour, Alice!"
//!
//! // Pluralization
//! let msg = i18n.t_plural("items", 5, &Locale::en());  // "5 items"
//! ```
//!
//! # Accept-Language Parsing
//!
//! ```rust,ignore
//! use armature_i18n::locale::parse_accept_language;
//!
//! let header = "en-US,en;q=0.9,fr;q=0.8";
//! let locales = parse_accept_language(header);
//! // [Locale::en_us(), Locale::en(), Locale::fr()]
//! ```
//!
//! # Date/Number Formatting
//!
//! ```rust,ignore
//! use armature_i18n::format::{format_number, format_date, format_currency};
//!
//! let locale = Locale::de_de();
//! format_number(1234567.89, &locale);  // "1.234.567,89"
//! format_currency(99.99, "EUR", &locale);  // "99,99 €"
//! ```

mod bundle;
mod error;
mod format;
mod locale;
mod messages;
mod plural;

pub use bundle::FluentBundle;
pub use error::I18nError;
pub use format::{
    CurrencyFormatter, DateFormatter, DateStyle, NumberFormatter, TimeStyle, TimeZone,
    format_currency, format_date, format_number, format_percent,
};
pub use locale::{Locale, LocaleBuilder, negotiate_locale, parse_accept_language};
pub use messages::{I18n, MessageBundle, Messages, TranslationSource};
pub use plural::{PluralCategory, PluralRules, plural_category};

/// Result type for i18n operations
pub type Result<T> = std::result::Result<T, I18nError>;

/// Prelude for common imports
pub mod prelude {
    pub use crate::{
        I18n, I18nError, Locale, LocaleBuilder, PluralCategory, Result, format_currency,
        format_date, format_number, negotiate_locale, parse_accept_language, plural_category,
    };
}
