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

/// Why an external amount could not be turned into a [`Money`].
///
/// Every variant here describes an amount this crate cannot represent
/// faithfully. None of them may be recovered from by substituting a default:
/// the conversions these come from sit on the charge and refund paths, where a
/// zero reported as a success is a charge for nothing that no reconciliation
/// run can detect after the fact.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MoneyError {
    /// The float was `NaN` or an infinity.
    #[error("amount {amount} is not a finite number ({currency})")]
    NotFinite {
        /// The rejected amount, rendered for diagnostics.
        amount: String,
        /// The currency the conversion was attempted in.
        currency: Currency,
    },

    /// The amount scaled to the currency's minor units does not fit in `i64`.
    #[error("amount {amount} does not fit in {currency}'s minor units")]
    Overflow {
        /// The rejected amount, rendered for diagnostics.
        amount: String,
        /// The currency the conversion was attempted in.
        currency: Currency,
    },

    /// The currency declares more decimal places than an `i64` multiplier can
    /// express. Unreachable while every [`Currency::decimals`] is 0 or 2; it
    /// exists so adding an implausible precision surfaces as an error rather
    /// than a panic inside `10i64.pow`.
    #[error("{currency} declares a precision that cannot be represented")]
    UnrepresentablePrecision {
        /// The offending currency.
        currency: Currency,
    },
}

impl MoneyError {
    /// Build an [`MoneyError::Overflow`] from a decimal amount.
    fn overflow(amount: Decimal, currency: Currency) -> Self {
        Self::Overflow {
            amount: amount.to_string(),
            currency,
        }
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

    /// Create from a decimal amount (e.g. `29.99` USD is 2999 cents).
    ///
    /// # Rounding
    ///
    /// Scaling to minor units uses [`Decimal::round`], i.e. **banker's
    /// rounding** (half-to-even): `0.125` USD is 12 cents and `0.135` USD is 14
    /// cents. Sub-minor-unit precision is therefore accepted, not rejected — if
    /// you need a value that cannot be rounded to be an error, use
    /// [`from_gateway_string`](Self::from_gateway_string) instead.
    ///
    /// # Errors
    ///
    /// [`MoneyError::Overflow`] if the amount scaled to minor units does not fit
    /// in an `i64`. That case previously produced **zero**, which on a payment
    /// path is a charge for nothing reported as a success.
    pub fn try_from_decimal(amount: Decimal, currency: Currency) -> Result<Self, MoneyError> {
        use rust_decimal::prelude::ToPrimitive;

        let multiplier = 10i64
            .checked_pow(currency.decimals())
            .ok_or(MoneyError::UnrepresentablePrecision { currency })?;
        let minor = amount
            .checked_mul(Decimal::from(multiplier))
            .ok_or_else(|| MoneyError::overflow(amount, currency))?
            .round()
            .to_i64()
            .ok_or_else(|| MoneyError::overflow(amount, currency))?;

        Ok(Self {
            amount: minor,
            currency,
        })
    }

    /// Create from a binary float amount.
    ///
    /// Prefer [`try_from_decimal`](Self::try_from_decimal): a binary float
    /// cannot represent most decimal money amounts exactly, so `0.29 * 100.0` is
    /// 28.999999999999996 and only lands on 29 cents because the rounding
    /// happens to go the right way.
    ///
    /// # Rounding
    ///
    /// Scaling to minor units uses [`f64::round`], i.e. **half away from zero**
    /// — which differs from [`try_from_decimal`](Self::try_from_decimal)'s
    /// banker's rounding at exact halfway points.
    ///
    /// # Errors
    ///
    /// [`MoneyError::NotFinite`] for `NaN` and the infinities, and
    /// [`MoneyError::Overflow`] when the scaled amount is outside `i64`. Both
    /// previously produced a silent zero (`NaN as i64` saturates to 0, and
    /// out-of-range floats saturate to `i64::MIN`/`i64::MAX`).
    pub fn try_from_float(amount: f64, currency: Currency) -> Result<Self, MoneyError> {
        if !amount.is_finite() {
            return Err(MoneyError::NotFinite {
                amount: amount.to_string(),
                currency,
            });
        }

        let multiplier = 10i64
            .checked_pow(currency.decimals())
            .ok_or(MoneyError::UnrepresentablePrecision { currency })?;
        let scaled = (amount * multiplier as f64).round();

        // `i64::MAX as f64` rounds *up* to 2^63, which is one past the last
        // representable i64, so the upper bound must be exclusive. `i64::MIN as
        // f64` is exactly -2^63 and stays inclusive.
        if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled >= i64::MAX as f64 {
            return Err(MoneyError::Overflow {
                amount: amount.to_string(),
                currency,
            });
        }

        Ok(Self {
            amount: scaled as i64,
            currency,
        })
    }

    /// Create from decimal amount (e.g., 29.99)
    ///
    /// # Panics
    ///
    /// Panics if the amount cannot be represented — see
    /// [`try_from_decimal`](Self::try_from_decimal), which reports the same
    /// condition as an error instead.
    #[deprecated(
        since = "0.2.0",
        note = "returns a panic where it used to silently produce 0; use Money::try_from_decimal"
    )]
    pub fn from_decimal(amount: Decimal, currency: Currency) -> Self {
        Self::try_from_decimal(amount, currency).expect("money amount is not representable")
    }

    /// Create from float amount
    ///
    /// # Panics
    ///
    /// Panics if the amount cannot be represented — see
    /// [`try_from_float`](Self::try_from_float), which reports the same
    /// condition as an error instead.
    #[deprecated(
        since = "0.2.0",
        note = "returns a panic where it used to silently produce 0; use Money::try_from_float"
    )]
    pub fn from_float(amount: f64, currency: Currency) -> Self {
        Self::try_from_float(amount, currency).expect("money amount is not representable")
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
    ///
    /// `false` when `compare_at` is in a different currency than `amount`.
    /// Minor units are only comparable within one currency: ¥2500 against
    /// $20.00 is 2500 against 2000 in raw units, which would report a sale on a
    /// price that is roughly four times higher.
    pub fn is_on_sale(&self) -> bool {
        self.compare_at.is_some_and(|compare| {
            compare.currency == self.amount.currency && compare.amount > self.amount.amount
        })
    }

    /// Get discount percentage
    ///
    /// `None` when there is no compare-at price, or when it is denominated in a
    /// different currency than `amount` — see [`is_on_sale`](Self::is_on_sale)
    /// for why a cross-currency comparison of minor units is meaningless. This
    /// crate carries no exchange rates, so converting is not an option.
    pub fn discount_percent(&self) -> Option<f64> {
        let compare = self.compare_at?;
        if compare.currency != self.amount.currency {
            return None;
        }
        if compare.amount <= 0 {
            return Some(0.0);
        }
        Some(((compare.amount - self.amount.amount) as f64 / compare.amount as f64) * 100.0)
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
        let money = Money::try_from_float(29.99, Currency::USD).unwrap();
        assert_eq!(money.amount, 2999);
    }

    #[test]
    fn try_from_decimal_scales_to_minor_units() {
        assert_eq!(
            Money::try_from_decimal(Decimal::from_str_exact("29.99").unwrap(), Currency::USD),
            Ok(Money::usd(2999))
        );
        assert_eq!(
            Money::try_from_decimal(Decimal::from_str_exact("-29.99").unwrap(), Currency::USD),
            Ok(Money::usd(-2999))
        );
        // Zero-decimal currencies are already whole units.
        assert_eq!(
            Money::try_from_decimal(Decimal::from_str_exact("1000").unwrap(), Currency::JPY),
            Ok(Money::new(1000, Currency::JPY))
        );
        // Banker's rounding (half-to-even) at exact halfway points.
        assert_eq!(
            Money::try_from_decimal(Decimal::from_str_exact("0.125").unwrap(), Currency::USD),
            Ok(Money::usd(12))
        );
        assert_eq!(
            Money::try_from_decimal(Decimal::from_str_exact("0.135").unwrap(), Currency::USD),
            Ok(Money::usd(14))
        );
    }

    #[test]
    fn try_from_decimal_reports_overflow_instead_of_zero() {
        // The regression: `.parse().unwrap_or(0)` turned every one of these into
        // a charge for nothing that reported success.
        for huge in ["100000000000000000000", "-100000000000000000000"] {
            let amount = Decimal::from_str_exact(huge).unwrap();
            assert!(
                matches!(
                    Money::try_from_decimal(amount, Currency::USD),
                    Err(MoneyError::Overflow { .. })
                ),
                "{huge} must not silently become 0"
            );
        }
        // Scaling itself overflows Decimal, before the i64 conversion.
        assert!(matches!(
            Money::try_from_decimal(Decimal::MAX, Currency::USD),
            Err(MoneyError::Overflow { .. })
        ));
    }

    #[test]
    fn try_from_float_rejects_nan_and_infinities() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(
                    Money::try_from_float(bad, Currency::USD),
                    Err(MoneyError::NotFinite { .. })
                ),
                "{bad} must not silently become 0"
            );
        }
    }

    #[test]
    fn try_from_float_reports_overflow_instead_of_saturating() {
        // `as i64` saturates at the boundaries, so these used to produce
        // i64::MAX / i64::MIN worth of cents rather than an error.
        for huge in [1e30_f64, -1e30_f64, 1e17_f64] {
            assert!(
                matches!(
                    Money::try_from_float(huge, Currency::USD),
                    Err(MoneyError::Overflow { .. })
                ),
                "{huge} must not saturate"
            );
        }
        // Just inside the range still converts.
        assert!(Money::try_from_float(1e10, Currency::USD).is_ok());
    }

    #[test]
    #[should_panic(expected = "money amount is not representable")]
    #[allow(deprecated)]
    fn deprecated_from_float_panics_rather_than_returning_zero() {
        let _ = Money::from_float(f64::NAN, Currency::USD);
    }

    #[test]
    #[should_panic(expected = "money amount is not representable")]
    #[allow(deprecated)]
    fn deprecated_from_decimal_panics_rather_than_returning_zero() {
        let _ = Money::from_decimal(Decimal::MAX, Currency::USD);
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

    #[test]
    fn price_comparison_requires_a_matching_currency() {
        // ¥2500 and $20.00 are 2500 and 2000 in raw minor units, so comparing
        // them reported a 20% discount on a price that is actually far higher.
        let price = Price::new(Money::usd(2000)).with_compare_at(Money::new(2500, Currency::JPY));

        assert!(
            !price.is_on_sale(),
            "cross-currency compare-at is not a sale"
        );
        assert_eq!(price.discount_percent(), None);

        // The same-currency case is unchanged.
        let same = Price::new(Money::usd(2000)).with_compare_at(Money::usd(2500));
        assert!(same.is_on_sale());
        assert!((same.discount_percent().unwrap() - 20.0).abs() < 0.01);

        // No compare-at at all remains "not on sale" / `None`.
        let plain = Price::new(Money::usd(2000));
        assert!(!plain.is_on_sale());
        assert_eq!(plain.discount_percent(), None);
    }
}
