//! Secret redaction (§6, §19).
//!
//! Cookies, JWTs and API keys must never reach a prompt, an event, an agent
//! message or an artifact index. Everything the runtime persists goes through
//! here first.
use serde_json::{Map, Value as JsonValue};

/// Field names whose values must never be stored or forwarded verbatim (§6.2).
const SECRET_FIELDS: [&str; 40] = [
    "fofakey",
    "cookie",
    "setcookie",
    "cookiejar",
    "authorization",
    "proxyauthorization",
    "apikey",
    "apisecret",
    "accesskey",
    "accesskeysecret",
    "secretkey",
    "clientsecret",
    "xapikey",
    "openaiapikey",
    "authorizationcode",
    "authcode",
    "oauthcode",
    "token",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "sessiontoken",
    "authtoken",
    "bearertoken",
    "xsrftoken",
    "csrftoken",
    "xsrf",
    "csrf",
    "password",
    "passwd",
    "pwd",
    "passwordconfirmation",
    "secret",
    "jwt",
    "session",
    "sessionid",
    "sid",
    "jsessionid",
    "phpsessid",
    "aspnetsession",
];

/// Suffixes that make any key a credential, once separators are ignored.
const SECRET_SUFFIXES: [&str; 13] = [
    "apikey",
    "apisecret",
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "cookie",
    "credential",
    "privatekey",
    "sessionid",
    "session",
    "csrf",
];

/// Normalize `X-Api-Key`, `x_api_key` and `apiKey` into one comparable form.
fn normalize_key(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '_' && *character != '-' && *character != ' ')
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

pub fn is_secret_field(name: &str) -> bool {
    let normalized = normalize_key(name);
    if normalized.is_empty() {
        return false;
    }
    SECRET_FIELDS.contains(&normalized.as_str())
        || SECRET_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
}

/// Per-run salt. Within one run the same secret must produce the same marker so
/// A/B and replay comparisons still work; across runs the marker must not be
/// linkable, so the salt is folded into every fingerprint (§6.2).
#[derive(Clone, Debug)]
pub struct RedactionContext {
    salt: String,
}

impl Default for RedactionContext {
    fn default() -> Self {
        Self::new("run")
    }
}

impl RedactionContext {
    pub fn new(salt: impl Into<String>) -> Self {
        Self { salt: salt.into() }
    }

    /// `<redacted:kind:fingerprint>` — the kind keeps short enums and roles usable
    /// for reasoning, the fingerprint keeps them comparable.
    pub fn marker(&self, kind: &str, raw: &str) -> String {
        let digest = super::store::stable_hash(&format!("{}\u{1}{raw}", self.salt));
        format!("<redacted:{kind}:{}>", &digest[..12])
    }

    /// Unsalted marker, for durable audit rows where correlation is a feature.
    pub fn durable_marker(kind: &str, raw: &str) -> String {
        let digest = super::store::stable_hash(raw);
        format!("<redacted:{kind}:{}>", &digest[..12])
    }
}

pub fn redact_json(value: &JsonValue) -> JsonValue {
    redact_json_with(value, None)
}

/// Recursively replace credential values, with a run salt when the result is
/// going to a model. Both `{"cookie": "..."}` and header lists shaped like
/// `[{"name":"Cookie","value":"..."}]` are covered.
pub fn redact_json_with(value: &JsonValue, context: Option<&RedactionContext>) -> JsonValue {
    match value {
        JsonValue::Object(map) => {
            let mut out = Map::new();
            for (key, item) in map {
                let named_secret = item
                    .as_object()
                    .and_then(|object| object.get("name"))
                    .and_then(JsonValue::as_str)
                    .is_some_and(|name| !name.is_empty() && is_secret_field(name));
                let value_key = item
                    .as_object()
                    .and_then(|object| object.get("value"))
                    .map(|inner| redact_json_with(inner, context))
                    .filter(|_| named_secret);
                let entry = match value_key {
                    Some(redacted) => {
                        let mut cloned = item.clone();
                        if let Some(object) = cloned.as_object_mut() {
                            object.insert("value".to_string(), redacted);
                        }
                        cloned
                    }
                    None if is_secret_field(key) => match item {
                        JsonValue::String(text) => {
                            JsonValue::String(marker_for(context, "auth", text))
                        }
                        other => redact_json_with(other, context),
                    },
                    None => redact_json_with(item, context),
                };
                out.insert(key.clone(), entry);
            }
            JsonValue::Object(out)
        }
        JsonValue::Array(rows) => JsonValue::Array(
            rows.iter()
                .map(|row| redact_json_with(row, context))
                .collect(),
        ),
        JsonValue::String(text) => JsonValue::String(redact_text_with(text, context)),
        other => other.clone(),
    }
}

fn marker_for(context: Option<&RedactionContext>, kind: &str, raw: &str) -> String {
    context
        .map(|value| value.marker(kind, raw))
        .unwrap_or_else(|| RedactionContext::durable_marker(kind, raw))
}

/// Scrub credential-shaped material out of free text: dumped headers, bearer
/// tokens, JWTs, provider keys and personal values quoted back inside an error
/// body.
pub fn redact_text_with(text: &str, context: Option<&RedactionContext>) -> String {
    let mut output = scrub_url_userinfo(text, context);
    output = scrub_header_values(&output, context);
    output = scrub_bearer_phrases(&output, context);
    output = scrub_tokens(&output, context, looks_like_jwt, "assertion");
    output = scrub_tokens(&output, context, looks_like_bearer, "bearer");
    output = scrub_tokens(&output, context, looks_like_api_key, "opaque");
    output = scrub_pairs(&output, context);
    scrub_pii(&output, context)
}

// A bearer scheme and its credential are separate whitespace tokens; the
// single-token scrubber below cannot see both when they appear in prose.
fn scrub_bearer_phrases(text: &str, context: Option<&RedactionContext>) -> String {
    let lower = text.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find("bearer ") {
        let start = cursor + offset;
        let token_start = start + "bearer ".len();
        let token_end = text[token_start..]
            .char_indices()
            .find(|(_, character)| {
                character.is_whitespace() || ",;\"'<>{}()[]".contains(*character)
            })
            .map(|(offset, _)| token_start + offset)
            .unwrap_or(text.len());
        let boundary = start == 0
            || !text[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        if boundary && looks_like_bearer(&text[start..token_end]) {
            result.push_str(&text[cursor..token_start]);
            result.push_str(&marker_for(
                context,
                "bearer",
                &text[token_start..token_end],
            ));
            cursor = token_end;
        } else {
            result.push_str(&text[cursor..token_start]);
            cursor = token_start;
        }
    }
    result.push_str(&text[cursor..]);
    result
}

/// Credentials embedded before a URL's `@` are not key=value pairs. Scrub
/// them before the general token rules can persist or forward the URL.
fn scrub_url_userinfo(text: &str, context: Option<&RedactionContext>) -> String {
    let lower = text.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = ["http://", "https://"]
        .into_iter()
        .filter_map(|scheme| lower[cursor..].find(scheme))
        .min()
    {
        let start = cursor + relative;
        let scheme_end = start
            + if lower[start..].starts_with("https://") {
                8
            } else {
                7
            };
        let authority_end = text[scheme_end..]
            .char_indices()
            .find(|(_, character)| {
                character.is_whitespace() || "/?#<>'\"()[]{}，。；、,;".contains(*character)
            })
            .map(|(offset, _)| scheme_end + offset)
            .unwrap_or(text.len());
        output.push_str(&text[cursor..scheme_end]);
        if let Some(at) = text[scheme_end..authority_end].rfind('@') {
            let credential_end = scheme_end + at;
            output.push_str(&marker_for(
                context,
                "auth",
                &text[scheme_end..credential_end],
            ));
            output.push_str(&text[credential_end..authority_end]);
        } else {
            output.push_str(&text[scheme_end..authority_end]);
        }
        cursor = authority_end;
    }
    output.push_str(&text[cursor..]);
    output
}

/// `password=...`, `"token": "..."` and `?sid=...` forms inside a body, a query,
/// a form post or a dumped header block: the key decides, the value is
/// replaced (§6.2).
fn scrub_pairs(text: &str, context: Option<&RedactionContext>) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(offset) = rest.find(['=', ':']) {
        let before = &rest[..offset];
        // A JSON key sits between quotes, a query key after `?` or `&`: the name
        // itself is the run of name characters immediately before the separator.
        let key_end = before.trim_end_matches('"').len();
        let head = &before[..key_end];
        let key_start = head
            .char_indices()
            .rfind(|(_, character)| {
                !(character.is_ascii_alphanumeric() || "_-.".contains(*character))
            })
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(0);
        let key = &head[key_start..];
        output.push_str(&rest[..key_start]);
        if key.is_empty() || !is_secret_field(key) {
            output.push_str(&rest[key_start..offset + 1]);
            rest = &rest[offset + 1..];
            continue;
        }
        output.push_str(&rest[key_start..offset + 1]);
        // A credential is a scalar. When a credential-named key (for example
        // `multiAgentIdentitySession`, ending in `session`) holds a JSON object
        // or array, the old span consumed only the opening bracket: nothing
        // secret was removed and the embedded JSON became invalid. Keep the
        // container intact and keep scanning inside it, exactly like
        // `redact_json_with` recurses into a credential-named container; every
        // credential scalar nested inside is still redacted by its own key.
        if json_container_starts_at(rest, offset + 1) {
            rest = &rest[offset + 1..];
            continue;
        }
        let (value_start, value_end) = pair_value_span(rest, offset + 1);
        output.push_str(&rest[offset + 1..value_start]);
        output.push_str(&marker_for(context, "auth", &rest[value_start..value_end]));
        rest = &rest[value_end..];
    }
    output.push_str(rest);
    output
}

/// Whether the value after a separator is a JSON object/array (`{"`, `{}`,
/// `["`, `[{`, `[[`, `[]`), not a bare `{token}` / `[token]` credential.
fn json_container_starts_at(text: &str, from: usize) -> bool {
    let value = text[from..].trim_start_matches([' ', '\t', '\n', '\r']);
    let mut characters = value.chars();
    let opener = characters.next();
    let next = characters
        .as_str()
        .trim_start_matches([' ', '\t', '\n', '\r'])
        .chars()
        .next();
    matches!(
        (opener, next),
        (Some('{'), Some('"' | '}')) | (Some('['), Some('"' | '{' | '[' | ']'))
    )
}

/// One `key=value` pair's value: up to the next pair, string quote or line end.
fn pair_value_span(text: &str, from: usize) -> (usize, usize) {
    let tail = &text[from..];
    let at = from + (tail.len() - tail.trim_start_matches([' ', '\t']).len());
    if text[at..].starts_with('"') {
        let inner = at + 1;
        return (
            inner,
            text[inner..]
                .find('"')
                .map(|offset| inner + offset)
                .unwrap_or(text.len()),
        );
    }
    (
        at,
        text[at..]
            .find(['&', ';', '\n', '\r', ' ', ',', '"', '\'', '}'])
            .map(|offset| at + offset)
            .unwrap_or(text.len()),
    )
}

/// Personal data the existing rules already treat as sensitive: mail addresses,
/// mainland id numbers and mobile numbers.
fn scrub_pii(text: &str, context: Option<&RedactionContext>) -> String {
    let mut output = String::with_capacity(text.len());
    for token in text.split_inclusive(|character: char| {
        character.is_whitespace() || ",;\"'()[]<>{}".contains(character)
    }) {
        let body = token.trim_end_matches(|character: char| {
            character.is_whitespace() || ",;\"'()[]<>{}".contains(character)
        });
        if let Some(kind) = pii_kind(body) {
            let padding = &token[body.len()..];
            output.push_str(&marker_for(context, kind, body));
            output.push_str(padding);
        } else {
            output.push_str(token);
        }
    }
    output
}

/// Which personal-data class a bare value is: mail address, mainland id number or
/// mobile number. Also used by the field fingerprinter so a value never leaves
/// that stage unmasked (§6.2).
pub fn pii_kind(body: &str) -> Option<&'static str> {
    let digits: String = body
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    if body.contains('@')
        && body
            .split('@')
            .nth(1)
            .is_some_and(|tail| tail.contains('.'))
        && !body.contains(' ')
    {
        Some("email")
    } else if matches!(digits.len(), 15 | 18) && digits.len() == body.len() {
        Some("national_id")
    } else if digits.len() == 11 && digits.starts_with('1') && digits.len() == body.len() {
        Some("phone")
    } else {
        None
    }
}

fn scrub_header_values(text: &str, context: Option<&RedactionContext>) -> String {
    let mut output = String::with_capacity(text.len());
    let lowered = text.to_ascii_lowercase();
    let mut cursor = 0usize;
    while cursor < text.len() {
        let next = [
            "cookie:",
            "set-cookie:",
            "authorization:",
            "proxy-authorization:",
        ]
        .iter()
        .filter_map(|needle| lowered[cursor..].find(needle).map(|offset| cursor + offset))
        .min();
        let Some(start) = next else { break };
        // A longer needle can start at the same offset ("set-cookie:" vs "cookie:"),
        // and both carry secrets.
        let colon = start + text[start..].find(':').unwrap_or(0);
        output.push_str(&text[cursor..colon + 1]);
        let value_start = colon
            + 1
            + match text[colon + 1..].chars().next() {
                Some(' ') | Some('\t') => 1,
                _ => 0,
            };
        // A header dump keeps its whole line as one secret value; a JSON fragment
        // only its quoted string, so the rest of the body stays readable.
        let quoted = text[value_start..].starts_with('"');
        let inner = value_start + usize::from(quoted);
        if quoted {
            output.push('"');
        }
        let value_end = if quoted {
            text[inner..]
                .find('"')
                .map(|offset| inner + offset)
                .unwrap_or(text.len())
        } else {
            text[inner..]
                .find(['\r', '\n'])
                .map(|offset| inner + offset)
                // A single pasted header commonly has no trailing newline. In
                // that case the credential runs to the end of the string; using
                // `inner` here would append the original value after the marker.
                .unwrap_or(text.len())
        };
        output.push_str(&marker_for(context, "auth", &text[inner..value_end]));
        cursor = value_end;
    }
    output.push_str(&text[cursor..]);
    output
}

fn scrub_tokens(
    text: &str,
    context: Option<&RedactionContext>,
    predicate: impl Fn(&str) -> bool,
    kind: &str,
) -> String {
    // `split_inclusive` keeps the original separators, so layout survives.
    text.split_inclusive(|character: char| character.is_ascii_whitespace())
        .map(|token| {
            let trailing = token.len().saturating_sub(
                token
                    .trim_end_matches(|character: char| character.is_ascii_whitespace())
                    .len(),
            );
            let body = &token[..token.len() - trailing];
            let core = body.trim_matches(|character: char| ",;\"'()[]<>{}".contains(character));
            if !core.is_empty() && predicate(core) {
                let leading = &body[..body.find(core).unwrap_or(0)];
                let trailing_punct = &body[leading.len() + core.len()..];
                format!(
                    "{leading}{}{trailing_punct}{}",
                    marker_for(context, kind, core),
                    &token[body.len()..]
                )
            } else {
                token.to_string()
            }
        })
        .collect()
}

/// `eyJ` is the base64url of a JSON object opening brace: every JWT starts there.
pub fn looks_like_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts[0].starts_with("eyJ")
        && parts[0].len() >= 8
        && parts.iter().all(|part| !part.is_empty())
}

pub fn looks_like_bearer(value: &str) -> bool {
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .is_some_and(|rest| rest.chars().count() >= 16 && !rest.chars().any(char::is_whitespace))
}

/// Long opaque provider keys. Pure hex digests are excluded because they are
/// evidence hashes, not credentials.
pub fn looks_like_api_key(value: &str) -> bool {
    let length = value.chars().count();
    if !(24..=200).contains(&length) {
        return false;
    }
    if value
        .chars()
        .any(|character| character.is_whitespace() || "/:\\%".contains(character))
    {
        return false;
    }
    if value.chars().all(|character| character.is_ascii_hexdigit()) {
        return false;
    }
    let allowed = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_.".contains(*character))
        .count();
    allowed == length
        && value
            .chars()
            .any(|character| character.is_ascii_uppercase())
}

// Phase 2/3 consumes this; the rules around it are covered by tests now.
#[allow(dead_code)]
pub fn contains_secret(text: &str) -> bool {
    text.split_ascii_whitespace().any(|token| {
        let cleaned = token.trim_matches(|character: char| ",;\"'()[]<>{}".contains(character));
        looks_like_jwt(cleaned) || looks_like_bearer(cleaned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // PROD-BUGFIX-2 regression: a credential-named key holding a JSON object
    // (the real `multiAgentIdentitySession` evidence) must not corrupt the
    // embedded JSON of an archived specialist request.
    #[test]
    fn credential_named_json_container_stays_valid_and_matches_structured_redaction() {
        let candidate = serde_json::json!({
            "evidence": {
                "multiAgentIdentitySession": {"identityMode": "IdentitySet", "summary": "metadata only"},
                "multiAgentMapper": {"summary": "mapper"},
            },
            "target": "https://authorized.example.test",
        });
        let request = serde_json::json!({"messages": [
            {"role": "system", "content": "review"},
            {"role": "user", "content": candidate.to_string()},
        ]});
        let archived = redact_json(&request);
        let content = archived["messages"][1]["content"].as_str().unwrap();
        let parsed: JsonValue = serde_json::from_str(content).expect("archived input must stay valid JSON");
        assert_eq!(parsed, redact_json(&candidate));
        assert_eq!(parsed["evidence"]["multiAgentIdentitySession"]["identityMode"], "IdentitySet");
    }

    // Guard still bites: real credential scalars, including ones nested inside
    // a credential-named container and bare non-JSON `{token}` / `[token]`
    // values, are still replaced, and the archived JSON stays valid.
    #[test]
    fn real_credentials_are_still_redacted_inside_and_outside_json_containers() {
        let raw = serde_json::json!({
            "apiKey": "sk-live-0123456789abcdef",
            "password": "hunter2-correct-horse",
            "session": "session-token-value-1",
            "multiAgentIdentitySession": {"accessToken": "nested-access-token", "identityMode": "IdentitySet"},
            "userSession": {"sessionId": "nested-session-id"},
            "cookies": [{"name": "Cookie", "value": "sid=abcdef"}],
        });
        let text = raw.to_string();
        let scrubbed = redact_text_with(&text, None);
        let parsed: JsonValue = serde_json::from_str(&scrubbed).expect("scrubbed JSON must stay valid");
        for secret in ["sk-live-0123456789abcdef", "hunter2-correct-horse", "session-token-value-1",
            "nested-access-token", "nested-session-id"] {
            assert!(!scrubbed.contains(secret), "{secret} leaked: {scrubbed}");
        }
        for pointer in ["/apiKey", "/password", "/session",
            "/multiAgentIdentitySession/accessToken", "/userSession/sessionId"] {
            assert!(parsed.pointer(pointer).and_then(JsonValue::as_str).unwrap().starts_with("<redacted:"), "{pointer}");
        }
        assert_eq!(parsed["multiAgentIdentitySession"]["identityMode"], "IdentitySet");
        let structured = redact_json(&raw);
        assert!(!structured.to_string().contains("nested-access-token"));
        assert!(!structured.to_string().contains("sid=abcdef"));
        // Bare (non-JSON) bracketed credentials are still consumed whole.
        for bare in ["token=[abc123secret]", "session={abc123secret}", "password: [hunter2]"] {
            let out = redact_text_with(bare, None);
            assert!(!out.contains("abc123secret") && !out.contains("hunter2"), "{bare} -> {out}");
        }
    }
}
