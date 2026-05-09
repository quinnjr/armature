# Content Negotiation

This guide covers HTTP content negotiation in Armature, allowing your server to serve different representations of resources based on client preferences.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Accept Header (Media Types)](#accept-header-media-types)
- [Accept-Language Header](#accept-language-header)
- [Accept-Encoding Header](#accept-encoding-header)
- [Accept-Charset Header](#accept-charset-header)
- [Request Extensions](#request-extensions)
- [Content Negotiator](#content-negotiator)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

## Overview

Content negotiation is an HTTP mechanism that allows servers to serve different representations of a resource at the same URI. The client indicates its preferences through `Accept-*` headers, and the server selects the best matching representation.

Armature provides comprehensive support for:
- **Media type negotiation** (`Accept` header)
- **Language negotiation** (`Accept-Language` header)
- **Encoding negotiation** (`Accept-Encoding` header)
- **Charset negotiation** (`Accept-Charset` header)

## Features

- ✅ Full quality value (`q=`) support with proper sorting
- ✅ Wildcard matching (`*/*`, `*/json`, `text/*`)
- ✅ Specificity-based selection when quality values are equal
- ✅ Convenient request extension methods
- ✅ `ContentNegotiator` helper for multi-format responses
- ✅ Support for all standard `Accept-*` headers

## Accept Header (Media Types)

### Parsing Accept Headers

```rust
use armature_framework::prelude::*;

// Parse an Accept header
let accept = Accept::parse("application/json, text/html;q=0.9, */*;q=0.1");

// Access parsed media types (sorted by preference)
for (media_type, quality) in &accept.media_types {
    println!("{}: q={}", media_type, quality);
}
// Output:
// application/json: q=1.0
// text/html: q=0.9
// */*: q=0.1
```

### MediaType Helpers

```rust
use armature_framework::prelude::*;

// Built-in media type constructors
let json = MediaType::json();           // application/json
let html = MediaType::html();           // text/html
let xml = MediaType::xml();             // application/xml
let text = MediaType::plain_text();     // text/plain
let any = MediaType::any();             // */*

// Parse custom media types
let custom = MediaType::parse("application/vnd.api+json").unwrap();

// Add parameters
let with_charset = MediaType::html()
    .with_param("charset", "utf-8");

// Check for matches (considering wildcards)
assert!(MediaType::any().matches(&MediaType::json()));
assert!(MediaType::json().matches(&MediaType::any()));
```

### Negotiating Media Types

```rust
use armature_framework::prelude::*;

#[controller("/api")]
struct ApiController;

#[controller]
impl ApiController {
    #[get("/data")]
    async fn get_data(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        // Define available formats
        let available = vec![
            MediaType::json(),
            MediaType::xml(),
            MediaType::html(),
        ];

        // Negotiate best match
        let best = request.negotiate_media_type(&available)
            .unwrap_or(&MediaType::json());

        // Build response based on negotiated type
        if best.matches(&MediaType::json()) {
            HttpResponse::ok()
                .with_json(&serde_json::json!({"message": "Hello"}))
        } else if best.matches(&MediaType::xml()) {
            Ok(HttpResponse::ok()
                .with_header("Content-Type".into(), "application/xml".into())
                .with_body(b"<message>Hello</message>".to_vec()))
        } else {
            Ok(HttpResponse::ok()
                .with_header("Content-Type".into(), "text/html".into())
                .with_body(b"<h1>Hello</h1>".to_vec()))
        }
    }
}
```

### Quick Preference Checks

```rust
use armature_framework::prelude::*;

#[get("/")]
async fn index(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    if request.prefers_json() {
        // API client - return JSON
        HttpResponse::ok().with_json(&serde_json::json!({"page": "home"}))
    } else if request.prefers_html() {
        // Browser - return HTML
        Ok(HttpResponse::ok()
            .with_header("Content-Type".into(), "text/html".into())
            .with_body(b"<h1>Welcome</h1>".to_vec()))
    } else {
        // Default to JSON
        HttpResponse::ok().with_json(&serde_json::json!({"page": "home"}))
    }
}
```

## Accept-Language Header

### Parsing Language Preferences

```rust
use armature_framework::prelude::*;

let accept_lang = AcceptLanguage::parse("en-US, en;q=0.9, fr;q=0.8, *;q=0.1");

// Get preferred language
if let Some(preferred) = accept_lang.preferred() {
    println!("Preferred: {}", preferred); // en-US
}

// Check quality for specific languages
let en_quality = accept_lang.quality_for(&LanguageTag::new("en"));
let fr_quality = accept_lang.quality_for(&LanguageTag::new("fr"));
```

### Language Negotiation

```rust
use armature_framework::prelude::*;

#[get("/greeting")]
async fn greeting(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let available = vec![
        LanguageTag::with_subtag("en", "US"),
        LanguageTag::with_subtag("en", "GB"),
        LanguageTag::new("fr"),
        LanguageTag::new("de"),
    ];

    let best = request.negotiate_language(&available)
        .unwrap_or(&LanguageTag::new("en"));

    let message = match best.primary.as_str() {
        "en" => "Hello!",
        "fr" => "Bonjour!",
        "de" => "Hallo!",
        _ => "Hello!",
    };

    HttpResponse::ok()
        .with_header("Content-Language".into(), best.to_string())
        .with_json(&serde_json::json!({"message": message}))
}
```

## Accept-Encoding Header

### Parsing Encoding Preferences

```rust
use armature_framework::prelude::*;

let accept_enc = AcceptEncoding::parse("gzip, deflate, br;q=0.9");

// Check if specific encoding is accepted
if accept_enc.accepts(Encoding::Gzip) {
    // Client supports gzip
}

// Get preferred encoding
if let Some(preferred) = accept_enc.preferred() {
    println!("Preferred: {}", preferred); // gzip
}
```

### Encoding Negotiation

```rust
use armature_framework::prelude::*;

#[get("/data")]
async fn get_data(&self, request: HttpRequest) -> HttpResponse {
    let available = vec![
        Encoding::Brotli,
        Encoding::Gzip,
        Encoding::Deflate,
    ];

    if let Some(encoding) = request.negotiate_encoding(&available) {
        // Compress response with selected encoding
        let mut response = HttpResponse::ok();
        response.headers.insert(
            "Content-Encoding".to_string(),
            encoding.to_header_value().to_string(),
        );
        // ... compress body ...
        response
    } else {
        // Send uncompressed
        HttpResponse::ok()
    }
}
```

## Accept-Charset Header

```rust
use armature_framework::prelude::*;

let accept_charset = AcceptCharset::parse("utf-8, iso-8859-1;q=0.8");

// Check quality for specific charset
let utf8_quality = accept_charset.quality_for("utf-8");  // 1.0

// Get preferred charset
if let Some(preferred) = accept_charset.preferred() {
    println!("Preferred: {}", preferred); // utf-8
}
```

## Request Extensions

Armature adds convenient methods directly to `HttpRequest`:

```rust
use armature_framework::prelude::*;

fn handle_request(request: &HttpRequest) {
    // Parse Accept header
    let accept = request.accept();

    // Parse Accept-Language header
    let accept_lang = request.accept_language();

    // Parse Accept-Encoding header
    let accept_enc = request.accept_encoding();

    // Parse Accept-Charset header
    let accept_charset = request.accept_charset();

    // Quick checks
    let accepts_json = request.accepts(&MediaType::json());
    let prefers_json = request.prefers_json();
    let prefers_html = request.prefers_html();

    // Negotiate from available options
    let available_types = vec![MediaType::json(), MediaType::html()];
    let best_type = request.negotiate_media_type(&available_types);

    let available_langs = vec![LanguageTag::new("en"), LanguageTag::new("fr")];
    let best_lang = request.negotiate_language(&available_langs);

    let available_encs = vec![Encoding::Gzip, Encoding::Brotli];
    let best_enc = request.negotiate_encoding(&available_encs);
}
```

## Content Negotiator

For complex scenarios, use `ContentNegotiator` to define multiple response formats:

```rust
use armature_framework::prelude::*;
use armature_core::content_negotiation::ContentNegotiator;

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
    email: String,
}

#[get("/user/:id")]
async fn get_user(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let user = User {
        id: 1,
        name: "John".to_string(),
        email: "john@example.com".to_string(),
    };

    ContentNegotiator::new()
        .json(move || serde_json::json!({
            "id": user.id,
            "name": user.name,
            "email": user.email
        }))
        .html(|| format!(
            "<div><h1>{}</h1><p>{}</p></div>",
            "John", "john@example.com"
        ))
        .plain_text(|| "User: John (john@example.com)".to_string())
        .xml(|| "<user><name>John</name><email>john@example.com</email></user>".to_string())
        .negotiate(&request)
}
```

### Simple Response Helper

For straightforward cases, use `respond_with`:

```rust
use armature_core::content_negotiation::respond_with;

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    data: String,
}

#[get("/status")]
async fn status(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let data = ApiResponse {
        success: true,
        data: "All systems operational".to_string(),
    };

    // Automatically returns JSON or HTML based on Accept header
    respond_with(&request, &data)
}
```

## Best Practices

### 1. Always Include Vary Header

```rust
response.headers.insert("Vary".to_string(), "Accept".to_string());
```

This ensures caches handle content negotiation correctly.

### 2. Provide Sensible Defaults

```rust
let best = request.negotiate_media_type(&available)
    .unwrap_or(&MediaType::json());  // Default to JSON
```

### 3. Return 406 Not Acceptable When Appropriate

```rust
use armature_framework::prelude::*;

#[get("/data")]
async fn get_data(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let available = vec![MediaType::json()];

    if let Some(_) = request.negotiate_media_type(&available) {
        HttpResponse::ok().with_json(&serde_json::json!({"data": "value"}))
    } else {
        Err(Error::NotAcceptable(
            "Only application/json is supported".to_string()
        ))
    }
}
```

### 4. Support Common Formats

For APIs, consider supporting:
- `application/json` (primary)
- `application/xml` (optional)
- `text/html` (for browser debugging)

### 5. Use Quality Values Correctly

When the client sends:
```
Accept: application/json;q=0.9, text/html;q=1.0
```

The server should prefer HTML (q=1.0) over JSON (q=0.9).

## Examples

### Complete API Endpoint

```rust
use armature_framework::prelude::*;
use armature_core::content_negotiation::ContentNegotiator;

#[derive(Serialize)]
struct Product {
    id: u64,
    name: String,
    price: f64,
}

#[controller("/products")]
struct ProductController;

#[controller]
impl ProductController {
    #[get("/:id")]
    async fn get_product(
        &self,
        request: HttpRequest,
        #[param("id")] id: u64,
    ) -> Result<HttpResponse, Error> {
        // Fetch product (simplified)
        let product = Product {
            id,
            name: "Widget".to_string(),
            price: 29.99,
        };

        // Negotiate response format
        let name = product.name.clone();
        let price = product.price;

        let mut response = ContentNegotiator::new()
            .json(move || serde_json::json!({
                "id": id,
                "name": name.clone(),
                "price": price,
            }))
            .html(move || format!(
                r#"<!DOCTYPE html>
                <html>
                <body>
                    <h1>{}</h1>
                    <p>Price: ${:.2}</p>
                </body>
                </html>"#,
                product.name, product.price
            ))
            .negotiate(&request)?;

        // Add language header if negotiated
        let langs = vec![LanguageTag::new("en"), LanguageTag::new("es")];
        if let Some(lang) = request.negotiate_language(&langs) {
            response.headers.insert(
                "Content-Language".to_string(),
                lang.to_string(),
            );
        }

        Ok(response)
    }
}
```

### Multi-Language API

```rust
use armature_framework::prelude::*;
use std::collections::HashMap;

#[controller("/i18n")]
struct I18nController {
    translations: HashMap<String, HashMap<String, String>>,
}

#[controller]
impl I18nController {
    #[get("/greeting")]
    async fn greeting(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        let available = vec![
            LanguageTag::new("en"),
            LanguageTag::new("es"),
            LanguageTag::new("fr"),
            LanguageTag::new("de"),
        ];

        let lang = request.negotiate_language(&available)
            .unwrap_or(&LanguageTag::new("en"));

        let greeting = match lang.primary.as_str() {
            "en" => "Hello, World!",
            "es" => "¡Hola, Mundo!",
            "fr" => "Bonjour, le Monde!",
            "de" => "Hallo, Welt!",
            _ => "Hello, World!",
        };

        HttpResponse::ok()
            .with_header("Content-Language".into(), lang.to_string())
            .with_header("Vary".into(), "Accept-Language".into())
            .with_json(&serde_json::json!({"greeting": greeting}))
    }
}
```

## Common Pitfalls

- ❌ Ignoring quality values in Accept headers
- ❌ Not including Vary header in responses
- ❌ Returning 200 when format is not supported (use 406)
- ❌ Hardcoding response format without checking preferences

- ✅ Always negotiate when multiple formats are available
- ✅ Include appropriate Vary headers for caching
- ✅ Return 406 Not Acceptable when no suitable format exists
- ✅ Provide sensible defaults for missing Accept headers

## Summary

| Component | Purpose |
|-----------|---------|
| `Accept` | Parse media type preferences |
| `AcceptLanguage` | Parse language preferences |
| `AcceptEncoding` | Parse encoding preferences |
| `AcceptCharset` | Parse charset preferences |
| `MediaType` | Represent and match MIME types |
| `ContentNegotiator` | Build multi-format responses |
| `respond_with` | Simple automatic format selection |

**Key Points:**

1. **Parse headers** - Use `request.accept()`, `request.accept_language()`, etc.
2. **Negotiate** - Use `negotiate_*` functions with available options
3. **Set Vary** - Always include `Vary` header for cached responses
4. **Handle 406** - Return Not Acceptable when no format matches
5. **Default wisely** - JSON is typically a safe default for APIs


