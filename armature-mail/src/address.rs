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
    pub fn with_name(email: impl Into<String>, name: impl Into<String>) -> Result<Self> {
        let email = email.into();
        validate_email(&email)?;
        Ok(Self {
            email,
            name: Some(name.into()),
        })
    }

    /// Parse an address from a string like "Name <email@example.com>" or "email@example.com".
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();

        // Check for "Name <email>" format
        if let Some(start) = s.find('<')
            && let Some(end) = s.find('>')
        {
            let name = s[..start].trim().trim_matches('"');
            let email = s[start + 1..end].trim();

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

/// Validate an email address (basic validation).
fn validate_email(email: &str) -> Result<()> {
    let email = email.trim();

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
}
