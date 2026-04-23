//! Money and currency types

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Mul, Sub};

/// Currency codes (ISO 4217)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    USD,
    EUR,
    GBP,
    JPY,
    CAD,
    AUD,
    CHF,
    CNY,
    INR,
    MXN,
    BRL,
    SGD,
    HKD,
    NZD,
    SEK,
    NOK,
    DKK,
    PLN,
    ZAR,
    KRW,
}

impl Currency {
    /// Get currency code string
    pub fn code(&self) -> &'static str {
        match self {
            Self::USD => "USD",
            Self::EUR => "EUR",
            Self::GBP => "GBP",
            Self::JPY => "JPY",
            Self::CAD => "CAD",
            Self::AUD => "AUD",
            Self::CHF => "CHF",
            Self::CNY => "CNY",
            Self::INR => "INR",
            Self::MXN => "MXN",
            Self::BRL => "BRL",
            Self::SGD => "SGD",
            Self::HKD => "HKD",
            Self::NZD => "NZD",
            Self::SEK => "SEK",
            Self::NOK => "NOK",
            Self::DKK => "DKK",
            Self::PLN => "PLN",
            Self::ZAR => "ZAR",
            Self::KRW => "KRW",
        }
    }

    /// Get currency symbol
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::USD | Self::CAD | Self::AUD | Self::NZD | Self::SGD | Self::HKD | Self::MXN => {
                "$"
            }
            Self::EUR => "€",
            Self::GBP => "£",
            Self::JPY | Self::CNY => "¥",
            Self::CHF => "CHF",
            Self::INR => "₹",
            Self::BRL => "R$",
            Self::SEK | Self::NOK | Self::DKK => "kr",
            Self::PLN => "zł",
            Self::ZAR => "R",
            Self::KRW => "₩",
        }
    }

    /// Get decimal places (0 for zero-decimal currencies)
    pub fn decimals(&self) -> u32 {
        match self {
            Self::JPY | Self::KRW => 0,
            _ => 2,
        }
    }

    /// Is a zero-decimal currency
    pub fn is_zero_decimal(&self) -> bool {
        self.decimals() == 0
    }

    /// Parse from string
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_uppercase().as_str() {
            "USD" => Some(Self::USD),
            "EUR" => Some(Self::EUR),
            "GBP" => Some(Self::GBP),
            "JPY" => Some(Self::JPY),
            "CAD" => Some(Self::CAD),
            "AUD" => Some(Self::AUD),
            "CHF" => Some(Self::CHF),
            "CNY" => Some(Self::CNY),
            "INR" => Some(Self::INR),
            "MXN" => Some(Self::MXN),
            "BRL" => Some(Self::BRL),
            "SGD" => Some(Self::SGD),
            "HKD" => Some(Self::HKD),
            "NZD" => Some(Self::NZD),
            "SEK" => Some(Self::SEK),
            "NOK" => Some(Self::NOK),
            "DKK" => Some(Self::DKK),
            "PLN" => Some(Self::PLN),
            "ZAR" => Some(Self::ZAR),
            "KRW" => Some(Self::KRW),
            _ => None,
        }
    }
}

impl Default for Currency {
    fn default() -> Self {
        Self::USD
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Money amount with currency
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// Amount in smallest currency unit (cents, pence, etc.)
    pub amount: i64,
    /// Currency
    pub currency: Currency,
}

impl Money {
    /// Create a new money amount from smallest unit
    pub fn new(amount: i64, currency: Currency) -> Self {
        Self { amount, currency }
    }

    /// Create from decimal amount (e.g., 29.99)
    pub fn from_decimal(amount: Decimal, currency: Currency) -> Self {
        let multiplier = 10i64.pow(currency.decimals());
        let amount = (amount * Decimal::from(multiplier))
            .round()
            .to_string()
            .parse()
            .unwrap_or(0);
        Self { amount, currency }
    }

    /// Create from float amount
    pub fn from_float(amount: f64, currency: Currency) -> Self {
        let multiplier = 10f64.powi(currency.decimals() as i32);
        let amount = (amount * multiplier).round() as i64;
        Self { amount, currency }
    }

    /// Create USD amount from cents
    pub fn usd(cents: i64) -> Self {
        Self::new(cents, Currency::USD)
    }

    /// Create EUR amount from cents
    pub fn eur(cents: i64) -> Self {
        Self::new(cents, Currency::EUR)
    }

    /// Create GBP amount from pence
    pub fn gbp(pence: i64) -> Self {
        Self::new(pence, Currency::GBP)
    }

    /// Get amount as decimal
    pub fn to_decimal(&self) -> Decimal {
        let divisor = Decimal::from(10i64.pow(self.currency.decimals()));
        Decimal::from(self.amount) / divisor
    }

    /// Get amount as float
    pub fn to_float(&self) -> f64 {
        let divisor = 10f64.powi(self.currency.decimals() as i32);
        self.amount as f64 / divisor
    }

    /// Format for display
    pub fn format(&self) -> String {
        let decimal = self.to_decimal();
        format!(
            "{}{:.prec$}",
            self.currency.symbol(),
            decimal,
            prec = self.currency.decimals() as usize
        )
    }

    /// Check if zero
    pub fn is_zero(&self) -> bool {
        self.amount == 0
    }

    /// Check if negative
    pub fn is_negative(&self) -> bool {
        self.amount < 0
    }

    /// Absolute value
    pub fn abs(&self) -> Self {
        Self {
            amount: self.amount.abs(),
            currency: self.currency,
        }
    }

    /// Negate
    pub fn negate(&self) -> Self {
        Self {
            amount: -self.amount,
            currency: self.currency,
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

impl Add for Money {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        assert_eq!(self.currency, other.currency, "Currency mismatch");
        Self {
            amount: self.amount + other.amount,
            currency: self.currency,
        }
    }
}

impl Sub for Money {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        assert_eq!(self.currency, other.currency, "Currency mismatch");
        Self {
            amount: self.amount - other.amount,
            currency: self.currency,
        }
    }
}

impl Mul<i64> for Money {
    type Output = Self;

    fn mul(self, rhs: i64) -> Self {
        Self {
            amount: self.amount * rhs,
            currency: self.currency,
        }
    }
}

/// Price with optional compare-at price
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Price {
    /// Current price
    pub amount: Money,
    /// Compare-at price (original/list price)
    pub compare_at: Option<Money>,
}

impl Price {
    /// Create a new price
    pub fn new(amount: Money) -> Self {
        Self {
            amount,
            compare_at: None,
        }
    }

    /// With compare-at price
    pub fn with_compare_at(mut self, compare_at: Money) -> Self {
        self.compare_at = Some(compare_at);
        self
    }

    /// Is on sale
    pub fn is_on_sale(&self) -> bool {
        self.compare_at
            .map(|c| c.amount > self.amount.amount)
            .unwrap_or(false)
    }

    /// Get discount percentage
    pub fn discount_percent(&self) -> Option<f64> {
        self.compare_at.map(|compare| {
            if compare.amount > 0 {
                ((compare.amount - self.amount.amount) as f64 / compare.amount as f64) * 100.0
            } else {
                0.0
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_money_creation() {
        let money = Money::usd(2999);
        assert_eq!(money.amount, 2999);
        assert_eq!(money.currency, Currency::USD);
    }

    #[test]
    fn test_money_from_float() {
        let money = Money::from_float(29.99, Currency::USD);
        assert_eq!(money.amount, 2999);
    }

    #[test]
    fn test_money_format() {
        let money = Money::usd(2999);
        assert_eq!(money.format(), "$29.99");

        let yen = Money::new(1000, Currency::JPY);
        assert_eq!(yen.format(), "¥1000");
    }

    #[test]
    fn test_money_arithmetic() {
        let a = Money::usd(1000);
        let b = Money::usd(500);

        assert_eq!((a + b).amount, 1500);
        assert_eq!((a - b).amount, 500);
        assert_eq!((a * 2).amount, 2000);
    }

    #[test]
    fn test_currency() {
        assert_eq!(Currency::USD.symbol(), "$");
        assert_eq!(Currency::EUR.symbol(), "€");
        assert_eq!(Currency::JPY.decimals(), 0);
        assert!(Currency::JPY.is_zero_decimal());
    }

    #[test]
    fn test_price_discount() {
        let price = Price::new(Money::usd(2000)).with_compare_at(Money::usd(2500));

        assert!(price.is_on_sale());
        assert!((price.discount_percent().unwrap() - 20.0).abs() < 0.01);
    }
}
