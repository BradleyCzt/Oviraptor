use super::*;

#[test]
fn reported_response_pricing_requires_cache_provenance_when_prices_differ() {
    use crate::agent_runtime::{model::gateway::ModelResponse, store::UsageDelta};
    let mut response = ModelResponse {
        usage: UsageDelta { input_tokens: 100, cached_input_tokens: 0, output_tokens: 20, total_tokens: 120, model_requests: 1 },
        usage_reported: true,
        ..Default::default()
    };
    let binding = "a".repeat(64);
    for pricing in [card(2_000_000_000, 500_000_000, 4_000_000_000, 10_000_000), card(500_000_000, 2_000_000_000, 4_000_000_000, 10_000_000)] {
        assert_eq!(pricing.quote_reported_response(&binding, &response).unwrap_err(), "recovery_pricing_cached_usage_unreported");
        response.cached_usage_reported = true;
        assert_eq!(pricing.quote_reported_response(&binding, &response).unwrap(), pricing.quote(&binding, usage(100, 0, 20)).unwrap());
        response.usage_reported = false;
        assert_eq!(pricing.quote_reported_response(&binding, &response).unwrap_err(), "recovery_pricing_usage_unreported");
        response.usage_reported = true;
        response.cached_usage_reported = false;
    }
    let equal = card(2_000_000_000, 2_000_000_000, 4_000_000_000, 10_000_000);
    assert_eq!(equal.quote_reported_response(&binding, &response).unwrap(), equal.quote(&binding, usage(100, 0, 20)).unwrap());
    response.usage.total_tokens = 119;
    assert_eq!(equal.quote_reported_response(&binding, &response).unwrap_err(), "recovery_pricing_usage_invalid");
    response.usage.total_tokens = 120;
    assert_eq!(equal.quote_reported_response(&"b".repeat(64), &response).unwrap_err(), "recovery_pricing_model_binding_changed");
    // A verified zero input has no cache subset to price; this does not turn
    // absent cache provenance into an explicit provider report.
    response.usage.input_tokens = 0;
    response.usage.total_tokens = 20;
    let unequal = card(2_000_000_000, 500_000_000, 4_000_000_000, 10_000_000);
    assert_eq!(unequal.quote_reported_response(&binding, &response).unwrap().nanos(), 10_080_000);
    assert!(!response.cached_usage_reported);
}

#[test]
fn financial_terms_require_explicit_exact_positive_limit_and_known_version() {
    assert!(FinancialTerms::new(Money::ZERO, card(1, 1, 1, 0)).is_err());
    assert!(FinancialTerms::new(
        Money::parse_usd("10000.000000001").unwrap(),
        card(1, 1, 1, 0)
    )
    .is_err());
    let terms =
        FinancialTerms::new(Money::parse_usd("0.000000001").unwrap(), card(1, 1, 1, 0)).unwrap();
    assert_eq!(terms.limit().nanos(), 1);
    let raw = serde_json::to_value(&terms).unwrap();
    assert_eq!(raw["maxNanoUsd"], "1");
    assert_eq!(
        serde_json::from_value::<FinancialTerms>(raw.clone()).unwrap(),
        terms
    );
    for (key, value) in [
        ("schemaVersion", serde_json::json!(2)),
        ("maxNanoUsd", serde_json::json!(1)),
        ("maxNanoUsd", serde_json::json!("0")),
        ("executionAllowed", serde_json::json!(true)),
    ] {
        let mut bad = raw.clone();
        bad[key] = value;
        assert!(
            serde_json::from_value::<FinancialTerms>(bad).is_err(),
            "{key}"
        );
    }
}

fn card(input: i64, cached: i64, output: i64, fixed: i64) -> RateCard {
    RateCard::new(
        "a".repeat(64),
        "test-2026.10.08".into(),
        Money(input),
        Money(cached),
        Money(output),
        Money(fixed),
    )
    .unwrap()
}
fn usage(input: i64, cached: i64, output: i64) -> ModelUsage {
    ModelUsage {
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
        total_tokens: input + output,
        model_requests: 1,
    }
}

#[test]
fn decimal_usd_is_exact_bounded_and_never_rounded_into_more_permission() {
    for (text, nanos) in [
        ("0", 0),
        ("0.000000001", 1),
        ("1.000000010", 1_000_000_010),
        ("10000", 10_000_000_000_000),
        ("9223372036.854775807", i64::MAX),
    ] {
        let money = Money::parse_usd(text).unwrap();
        assert_eq!(money.nanos(), nanos);
        assert_eq!(Money::parse_usd(&money.usd()).unwrap(), money);
    }
    for text in [
        "",
        " 1",
        "1 ",
        "+1",
        "-1",
        "01",
        "00.1",
        ".1",
        "1.",
        "1e2",
        "1.2.3",
        "0.0000000001",
        "9223372036.854775808",
        "9223372037",
        "１",
        "NaN",
        "Infinity",
    ] {
        assert!(Money::parse_usd(text).is_err(), "{text}");
    }
    assert!(Money::from_nanos(-1).is_err());
}

#[test]
fn money_wire_requires_canonical_integer_strings_including_beyond_js_safe_integer() {
    let money = Money::from_nanos(i64::MAX).unwrap();
    let raw = serde_json::to_string(&money).unwrap();
    assert_eq!(raw, "\"9223372036854775807\"");
    assert_eq!(serde_json::from_str::<Money>(&raw).unwrap(), money);
    for raw in [
        "1",
        "1.0",
        "null",
        "true",
        "\"01\"",
        "\"+1\"",
        "\"1e2\"",
        "\"1.0\"",
        "\"9223372036854775808\"",
    ] {
        assert!(serde_json::from_str::<Money>(raw).is_err(), "{raw}");
    }
}

#[test]
fn cached_subset_pricing_and_fixed_call_charge_are_exact() {
    // $2/M uncached, $0.50/M cached, $4/M output, $0.01/call.
    let pricing = card(2_000_000_000, 500_000_000, 4_000_000_000, 10_000_000);
    assert_eq!(
        pricing
            .quote(&"a".repeat(64), usage(1_000_000, 400_000, 100_000))
            .unwrap()
            .nanos(),
        1_810_000_000
    );
    assert_eq!(
        pricing
            .quote(&"a".repeat(64), usage(0, 0, 0))
            .unwrap()
            .nanos(),
        10_000_000
    );
    assert_eq!(
        card(0, 0, 0, 0)
            .quote(&"a".repeat(64), usage(10, 5, 20))
            .unwrap(),
        Money::ZERO
    );
}

#[test]
fn rounding_is_up_per_original_call_not_per_token_class_or_aggregate() {
    let pricing = card(1, 1, 1, 0);
    assert_eq!(
        pricing
            .quote(&"a".repeat(64), usage(2, 1, 1))
            .unwrap()
            .nanos(),
        1
    );
    let mut separate = 0;
    for _ in 0..3 {
        separate += pricing
            .quote(&"a".repeat(64), usage(1, 0, 0))
            .unwrap()
            .nanos();
    }
    assert_eq!(separate, 3);
    let mut combined = usage(3, 0, 0);
    combined.model_requests = 3;
    assert!(
        pricing.quote(&"a".repeat(64), combined).is_err(),
        "cannot substitute aggregate rounding for original calls"
    );
}

#[test]
fn malformed_usage_model_binding_and_overflow_never_quote_zero() {
    let pricing = card(1, 1, 1, 0);
    for bad in [
        usage(-1, 0, 1),
        usage(1, -1, 0),
        usage(1, 2, 0),
        usage(1, 0, -1),
        ModelUsage {
            total_tokens: 2,
            ..usage(1, 0, 0)
        },
        ModelUsage {
            model_requests: 0,
            ..usage(1, 0, 0)
        },
        ModelUsage {
            input_tokens: i64::MAX,
            output_tokens: 1,
            total_tokens: i64::MAX,
            ..usage(1, 0, 0)
        },
    ] {
        assert_eq!(
            pricing.quote(&"a".repeat(64), bad).unwrap_err(),
            "recovery_pricing_usage_invalid"
        );
    }
    assert_eq!(
        pricing.quote(&"b".repeat(64), usage(1, 0, 0)).unwrap_err(),
        "recovery_pricing_model_binding_changed"
    );
    assert_eq!(
        card(i64::MAX, 0, i64::MAX, i64::MAX)
            .quote(&"a".repeat(64), usage(i64::MAX, 0, 0))
            .unwrap_err(),
        "recovery_money_overflow"
    );
}

#[test]
fn independently_proven_bounds_cover_every_cached_split_including_expensive_cache() {
    let binding = "a".repeat(64);
    for pricing in [
        card(2_000_000_000, 500_000_000, 4_000_000_000, 10),
        card(1, 3, 7, 1),
        card(0, 0, 0, 0),
    ] {
        let upper = pricing.quote_upper_bound(&binding, 12, 8).unwrap();
        for input in 0..=12 {
            for cached in 0..=input {
                for output in 0..=8 {
                    assert!(
                        pricing
                            .quote(&binding, usage(input, cached, output))
                            .unwrap()
                            <= upper
                    );
                }
            }
        }
        assert!(pricing.quote_upper_bound(&binding, -1, 8).is_err());
        assert!(pricing.quote_upper_bound(&binding, 12, -1).is_err());
        assert!(pricing.quote_upper_bound(&binding, i64::MAX, 1).is_err());
        assert!(pricing.quote_upper_bound(&"b".repeat(64), 12, 8).is_err());
    }
}

#[test]
fn frozen_rate_card_roundtrip_rejects_future_semantics_and_unknown_fields() {
    let pricing = card(2, 1, 4, 3);
    let raw = serde_json::to_value(&pricing).unwrap();
    assert_eq!(
        serde_json::from_value::<RateCard>(raw.clone()).unwrap(),
        pricing
    );
    for (field, value) in [
        ("schemaVersion", serde_json::json!(2)),
        ("basis", serde_json::json!("provider_invoice")),
        ("tokenRateUnit", serde_json::json!("usd_per_token")),
        ("cachedSemantics", serde_json::json!("additional_input")),
        ("rounding", serde_json::json!("nearest")),
        ("modelBindingHash", serde_json::json!("A".repeat(64))),
        ("priceRevision", serde_json::json!("")),
        ("uncachedInput", serde_json::json!(0)),
        ("executionAllowed", serde_json::json!(true)),
    ] {
        let mut bad = raw.clone();
        bad[field] = value;
        assert!(serde_json::from_value::<RateCard>(bad).is_err(), "{field}");
    }
}
