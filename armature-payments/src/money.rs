//! Money and currency types

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Mul, Sub};

/// Currency codes (ISO 4217)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Currency {
    #[default]
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

    /// Parse from string.
    ///
    /// Compares case-insensitively without allocating: ISO 4217 codes are three
    /// ASCII characters, and `to_uppercase()` allocated a `String` on every
    /// call — twice per `Charge` projection, on the charge and refund paths.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| code.eq_ignore_ascii_case(candidate.code()))
    }

    /// Every currency this crate models.
    const ALL: [Self; 20] = [
        Self::USD,
        Self::EUR,
        Self::GBP,
        Self::JPY,
        Self::CAD,
        Self::AUD,
        Self::CHF,
        Self::CNY,
        Self::INR,
        Self::MXN,
        Self::BRL,
        Self::SGD,
        Self::HKD,
        Self::NZD,
        Self::SEK,
        Self::NOK,
        Self::DKK,
        Self::PLN,
        Self::ZAR,
        Self::KRW,
    ];
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

    /// Parse the decimal amount string a gateway put on the wire.
    ///
    /// The inverse of [`to_gateway_string`](Self::to_gateway_string): `"29.99"`
    /// with [`Currency::USD`] is 2999 cents, `"1000"` with [`Currency::JPY`] is
    /// ¥1000.
    ///
    /// Conversion goes through [`Decimal`], not `f64`. A binary float cannot
    /// represent most decimal money amounts exactly, so `"0.29"` parsed as `f64`
    /// and multiplied by 100 is 28.999999999999996 — correct only because the
    /// rounding happens to go the right way, which is not a property to bet a
    /// ledger on.
    ///
    /// # Returns
    ///
    /// `None` if `value` is not a decimal number, or if it carries more
    /// precision than the currency has minor units (`"29.999"` in USD). Both are
    /// gateway responses this crate cannot represent faithfully, and a silently
    /// rounded amount reported as a success is worse than a surfaced failure.
    pub fn from_gateway_string(value: &str, currency: Currency) -> Option<Self> {
        use rust_decimal::prelude::ToPrimitive;

        let parsed = Decimal::from_str_exact(value.trim()).ok()?;
        // `pow` panics on overflow. Unreachable while every `decimals()` is 0 or
        // 2, but this is a parser fed by a gateway on the payment path, and a
        // currency added with an implausible precision must surface as an
        // unparseable amount, not as a panic in a request handler.
        let multiplier = 10i64.checked_pow(currency.decimals())?;
        let scaled = parsed.checked_mul(Decimal::from(multiplier))?;
        if !scaled.fract().is_zero() {
            return None;
        }
        Some(Self {
            amount: scaled.to_i64()?,
            currency,
        })
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

    /// Render the amount the way a payment gateway expects it on the wire.
    ///
    /// Unlike [`format`](Self::format) this carries no currency symbol and no
    /// thousands separator — just the decimal amount.
    ///
    /// The decimal places come from [`Currency::decimals`], **not** a hardcoded
    /// `2`. Zero-decimal currencies are already expressed in whole units, so
    /// `format!("{:.2}", ..)` inflates them by a factor of 100 in meaning and
    /// PayPal rejects the request outright with `DECIMALS_NOT_SUPPORTED`:
    /// `Money::new(1000, Currency::JPY)` is ¥1000 and must serialize as
    /// `"1000"`, never `"1000.00"`.
    ///
    /// ```
    /// # use armature_payments::{Money, Currency};
    /// assert_eq!(Money::usd(2999).to_gateway_string(), "29.99");
    /// assert_eq!(Money::new(1000, Currency::JPY).to_gateway_string(), "1000");
    /// ```
    pub fn to_gateway_string(&self) -> String {
        format!(
            "{:.prec$}",
            self.to_decimal(),
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
    fn gateway_amounts_round_trip_exactly() {
        for money in [
            Money::usd(2999),
            Money::usd(0),
            Money::usd(5),
            Money::usd(-2999),
            Money::eur(123456),
            Money::new(1000, Currency::JPY),
            Money::new(0, Currency::KRW),
        ] {
            let rendered = money.to_gateway_string();
            assert_eq!(
                Money::from_gateway_string(&rendered, money.currency),
                Some(money),
                "{rendered} did not round-trip"
            );
        }
    }

    #[test]
    fn gateway_amounts_are_parsed_exactly_not_through_a_float() {
        // 0.29 * 100.0 in binary floating point is 28.999999999999996.
        assert_eq!(
            Money::from_gateway_string("0.29", Currency::USD),
            Some(Money::usd(29))
        );
        assert_eq!(
            Money::from_gateway_string("1.15", Currency::USD),
            Some(Money::usd(115))
        );
        // Zero-decimal currencies are already whole units.
        assert_eq!(
            Money::from_gateway_string("1000", Currency::JPY),
            Some(Money::new(1000, Currency::JPY))
        );
        // Surrounding whitespace is tolerated; anything else is not.
        assert_eq!(
            Money::from_gateway_string(" 12.50 ", Currency::USD),
            Some(Money::usd(1250))
        );
    }

    #[test]
    fn unrepresentable_gateway_amounts_are_rejected_not_zeroed() {
        // The whole point: a malformed amount must never silently become 0.00
        // on a path that reports success.
        for bad in ["", "abc", "12,50", "1.2.3", "NaN", "12 34", "$12.50"] {
            assert_eq!(
                Money::from_gateway_string(bad, Currency::USD),
                None,
                "{bad:?} should not parse"
            );
        }
        // More precision than the currency has minor units.
        assert_eq!(Money::from_gateway_string("29.999", Currency::USD), None);
        assert_eq!(Money::from_gateway_string("1000.50", Currency::JPY), None);
        // ...but trailing zeros within the currency's precision are fine.
        assert_eq!(
            Money::from_gateway_string("29.990", Currency::USD),
            Some(Money::usd(2999))
        );
    }

    #[test]
    fn from_code_is_case_insensitive_and_total() {
        // Every variant must round-trip through its own code, in any casing —
        // the allocation-free comparison must not have dropped an entry.
        for currency in Currency::ALL {
            let code = currency.code();
            assert_eq!(Currency::from_code(code), Some(currency));
            assert_eq!(Currency::from_code(&code.to_lowercase()), Some(currency));
        }
        assert_eq!(Currency::from_code("uSd"), Some(Currency::USD));
        assert_eq!(Currency::from_code("nonsense"), None);
        assert_eq!(Currency::from_code(""), None);
        assert_eq!(Currency::from_code("US"), None);
    }

    #[test]
    fn every_currency_scales_without_overflowing() {
        // The premise `from_gateway_string`'s `checked_pow` guards: a currency
        // added with a precision beyond i64's reach would panic under `pow`.
        for currency in Currency::ALL {
            assert!(
                10i64.checked_pow(currency.decimals()).is_some(),
                "{} has an unrepresentable precision",
                currency.code()
            );
        }
    }

    #[test]
    fn test_currency() {
        assert_eq!(Currency::USD.symbol(), "$");
        assert_eq!(Currency::EUR.symbol(), "€");
        assert_eq!(Currency::JPY.decimals(), 0);
        assert!(Currency::JPY.is_zero_decimal());
    }

    #[test]
    fn gateway_string_uses_two_decimals_for_minor_unit_currencies() {
        assert_eq!(Money::usd(2999).to_gateway_string(), "29.99");
        assert_eq!(Money::usd(0).to_gateway_string(), "0.00");
        assert_eq!(Money::usd(5).to_gateway_string(), "0.05");
        assert_eq!(Money::usd(100).to_gateway_string(), "1.00");
        assert_eq!(Money::eur(123456).to_gateway_string(), "1234.56");
        assert_eq!(Money::gbp(1).to_gateway_string(), "0.01");
    }

    #[test]
    fn gateway_string_emits_no_decimals_for_zero_decimal_currencies() {
        // Regression: format!("{:.2}", ..) produced "1000.00" here, which
        // PayPal rejects with DECIMALS_NOT_SUPPORTED.
        assert_eq!(Money::new(1000, Currency::JPY).to_gateway_string(), "1000");
        assert_eq!(Money::new(0, Currency::JPY).to_gateway_string(), "0");
        assert_eq!(Money::new(1, Currency::JPY).to_gateway_string(), "1");
        assert_eq!(
            Money::new(50000, Currency::KRW).to_gateway_string(),
            "50000"
        );
    }

    #[test]
    fn gateway_string_matches_currency_decimals_for_every_variant() {
        // Guards against a new currency being added with decimals() != 2 while
        // to_gateway_string keeps assuming 2.
        for currency in [
            Currency::USD,
            Currency::EUR,
            Currency::GBP,
            Currency::JPY,
            Currency::KRW,
            Currency::CHF,
            Currency::INR,
        ] {
            let rendered = Money::new(12345, currency).to_gateway_string();
            let fractional = rendered.split_once('.').map_or(0, |(_, f)| f.len());
            assert_eq!(
                fractional,
                currency.decimals() as usize,
                "{} rendered as {rendered}",
                currency.code()
            );
        }
    }

    #[test]
    fn gateway_string_carries_no_symbol_or_separator() {
        let s = Money::usd(123456789).to_gateway_string();
        assert_eq!(s, "1234567.89");
        assert!(!s.contains('$'));
        assert!(!s.contains(','));
    }

    #[test]
    fn gateway_string_handles_negative_amounts() {
        assert_eq!(Money::usd(-2999).to_gateway_string(), "-29.99");
        assert_eq!(
            Money::new(-1000, Currency::JPY).to_gateway_string(),
            "-1000"
        );
    }

    #[test]
    fn test_price_discount() {
        let price = Price::new(Money::usd(2000)).with_compare_at(Money::usd(2500));

        assert!(price.is_on_sale());
        assert!((price.discount_percent().unwrap() - 20.0).abs() < 0.01);
    }
}
