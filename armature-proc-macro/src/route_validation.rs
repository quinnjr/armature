//! Compile-time route path validation
//!
//! Validates route paths at compile time to catch errors early:
//! - Path must start with "/" or be empty
//! - No double slashes
//! - Valid parameter syntax (:param or {param})
//! - Valid wildcard syntax (*name or {*name})
//! - No duplicate parameter names
//! - Wildcard must be at the end
//! - Parameter names must be valid identifiers

use proc_macro2::Span;
use syn::Error;

/// Result of route validation
#[allow(dead_code)]
pub struct ValidatedRoute {
    /// The validated path, preserved verbatim.
    ///
    /// Both `:param` and `{param}` forms are accepted and passed through
    /// unchanged; the path is *not* rewritten. The framework's router matches
    /// the `:param` form directly, so no conversion is performed here.
    pub path: String,
    /// Extracted parameter names for validation
    pub params: Vec<String>,
    /// Whether the route has a wildcard
    pub has_wildcard: bool,
}

/// Validate a controller base path at compile time
///
/// Controller paths have slightly different rules:
/// - Can be empty (no prefix)
/// - Must start with "/" if non-empty
/// - Cannot have parameters or wildcards (those belong in routes)
pub fn validate_controller_path(path: &str, span: Span) -> Result<(), Error> {
    // Empty path is valid (no prefix)
    if path.is_empty() {
        return Ok(());
    }

    // Path must start with /
    if !path.starts_with('/') {
        return Err(Error::new(
            span,
            format!(
                "controller path must start with '/' or be empty, got: \"{path}\"\n\
                 hint: change to \"/{path}\" or use \"\" for no prefix"
            ),
        ));
    }

    // Check for double slashes
    if path.contains("//") {
        return Err(Error::new(
            span,
            format!(
                "controller path contains double slashes: \"{}\"\n\
                 hint: remove consecutive slashes",
                path
            ),
        ));
    }

    // Check for trailing slash (except for root "/")
    if path.len() > 1 && path.ends_with('/') {
        return Err(Error::new(
            span,
            format!(
                "controller path should not have trailing slash: \"{}\"\n\
                 hint: remove the trailing slash",
                path
            ),
        ));
    }

    // Controller paths should not have parameters
    if path.contains(':') || path.contains('{') {
        return Err(Error::new(
            span,
            format!(
                "controller path should not contain path parameters: \"{}\"\n\
                 hint: path parameters belong in individual route definitions, not the controller prefix",
                path
            ),
        ));
    }

    // Controller paths should not have wildcards
    if path.contains('*') {
        return Err(Error::new(
            span,
            format!(
                "controller path should not contain wildcards: \"{}\"\n\
                 hint: wildcards belong in individual route definitions, not the controller prefix",
                path
            ),
        ));
    }

    // Validate each segment
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        validate_segment(segment, span)?;
    }

    Ok(())
}

/// Validate a route path at compile time
pub fn validate_route_path(path: &str, span: Span) -> Result<ValidatedRoute, Error> {
    // Empty path is valid (will be combined with controller prefix)
    if path.is_empty() {
        return Ok(ValidatedRoute {
            path: String::new(),
            params: Vec::new(),
            has_wildcard: false,
        });
    }

    // Path must start with /
    if !path.starts_with('/') {
        return Err(Error::new(
            span,
            format!(
                "route path must start with '/', got: \"{path}\"\n\
                 hint: change to \"/{path}\""
            ),
        ));
    }

    // Check for double slashes
    if path.contains("//") {
        return Err(Error::new(
            span,
            format!(
                "route path contains double slashes: \"{}\"\n\
                 hint: remove consecutive slashes",
                path
            ),
        ));
    }

    // Check for trailing slash (except for root "/")
    if path.len() > 1 && path.ends_with('/') {
        return Err(Error::new(
            span,
            format!(
                "route path should not have trailing slash: \"{}\"\n\
                 hint: remove the trailing slash",
                path
            ),
        ));
    }

    let mut params = Vec::new();
    let mut has_wildcard = false;
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    for (i, segment) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;

        // Check for parameter syntax
        if let Some(param_name) = parse_parameter(segment) {
            // Validate parameter name is a valid Rust identifier
            validate_identifier(&param_name, span)?;

            // Check for duplicates
            if params.contains(&param_name) {
                return Err(Error::new(
                    span,
                    format!(
                        "duplicate path parameter '{}' in route: \"{}\"\n\
                         hint: each parameter name must be unique",
                        param_name, path
                    ),
                ));
            }

            params.push(param_name);
        }
        // Check for wildcard syntax
        else if let Some(wildcard_name) = parse_wildcard(segment) {
            // Wildcard must be at the end
            if !is_last {
                return Err(Error::new(
                    span,
                    format!(
                        "wildcard '*{}' must be at the end of the route: \"{}\"\n\
                         hint: move the wildcard segment to the end",
                        wildcard_name, path
                    ),
                ));
            }

            // Validate wildcard name
            validate_identifier(&wildcard_name, span)?;

            // Check for duplicates with params
            if params.contains(&wildcard_name) {
                return Err(Error::new(
                    span,
                    format!(
                        "wildcard name '{}' conflicts with path parameter: \"{}\"\n\
                         hint: use a unique name for the wildcard",
                        wildcard_name, path
                    ),
                ));
            }

            params.push(wildcard_name);
            has_wildcard = true;
        }
        // Static segment - validate characters
        else {
            validate_segment(segment, span)?;
        }
    }

    Ok(ValidatedRoute {
        path: path.to_string(),
        params,
        has_wildcard,
    })
}

/// Parse a parameter from a segment (e.g., ":id" or "{id}")
fn parse_parameter(segment: &str) -> Option<String> {
    // :param syntax
    if segment.starts_with(':') && segment.len() > 1 {
        return Some(segment[1..].to_string());
    }

    // {param} syntax (not wildcard)
    if segment.starts_with('{')
        && segment.ends_with('}')
        && !segment.starts_with("{*")
        && segment.len() > 2
    {
        return Some(segment[1..segment.len() - 1].to_string());
    }

    None
}

/// Parse a wildcard from a segment (e.g., "*path" or "{*path}")
fn parse_wildcard(segment: &str) -> Option<String> {
    // *path syntax
    if segment.starts_with('*') && segment.len() > 1 {
        return Some(segment[1..].to_string());
    }

    // {*path} syntax
    if segment.starts_with("{*") && segment.ends_with('}') && segment.len() > 3 {
        return Some(segment[2..segment.len() - 1].to_string());
    }

    None
}

/// Validate that a string is a valid Rust identifier
fn validate_identifier(name: &str, span: Span) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::new(span, "parameter name cannot be empty"));
    }

    let first = name.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return Err(Error::new(
            span,
            format!(
                "parameter name '{}' must start with a letter or underscore\n\
                 hint: valid names start with a-z, A-Z, or _",
                name
            ),
        ));
    }

    for c in name.chars() {
        if !c.is_alphanumeric() && c != '_' {
            return Err(Error::new(
                span,
                format!(
                    "parameter name '{}' contains invalid character '{}'\n\
                     hint: use only letters, numbers, and underscores",
                    name, c
                ),
            ));
        }
    }

    // Check for Rust keywords (common ones that might be used)
    let keywords = [
        "type", "fn", "let", "mut", "ref", "self", "Self", "struct", "enum", "impl", "trait",
        "pub", "mod", "use", "const", "static", "async", "await", "loop", "while", "for", "if",
        "else", "match", "return", "break", "continue", "move", "box", "where", "as", "in",
        "extern", "crate", "super", "dyn", "unsafe",
    ];

    if keywords.contains(&name) {
        return Err(Error::new(
            span,
            format!(
                "parameter name '{}' is a Rust keyword\n\
                 hint: use a different name like '{}_param' or '_{}'",
                name, name, name
            ),
        ));
    }

    Ok(())
}

/// Validate a static segment contains only valid URL characters
fn validate_segment(segment: &str, span: Span) -> Result<(), Error> {
    // Allow alphanumeric, hyphen, underscore, dot, and tilde
    for c in segment.chars() {
        if !c.is_alphanumeric() && !matches!(c, '-' | '_' | '.' | '~') {
            return Err(Error::new(
                span,
                format!(
                    "route segment '{}' contains invalid character '{}'\n\
                     hint: use only letters, numbers, hyphens, underscores, dots, or tildes\n\
                     hint: for a path parameter, use ':{}' or '{{{}}}' syntax",
                    segment, c, segment, segment
                ),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(path: &str) -> Result<ValidatedRoute, String> {
        validate_route_path(path, Span::call_site()).map_err(|e| e.to_string())
    }

    #[test]
    fn test_valid_paths() {
        assert!(validate("/").is_ok());
        assert!(validate("/users").is_ok());
        assert!(validate("/users/:id").is_ok());
        assert!(validate("/users/{id}").is_ok());
        assert!(validate("/api/v1/users/:id/posts/:post_id").is_ok());
        assert!(validate("/files/*path").is_ok());
        assert!(validate("/static/{*filepath}").is_ok());
        assert!(validate("").is_ok()); // Empty is valid
    }

    #[test]
    fn test_invalid_paths() {
        assert!(validate("users").is_err()); // Missing leading /
        assert!(validate("/users/").is_err()); // Trailing slash
        assert!(validate("/users//posts").is_err()); // Double slash
        assert!(validate("/users/:").is_err()); // Empty param name
        assert!(validate("/users/:123").is_err()); // Param starts with number
        assert!(validate("/users/:id/:id").is_err()); // Duplicate param
        assert!(validate("/*path/more").is_err()); // Wildcard not at end
    }
}
