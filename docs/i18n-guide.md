# Internationalization (i18n) Guide

The `armature-i18n` crate provides comprehensive internationalization support for building multilingual applications.

## Features

- ✅ **Message Translation** - JSON and Fluent message formats
- ✅ **Locale Detection** - Accept-Language header parsing
- ✅ **Pluralization** - CLDR plural rules for 10+ language families
- ✅ **Date/Number Formatting** - Locale-aware formatting

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-i18n = "0.1"
```

### Feature Flags

```toml
# Default: simple JSON-based translation
armature-i18n = "0.1"

# Mozilla Fluent support (advanced formatting)
armature-i18n = { version = "0.1", features = ["fluent"] }

# ICU-based formatting (most accurate)
armature-i18n = { version = "0.1", features = ["icu"] }

# All features
armature-i18n = { version = "0.1", features = ["full"] }
```

## Quick Start

```rust
use armature_i18n::prelude::*;

// Create i18n instance
let i18n = I18n::new()
    .with_default_locale(Locale::en_us())
    .with_fallback(Locale::en())
    .load_from_dir("locales/")?;

// Simple translation
let msg = i18n.t("hello", &Locale::es());
// "¡Hola!"

// With arguments
let msg = i18n.t_args("greeting", &Locale::fr(), &[("name", "Alice")]);
// "Bonjour, Alice!"

// Pluralization
let msg = i18n.t_plural("items", 5, &Locale::en());
// "5 items"
```

## Locales

### Creating Locales

```rust
use armature_i18n::Locale;
use std::str::FromStr;

// From components
let en = Locale::new("en", None::<&str>);
let en_us = Locale::new("en", Some("US"));

// From BCP 47 tag
let fr_fr = Locale::parse("fr-FR")?;
let zh_hans = Locale::parse("zh-Hans-CN")?;

// Using FromStr
let de: Locale = "de-DE".parse()?;

// Pre-defined locales
let ja = Locale::ja();
let zh_cn = Locale::zh_cn();
```

### Locale Builder

```rust
use armature_i18n::LocaleBuilder;

let locale = LocaleBuilder::new()
    .language("zh")
    .script("Hans")
    .region("CN")
    .build()?;

assert_eq!(locale.tag(), "zh-Hans-CN");
```

## Accept-Language Parsing

Parse and negotiate locales from HTTP headers:

```rust
use armature_i18n::{parse_accept_language, negotiate_locale, Locale};

// Parse Accept-Language header
let header = "en-US,en;q=0.9,fr;q=0.8,de;q=0.7";
let requested = parse_accept_language(header);
// [en-US, en, fr, de] sorted by quality

// Negotiate best match
let available = vec![Locale::en_us(), Locale::fr_fr(), Locale::de_de()];
let default = Locale::en_us();

let best = negotiate_locale(&requested, &available, &default);
// Returns &Locale::en_us() (exact match)
```

### Match Scoring

```rust
let en_us = Locale::en_us();
let en = Locale::en();

// Exact match = 100
assert_eq!(en_us.match_score(&en_us), 100);

// Language match = 10+
assert!(en_us.match_score(&en) > 0);

// No match = 0
assert_eq!(en_us.match_score(&Locale::fr()), 0);
```

## Message Translation

### JSON Format

Create `locales/en.json`:

```json
{
  "hello": "Hello!",
  "greeting": "Hello, {name}!",
  "items": {
    "one": "{n} item",
    "other": "{n} items"
  },
  "nav": {
    "home": "Home",
    "about": "About"
  }
}
```

Create `locales/fr.json`:

```json
{
  "hello": "Bonjour!",
  "greeting": "Bonjour, {name}!",
  "items": {
    "one": "{n} article",
    "other": "{n} articles"
  }
}
```

### Loading Messages

```rust
use armature_i18n::{I18n, Locale};

let i18n = I18n::new()
    .with_default_locale(Locale::en_us())
    .load_from_dir("locales/")?;

// Simple translation
let hello = i18n.t("hello", &Locale::fr());
// "Bonjour!"

// With arguments
let greeting = i18n.t_args("greeting", &Locale::en(), &[("name", "World")]);
// "Hello, World!"

// Nested keys
let home = i18n.t("nav.home", &Locale::en());
// "Home"

// Pluralization
let items = i18n.t_plural("items", 1, &Locale::en());
// "1 item"
let items = i18n.t_plural("items", 5, &Locale::en());
// "5 items"
```

### Fluent Format (Advanced)

Enable with `features = ["fluent"]`.

Create `locales/en.ftl`:

```ftl
hello = Hello, World!

greeting = Hello, { $name }!

items = { $count ->
    [one] { $count } item
   *[other] { $count } items
}

# Gender-aware
welcome = { $gender ->
    [male] Welcome, Mr. { $name }
    [female] Welcome, Ms. { $name }
   *[other] Welcome, { $name }
}
```

```rust
use armature_i18n::{FluentBundle, FluentValue, Locale};

let mut bundle = FluentBundle::new(&Locale::en())?;
bundle.add_resource(include_str!("locales/en.ftl"))?;

let mut args = HashMap::new();
args.insert("name".to_string(), FluentValue::from("Alice"));

let msg = bundle.format("greeting", Some(&args))?;
// "Hello, Alice!"
```

## Pluralization

CLDR plural rules are implemented for major language families:

### Supported Languages

| Language | Categories |
|----------|------------|
| English, German | one, other |
| French | one (0, 1), other |
| Russian, Ukrainian | one, few, many |
| Polish | one, few, many |
| Arabic | zero, one, two, few, many, other |
| Japanese, Chinese, Korean | other (no plurals) |
| Welsh | zero, one, two, few, many, other |

### Examples

```rust
use armature_i18n::{plural_category, PluralCategory, Locale};

// English
let en = Locale::en();
assert_eq!(plural_category(1, &en), PluralCategory::One);
assert_eq!(plural_category(2, &en), PluralCategory::Other);
assert_eq!(plural_category(0, &en), PluralCategory::Other);

// French (0 and 1 are singular)
let fr = Locale::fr();
assert_eq!(plural_category(0, &fr), PluralCategory::One);
assert_eq!(plural_category(1, &fr), PluralCategory::One);
assert_eq!(plural_category(2, &fr), PluralCategory::Other);

// Russian (complex)
let ru = Locale::new("ru", None::<&str>);
assert_eq!(plural_category(1, &ru), PluralCategory::One);   // 1 книга
assert_eq!(plural_category(2, &ru), PluralCategory::Few);   // 2 книги
assert_eq!(plural_category(5, &ru), PluralCategory::Many);  // 5 книг
assert_eq!(plural_category(21, &ru), PluralCategory::One);  // 21 книга
```

## Number Formatting

Locale-aware number formatting with proper separators:

```rust
use armature_i18n::{format_number, format_percent, NumberFormatter, Locale};

let n = 1234567.89;

// US English: comma grouping, period decimal
format_number(n, &Locale::en_us());
// "1,234,567.89"

// German: period grouping, comma decimal
format_number(n, &Locale::de_de());
// "1.234.567,89"

// French: space grouping, comma decimal
format_number(n, &Locale::fr_fr());
// "1 234 567,89"

// Custom formatting
let formatter = NumberFormatter::new()
    .min_fraction_digits(2)
    .max_fraction_digits(4)
    .use_grouping(false);
formatter.format(1234.5, &Locale::en_us());
// "1234.50"

// Percentages
format_percent(0.75, &Locale::en_us());  // "75%"
format_percent(0.125, &Locale::de_de()); // "12,5%"
```

## Currency Formatting

```rust
use armature_i18n::{format_currency, CurrencyFormatter, Locale};

// US Dollars
format_currency(99.99, "USD", &Locale::en_us());
// "$99.99"

// Euros (Germany - symbol after)
format_currency(99.99, "EUR", &Locale::de_de());
// "99,99 €"

// British Pounds
format_currency(99.99, "GBP", &Locale::en_gb());
// "£99.99"

// Custom formatting
let formatter = CurrencyFormatter::new("USD")
    .use_symbol(false);
formatter.format(99.99, &Locale::en_us());
// "99.99 USD"
```

### Supported Currencies

USD, EUR, GBP, JPY, CNY, KRW, INR, RUB, BRL, CHF, CAD, AUD, HKD, SGD, SEK, NOK, DKK, PLN, CZK, MXN, THB, TWD

## Date Formatting

```rust
use armature_i18n::{format_date, DateFormatter, DateStyle, TimeStyle, Locale};

// Default (Medium)
format_date(2024, 1, 15, &Locale::en_us());
// "Jan 15, 2024"

format_date(2024, 1, 15, &Locale::de_de());
// "15 Jan 2024"

// Date styles
let formatter = DateFormatter::new().date_style(DateStyle::Short);
formatter.format_date(2024, 1, 15, &Locale::en_us());
// "1/15/24"

formatter.format_date(2024, 1, 15, &Locale::en_gb());
// "15/1/24" (day-first in UK)

// Time formatting
let formatter = DateFormatter::new().time_style(TimeStyle::Short);
formatter.format_time(14, 30, 0, &Locale::en_us());
// "2:30 PM" (12-hour)

formatter.format_time(14, 30, 0, &Locale::de_de());
// "14:30" (24-hour)
```

## Integration with HTTP

### Middleware Example

```rust
use armature_core::middleware::{Middleware, Next};
use armature_core::http::{HttpRequest, HttpResponse};
use armature_i18n::{I18n, Locale, parse_accept_language, negotiate_locale};

pub struct I18nMiddleware {
    i18n: I18n,
    available: Vec<Locale>,
}

impl Middleware for I18nMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        // Get Accept-Language header
        let accept_lang = req.headers.get("Accept-Language")
            .map(|s| s.as_str())
            .unwrap_or("en");

        // Negotiate locale
        let requested = parse_accept_language(accept_lang);
        let locale = negotiate_locale(&requested, &self.available, &Locale::en_us());

        // Store in request extensions
        req.extensions_mut().insert(locale.clone());

        next.run(req).await
    }
}
```

### Handler Example

```rust
async fn handler(req: HttpRequest, i18n: &I18n) -> HttpResponse {
    let locale = req.extensions().get::<Locale>()
        .unwrap_or(&Locale::en_us());

    let greeting = i18n.t("greeting", locale);
    HttpResponse::ok().with_body(greeting)
}
```

## Best Practices

1. **Use Language-Only Fallbacks**: Configure `en` as fallback for `en-US`, `en-GB`, etc.

2. **Externalize All Strings**: Even "OK" and "Cancel" should be translatable.

3. **Use ICU Message Format**: For complex formatting needs, enable the `fluent` feature.

4. **Test Plurals Thoroughly**: Especially for Slavic and Arabic languages.

5. **Consider RTL**: Arabic and Hebrew need right-to-left layout support.

6. **Use ISO Codes**: Stick to ISO 639-1 language codes and ISO 3166-1 region codes.

## Summary

The `armature-i18n` crate provides:

- **Locale parsing** with BCP 47 support
- **Accept-Language negotiation** for HTTP
- **CLDR plural rules** for 10+ language families
- **Locale-aware formatting** for numbers, currencies, and dates
- **Fluent support** for advanced message formatting

