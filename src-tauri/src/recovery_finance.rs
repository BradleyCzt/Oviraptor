//! Exact approved-rate arithmetic for the future dedicated Recovery contract.
//! These pure values do not verify an approval, provider invoice or usage receipt,
//! write a ledger, reserve capacity, or grant execution. Consumers must supply
//! independently verified original bindings before using a quote for settlement.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const NANOS_PER_USD: i64 = 1_000_000_000;
const TOKENS_PER_RATE_UNIT: i128 = 1_000_000;

/// Nonnegative nano-USD, bounded by the SQLite signed integer domain.
/// JSON uses a canonical integer string; floating point is never accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Self = Self(0);

    pub fn from_nanos(value: i64) -> Result<Self, &'static str> {
        if value < 0 {
            return Err("recovery_money_negative");
        }
        Ok(Self(value))
    }

    pub fn nanos(self) -> i64 {
        self.0
    }

    /// Exact decimal USD input: no whitespace, exponent, signs, leading zeros,
    /// omitted integer/fraction part, or more than nine fractional digits.
    /// Zero is valid arithmetic; an approval's positive limit is checked separately.
    pub fn parse_usd(value: &str) -> Result<Self, &'static str> {
        let (whole, fraction) = match value.split_once('.') {
            Some((whole, fraction)) => (whole, Some(fraction)),
            None => (value, None),
        };
        let whole = parse_integer(whole)?;
        let mut amount = whole
            .checked_mul(NANOS_PER_USD)
            .ok_or("recovery_money_overflow")?;
        if let Some(fraction) = fraction {
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|v| v.is_ascii_digit())
            {
                return Err("recovery_money_decimal_invalid");
            }
            let decimal = fraction
                .parse::<i64>()
                .map_err(|_| "recovery_money_decimal_invalid")?;
            let scale = 10_i64.pow(9 - fraction.len() as u32);
            amount = amount
                .checked_add(
                    decimal
                        .checked_mul(scale)
                        .ok_or("recovery_money_overflow")?,
                )
                .ok_or("recovery_money_overflow")?;
        }
        Self::from_nanos(amount)
    }

    pub fn usd(self) -> String {
        let whole = self.0 / NANOS_PER_USD;
        let fraction = self.0 % NANOS_PER_USD;
        if fraction == 0 {
            return whole.to_string();
        }
        format!("{whole}.{}", format!("{fraction:09}").trim_end_matches('0'))
    }
}

fn parse_integer(value: &str) -> Result<i64, &'static str> {
    if value.len() > 19 {
        return Err("recovery_money_overflow");
    }
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|v| v.is_ascii_digit())
    {
        return Err("recovery_money_integer_invalid");
    }
    value.parse().map_err(|_| "recovery_money_overflow")
}

impl Serialize for Money {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Money {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        parse_integer(&value)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

/// Usage for exactly one model call. This value alone is not a paid receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub model_requests: i64,
}

/// Revision-bound user-approved pricing, rather than a provider dollar invoice.
/// Token rates use nano-USD per million tokens; the fixed charge is per call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "RateCardWire",
    into = "RateCardWire"
)]
pub struct RateCard {
    wire: RateCardWire,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RateCardWire {
    schema_version: u8,
    basis: String,
    token_rate_unit: String,
    cached_semantics: String,
    rounding: String,
    model_binding_hash: String,
    price_revision: String,
    uncached_input: Money,
    cached_input: Money,
    output: Money,
    per_request: Money,
}

impl TryFrom<RateCardWire> for RateCard {
    type Error = &'static str;
    fn try_from(wire: RateCardWire) -> Result<Self, Self::Error> {
        if wire.schema_version != 1
            || wire.basis != "approved_rate_card"
            || wire.token_rate_unit != "nano_usd_per_million_tokens"
            || wire.cached_semantics != "subset_of_input"
            || wire.rounding != "ceil_total_per_model_call"
            || wire.model_binding_hash.len() != 64
            || !wire
                .model_binding_hash
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            || wire.price_revision.is_empty()
            || wire.price_revision.len() > 80
            || !wire
                .price_revision
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
        {
            return Err("recovery_pricing_contract_invalid");
        }
        Ok(Self { wire })
    }
}
impl From<RateCard> for RateCardWire {
    fn from(value: RateCard) -> Self {
        value.wire
    }
}

impl RateCard {
    pub fn model_binding_hash(&self) -> &str {
        &self.wire.model_binding_hash
    }

    pub fn new(
        model_binding_hash: String,
        price_revision: String,
        uncached_input: Money,
        cached_input: Money,
        output: Money,
        per_request: Money,
    ) -> Result<Self, &'static str> {
        RateCardWire {
            schema_version: 1,
            basis: "approved_rate_card".into(),
            token_rate_unit: "nano_usd_per_million_tokens".into(),
            cached_semantics: "subset_of_input".into(),
            rounding: "ceil_total_per_model_call".into(),
            model_binding_hash,
            price_revision,
            uncached_input,
            cached_input,
            output,
            per_request,
        }
        .try_into()
    }

    /// Price an independently established per-call input/output upper bound.
    /// This method does not derive or prove the bound from bytes, messages,
    /// tokenizer estimates or provider configuration. An adapter must do that
    /// before a consumer may use this amount for reservation.
    pub fn quote_upper_bound(
        &self,
        model_binding_hash: &str,
        input_upper: i64,
        output_upper: i64,
    ) -> Result<Money, &'static str> {
        let total_tokens = input_upper
            .checked_add(output_upper)
            .ok_or("recovery_pricing_bound_invalid")?;
        if input_upper < 0 || output_upper < 0 {
            return Err("recovery_pricing_bound_invalid");
        }
        // Caching need not be cheaper in an arbitrary explicitly approved card.
        // Use the more expensive input class until the original usage is known.
        let cached_input_tokens = if self.wire.cached_input > self.wire.uncached_input {
            input_upper
        } else {
            0
        };
        self.quote(
            model_binding_hash,
            ModelUsage {
                input_tokens: input_upper,
                cached_input_tokens,
                output_tokens: output_upper,
                total_tokens,
                model_requests: 1,
            },
        )
    }

    /// Quote only provider-reported usage. These transport flags are not an
    /// authenticated receipt: settlement must independently verify the original
    /// call, model, scope and approved rate card before consuming this result.
    pub fn quote_reported_response(
        &self,
        model_binding_hash: &str,
        response: &crate::agent_runtime::model::gateway::ModelResponse,
    ) -> Result<Money, &'static str> {
        if !response.usage_reported {
            return Err("recovery_pricing_usage_unreported");
        }
        if !response.cached_usage_reported
            && response.usage.input_tokens != 0
            && self.wire.cached_input != self.wire.uncached_input
        {
            return Err("recovery_pricing_cached_usage_unreported");
        }
        let usage = &response.usage;
        self.quote(model_binding_hash, ModelUsage {
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            model_requests: usage.model_requests,
        })
    }

    /// Cached input is deducted from input before the uncached rate is applied.
    /// Round up once per original model call, never after aggregating many calls.
    pub fn quote(
        &self,
        model_binding_hash: &str,
        usage: ModelUsage,
    ) -> Result<Money, &'static str> {
        if model_binding_hash != self.wire.model_binding_hash {
            return Err("recovery_pricing_model_binding_changed");
        }
        if usage.input_tokens < 0
            || usage.cached_input_tokens < 0
            || usage.output_tokens < 0
            || usage.cached_input_tokens > usage.input_tokens
            || usage.model_requests != 1
            || usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens)
        {
            return Err("recovery_pricing_usage_invalid");
        }
        let mut numerator = 0_i128;
        for (tokens, rate) in [
            (
                usage.input_tokens - usage.cached_input_tokens,
                self.wire.uncached_input,
            ),
            (usage.cached_input_tokens, self.wire.cached_input),
            (usage.output_tokens, self.wire.output),
        ] {
            numerator = numerator
                .checked_add(
                    i128::from(tokens)
                        .checked_mul(i128::from(rate.0))
                        .ok_or("recovery_money_overflow")?,
                )
                .ok_or("recovery_money_overflow")?;
        }
        let token_charge =
            numerator / TOKENS_PER_RATE_UNIT + i128::from(numerator % TOKENS_PER_RATE_UNIT != 0);
        let charge = token_charge
            .checked_add(i128::from(self.wire.per_request.0))
            .ok_or("recovery_money_overflow")?;
        Money::from_nanos(i64::try_from(charge).map_err(|_| "recovery_money_overflow")?)
    }
}

/// Exact versioned financial terms for a new explicit Recovery approval.
/// This is untrusted contract data until the application binds and signs it
/// with the original approval/model proof. It is never an execution capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "FinancialTermsWire",
    into = "FinancialTermsWire"
)]
pub struct FinancialTerms {
    wire: FinancialTermsWire,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinancialTermsWire {
    schema_version: u8,
    max_nano_usd: Money,
    pricing: RateCard,
}

impl TryFrom<FinancialTermsWire> for FinancialTerms {
    type Error = &'static str;
    fn try_from(wire: FinancialTermsWire) -> Result<Self, Self::Error> {
        if wire.schema_version != 1
            || wire.max_nano_usd == Money::ZERO
            || wire.max_nano_usd.nanos() > 10_000 * NANOS_PER_USD
        {
            return Err("recovery_financial_terms_invalid");
        }
        Ok(Self { wire })
    }
}
impl From<FinancialTerms> for FinancialTermsWire {
    fn from(value: FinancialTerms) -> Self {
        value.wire
    }
}
impl FinancialTerms {
    pub fn new(max_nano_usd: Money, pricing: RateCard) -> Result<Self, &'static str> {
        FinancialTermsWire {
            schema_version: 1,
            max_nano_usd,
            pricing,
        }
        .try_into()
    }
    pub fn limit(&self) -> Money {
        self.wire.max_nano_usd
    }
    pub fn pricing(&self) -> &RateCard {
        &self.wire.pricing
    }
}

#[cfg(test)]
mod tests;
