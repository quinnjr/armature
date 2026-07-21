# armature-i18n

Internationalization (i18n) support for the Armature framework.

## Features

- **Message Translation** - key/value messages, plus a `key = value` subset of Fluent
- **Locale Detection** - Accept-Language header parsing
- **Pluralization** - CLDR plural rules
- **Date/Number Formatting** - Locale-aware formatting
- **Locale Negotiation** - Best-match locale selection

## Installation

```toml
[dependencies]
armature-i18n = "0.1"
```

## Quick Start

`I18n` holds a set of per-locale `MessageBundle`s. Translation lookups take the
message key and the target `&Locale`, returning a `String` (falling back to the
fallback locale, then the default locale, then the key itself).

```rust
use armature_i18n::{I18n, Locale, MessageBundle, TranslationSource};

let i18n = I18n::new()
    .with_default_locale(Locale::en_us())
    .with_fallback(Locale::en());

// Build a bundle in code...
let mut en = MessageBundle::new();
en.add("hello", "Hello!");
en.add("greeting", "Hello, {name}!");
i18n.add_bundle(&Locale::en(), en);

// ...or from a JSON / Fluent / in-memory source.
let es = TranslationSource::Json(r#"{ "hello": "¡Hola!" }"#.to_string());
i18n.add_source(&Locale::es(), &es).unwrap();

// Simple translation
let msg = i18n.t("hello", &Locale::es());            // "¡Hola!"

// With arguments — replaces {name} placeholders
let msg = i18n.t_args("greeting", &Locale::en(), &[("name", "World")]);
// "Hello, World!"

// Pluralization (count-aware, CLDR plural categories)
let msg = i18n.t_plural("items", 5, &Locale::en());
```

Load every `*.json` bundle from a directory (filenames are locale tags, e.g.
`en.json`, `en-US.json`, `fr.json`):

```rust
use armature_i18n::{I18n, Locale};

let i18n = I18n::new()
    .with_default_locale(Locale::en_us())
    .with_fallback(Locale::en())
    .load_from_dir("locales/")?;
# Ok::<(), armature_i18n::I18nError>(())
```

## Message Files

JSON is the directory-loaded format; nested objects express plural forms:

```json
{
  "hello": "Hello!",
  "greeting": "Hello, {name}!",
  "items": { "one": "{n} item", "other": "{n} items" }
}
```

A pragmatic `key = value` subset of Fluent is also accepted via
`MessageBundle::from_fluent` / `TranslationSource::Fluent`:

```ftl
# locales/en.ftl
hello = Hello!
greeting = Hello, {name}!
```

## Locale Detection

```rust
use armature_i18n::{negotiate_locale, parse_accept_language, Locale};

// Parse an Accept-Language header (sorted by quality, highest first)
let requested = parse_accept_language("en-US,en;q=0.9,es;q=0.8");

// Negotiate the best available locale (highest match score wins)
let available = [Locale::en_us(), Locale::es_es(), Locale::fr_fr()];
let default = Locale::en_us();
let best = negotiate_locale(&requested, &available, &default);
```

## License

MIT OR Apache-2.0

