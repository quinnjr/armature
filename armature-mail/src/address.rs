//! Email address types.

use crate::{MailError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Email address with optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    /// The email address.
    pub email: String,
    /// Optional display name.
    pub name: Option<String>,
}

impl Address {
    /// Create a new address with just an email.
    pub fn new(email: impl Into<String>) -> Result<Self> {
        let email = email.into();
        validate_email(&email)?;
        Ok(Self { email, name: None })
    }

    /// Create a new address with a display name.
    ///
    /// The display name is rejected if it contains CR, LF, or any other ASCII
    /// control character: it is emitted verbatim into a header by the Mailgun
    /// and SendGrid transports, so a newline there is a header-injection
    /// primitive (`"Foo\r\nBcc: evil@example.com"`).
    pub fn with_name(email: impl Into<String>, name: impl Into<String>) -> Result<Self> {
        let email = email.into();
        validate_email(&email)?;
        let name = name.into();
        validate_no_control_chars(&name).map_err(|_| {
            MailError::InvalidAddress(format!(
                "Display name contains control characters: {:?}",
                name
            ))
        })?;
        Ok(Self {
            email,
            name: Some(name),
        })
    }

    /// Parse an address from a string like "Name <email@example.com>" or "email@example.com".
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();

        // Check for "Name <email>" format.
        //
        // The closing `>` is searched for *after* `start` only: locating the two
        // independently lets an input like `"a>b <x@y.com>"` produce an inverted
        // slice range and panic.
        if let Some(start) = s.find('<') {
            let Some(end) = s[start + 1..].find('>').map(|i| i + start + 1) else {
                return Err(MailError::InvalidAddress(format!(
                    "Unterminated angle-addr: {}",
                    s
                )));
            };

            let name = s[..start].trim().trim_matches('"');
            let email = s[start + 1..end].trim();

            // A `>` in the display-name position is malformed per RFC 5322 and is
            // exactly the shape that used to panic; reject rather than guess.
            if name.contains('>') || name.contains('<') {
                return Err(MailError::InvalidAddress(format!(
                    "Malformed angle-addr: {}",
                    s
                )));
            }

            if name.is_empty() {
                return Self::new(email);
            } else {
                return Self::with_name(email, name);
            }
        }

        // Just an email address
        Self::new(s)
    }

    /// Get the email address.
    pub fn email(&self) -> &str {
        &self.email
    }

    /// Get the display name.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Convert to a lettre address.
    pub(crate) fn to_lettre(&self) -> Result<lettre::Address> {
        self.email
            .parse()
            .map_err(|_| MailError::InvalidAddress(self.email.clone()))
    }

    /// Convert to a lettre mailbox.
    pub(crate) fn to_mailbox(&self) -> Result<lettre::message::Mailbox> {
        Ok(lettre::message::Mailbox::new(
            self.name.clone(),
            self.to_lettre()?,
        ))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{} <{}>", name, self.email),
            None => write!(f, "{}", self.email),
        }
    }
}

impl TryFrom<&str> for Address {
    type Error = MailError;

    fn try_from(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl TryFrom<String> for Address {
    type Error = MailError;

    fn try_from(s: String) -> Result<Self> {
        Self::parse(&s)
    }
}

/// Trait for types that can be converted to an Address.
///
/// This allows accepting both `Address` directly and string types that
/// can be parsed into addresses.
pub trait IntoAddress {
    /// Convert into an Address.
    fn into_address(self) -> Result<Address>;
}

impl IntoAddress for Address {
    fn into_address(self) -> Result<Address> {
        Ok(self)
    }
}

impl IntoAddress for &str {
    fn into_address(self) -> Result<Address> {
        Address::parse(self)
    }
}

impl IntoAddress for String {
    fn into_address(self) -> Result<Address> {
        Address::parse(&self)
    }
}

impl IntoAddress for &String {
    fn into_address(self) -> Result<Address> {
        Address::parse(self)
    }
}

/// A mailbox is an address with a required display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    /// The address.
    pub address: Address,
}

impl Mailbox {
    /// Create a new mailbox.
    pub fn new(email: impl Into<String>, name: impl Into<String>) -> Result<Self> {
        Ok(Self {
            address: Address::with_name(email, name)?,
        })
    }
}

impl From<Address> for Mailbox {
    fn from(address: Address) -> Self {
        Self { address }
    }
}

/// Reject CR, LF, and any other ASCII control character.
///
/// Every transport that writes headers by hand (Mailgun's form body, SendGrid's
/// JSON) inherits its injection protection from this check; only the SMTP path
/// is independently protected by lettre's encoder.
pub(crate) fn validate_no_control_chars(value: &str) -> Result<()> {
    if let Some(c) = value.chars().find(|c| c.is_ascii_control()) {
        return Err(MailError::InvalidAddress(format!(
            "Value contains control character {:?}",
            c
        )));
    }
    Ok(())
}

/// Validate an email address (basic validation).
fn validate_email(email: &str) -> Result<()> {
    // Checked against the *raw* input, not a trimmed copy: `Address` stores the
    // string as given, so trailing `\r\n` trimmed away here would still reach
    // the wire. A `\r\n` on the API transports is a header-injection primitive.
    validate_no_control_chars(email).map_err(|_| {
        MailError::InvalidAddress(format!("Email contains control characters: {:?}", email))
    })?;

    if email.chars().any(char::is_whitespace) {
        return Err(MailError::InvalidAddress(format!(
            "Email contains whitespace: {:?}",
            email
        )));
    }

    if email.is_empty() {
        return Err(MailError::InvalidAddress(
            "Email cannot be empty".to_string(),
        ));
    }

    if !email.contains('@') {
        return Err(MailError::InvalidAddress(format!(
            "Invalid email format: {}",
            email
        )));
    }

    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return Err(MailError::InvalidAddress(format!(
            "Invalid email format: {}",
            email
        )));
    }

    let local = parts[0];
    let domain = parts[1];

    if local.is_empty() || domain.is_empty() {
        return Err(MailError::InvalidAddress(format!(
            "Invalid email format: {}",
            email
        )));
    }

    if !domain.contains('.') {
        return Err(MailError::InvalidAddress(format!(
            "Invalid domain in email: {}",
            email
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_parse() {
        let addr = Address::parse("test@example.com").unwrap();
        assert_eq!(addr.email, "test@example.com");
        assert!(addr.name.is_none());

        let addr = Address::parse("John Doe <john@example.com>").unwrap();
        assert_eq!(addr.email, "john@example.com");
        assert_eq!(addr.name.as_deref(), Some("John Doe"));
    }

    #[test]
    fn test_address_display() {
        let addr = Address::new("test@example.com").unwrap();
        assert_eq!(format!("{}", addr), "test@example.com");

        let addr = Address::with_name("test@example.com", "John").unwrap();
        assert_eq!(format!("{}", addr), "John <test@example.com>");
    }

    #[test]
    fn test_invalid_email() {
        assert!(Address::new("invalid").is_err());
        assert!(Address::new("@example.com").is_err());
        assert!(Address::new("test@").is_err());
    }

    /// `find('<')` and `find('>')` used to be located independently, so a `>`
    /// before the `<` produced an inverted slice range and panicked.
    #[test]
    fn malformed_brackets_error_instead_of_panicking() {
        for input in ["a>b <x@y.com>", ">", "<", "><", ">@<", "a>b", "<no-close"] {
            let result = Address::parse(input);
            assert!(
                matches!(result, Err(MailError::InvalidAddress(_))),
                "expected InvalidAddress for {input:?}, got {result:?}"
            );
        }
    }

    #[test]
    fn crlf_in_email_is_rejected() {
        assert!(Address::new("a@b.com\r\nBcc: evil@x.com").is_err());
        assert!(Address::new("a@b.com\n").is_err());
        assert!(Address::new("a @b.com").is_err());
        assert!(Address::new("a@b.com\u{0}").is_err());
    }

    #[test]
    fn crlf_in_display_name_is_rejected() {
        assert!(Address::with_name("a@b.com", "Foo\r\nBcc: evil@x.com").is_err());
        assert!(Address::with_name("a@b.com", "Foo\nBar").is_err());
        assert!(Address::with_name("a@b.com", "Foo Bar").is_ok());
    }
}
