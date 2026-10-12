//! Usage accounting and tool schema identity (§5).
pub use super::super::store::UsageDelta;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
mod wire;
pub(crate) use wire::usage_wire_is_unambiguous;

/// Estimated characters per token, matching the LLM hook's fallback so historic
/// and native numbers stay comparable.
pub const CHARS_PER_TOKEN: i64 = 4;

impl UsageDelta {
    pub fn uncached_input(&self) -> i64 {
        (self.input_tokens - self.cached_input_tokens).max(0)
    }

    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn billable_uncached(&self) -> i64 {
        self.uncached_input() + self.output_tokens
    }

    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn plus(&self, other: &UsageDelta) -> UsageDelta {
        UsageDelta {
            input_tokens: self.input_tokens + other.input_tokens,
            cached_input_tokens: self.cached_input_tokens + other.cached_input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            total_tokens: self.total_tokens + other.total_tokens,
            model_requests: self.model_requests + other.model_requests,
        }
    }
}

// Phase 2/3 consumes this; the rules around it are covered by tests now.
#[allow(dead_code)]
pub fn estimate_tokens(bytes: usize) -> i64 {
    ((bytes as i64 + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN).max(1)
}

/// Parse provider usage, falling back to a wire-size estimate when the server
/// omits it (local servers frequently do).
pub fn usage_from_response(
    body: &JsonValue,
    request_bytes: usize,
    response_bytes: usize,
) -> UsageDelta {
    let usage = body.get("usage");
    let read = |keys: &[&str]| -> Option<i64> {
        keys.iter()
            .find_map(|key| usage?.get(*key).and_then(JsonValue::as_i64))
    };
    let cached = usage
        .and_then(|value| {
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .or_else(|| value.pointer("/input_tokens_details/cached_tokens"))
                .or_else(|| value.get("cached_tokens"))
                .or_else(|| value.get("prompt_cache_hit_tokens"))
        })
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    if let Some(total) = read(&["total_tokens"]) {
        return UsageDelta {
            input_tokens: read(&["prompt_tokens", "input_tokens"]).unwrap_or(0),
            cached_input_tokens: cached,
            output_tokens: read(&["completion_tokens", "output_tokens"]).unwrap_or(0),
            total_tokens: total,
            model_requests: 1,
        };
    }
    UsageDelta {
        input_tokens: estimate_tokens(request_bytes),
        cached_input_tokens: 0,
        output_tokens: estimate_tokens(response_bytes),
        total_tokens: estimate_tokens(request_bytes) + estimate_tokens(response_bytes),
        model_requests: 1,
    }
}

// A valid first alias must not hide another contradictory or malformed bill.
// Preserve the response's original diagnostic numbers elsewhere, but do not
// allow ambiguous usage to settle a durable dispatch reservation.
fn consistent_usage_aliases<'a>(
    values: impl IntoIterator<Item = Option<&'a JsonValue>>,
) -> Result<Option<i64>, ()> {
    let mut found = None;
    for value in values.into_iter().flatten() {
        let amount = value.as_i64().ok_or(())?;
        if found.is_some_and(|original| original != amount) {
            return Err(());
        }
        found = Some(amount);
    }
    Ok(found)
}

pub(crate) fn usage_is_reported(body: &JsonValue) -> bool {
    let Some(usage) = body.get("usage").filter(|value| value.is_object()) else {
        return false;
    };
    let read = |keys: &[&str]| consistent_usage_aliases(keys.iter().map(|key| usage.get(*key)));
    let (Ok(Some(input)), Ok(Some(output)), Ok(Some(total))) = (
        read(&["prompt_tokens", "input_tokens"]),
        read(&["completion_tokens", "output_tokens"]),
        read(&["total_tokens"]),
    ) else {
        return false;
    };
    if ["prompt_tokens_details", "input_tokens_details"]
        .iter()
        .any(|key| {
            usage
                .get(*key)
                .is_some_and(|value| !value.is_object() && !value.is_null())
        })
    {
        return false;
    }
    let Ok(cached) = consistent_usage_aliases([
        usage.pointer("/prompt_tokens_details/cached_tokens"),
        usage.pointer("/input_tokens_details/cached_tokens"),
        usage.get("cached_tokens"),
        usage.get("prompt_cache_hit_tokens"),
    ]) else {
        return false;
    };
    let cached = cached.unwrap_or(0);
    input >= 0
        && output >= 0
        && total >= 0
        && cached >= 0
        && cached <= input
        && input.checked_add(output) == Some(total)
}

pub(crate) fn cached_usage_is_reported(body: &JsonValue) -> bool {
    if !usage_is_reported(body) {
        return false;
    }
    let usage = &body["usage"];
    matches!(consistent_usage_aliases([
        usage.pointer("/prompt_tokens_details/cached_tokens"),
        usage.pointer("/input_tokens_details/cached_tokens"),
        usage.get("cached_tokens"),
        usage.get("prompt_cache_hit_tokens"),
    ]), Ok(Some(_)))
}

/// Compiled-in tool description. Names are static so a target response can
/// never introduce a new capability (§14).
#[derive(Clone, Debug)]
pub struct ToolSchema {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: JsonValue,
    pub schema_version: i64,
}

impl ToolSchema {
    pub fn new(name: &'static str, description: &'static str, parameters: JsonValue) -> Self {
        Self {
            name,
            description,
            parameters,
            schema_version: 1,
        }
    }

    pub fn as_function_spec(&self) -> JsonValue {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

/// A stable hash of the tool contract. It is written once per run into the
/// context instead of letting schema text grow every round (§5 layer 1).
pub fn tool_schema_hash(specs: &[ToolSchema]) -> String {
    let canonical: Vec<JsonValue> = specs
        .iter()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "version": spec.schema_version,
                "parameters": spec.parameters,
            })
        })
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_string(&canonical)
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod reported_usage_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matching_usage_aliases_are_known_and_conflicting_or_malformed_aliases_are_not() {
        let complete = json!({"usage":{
            "prompt_tokens":80,"input_tokens":80,
            "completion_tokens":20,"output_tokens":20,"total_tokens":100,
            "prompt_tokens_details":{"cached_tokens":10},
            "input_tokens_details":{"cached_tokens":10},
            "cached_tokens":10,"prompt_cache_hit_tokens":10
        }});
        assert!(usage_is_reported(&complete));
        for (field, value) in [
            ("input_tokens", json!(800)),
            ("input_tokens", json!("80")),
            ("output_tokens", json!(21)),
            ("cached_tokens", json!(11)),
            ("prompt_cache_hit_tokens", json!(null)),
            ("prompt_tokens_details", json!("unknown")),
            ("input_tokens_details", json!({"cached_tokens":11})),
        ] {
            let mut body = complete.clone();
            body["usage"][field] = value;
            assert!(!usage_is_reported(&body), "{field}");
        }
        let alternate = json!({"usage":{"input_tokens":80,"output_tokens":20,"total_tokens":100}});
        assert!(usage_is_reported(&alternate));
        let input_style = json!({"usage":{"input_tokens":80,"output_tokens":20,"total_tokens":100,"input_tokens_details":{"cached_tokens":10}}});
        assert!(usage_is_reported(&input_style));
        assert_eq!(
            usage_from_response(&input_style, 1, 1).cached_input_tokens,
            10
        );
        let nullable_details = json!({"usage":{"input_tokens":80,"output_tokens":20,"total_tokens":100,"prompt_tokens_details":null}});
        assert!(usage_is_reported(&nullable_details));
        let parsed = usage_from_response(&complete, 1, 1);
        assert_eq!(
            (
                parsed.input_tokens,
                parsed.cached_input_tokens,
                parsed.output_tokens,
                parsed.total_tokens
            ),
            (80, 10, 20, 100)
        );
    }
}
