//! Structured knowledge-analysis transport, shared by explicit catalog actions
//! and Scanning's learning producer. Uses the application's single resolver;
//! model policy and the existing request/retry contract are preserved here.
use crate::model_configuration::{self, openai_chat_completion_model, ModelConnection};
use serde_json::Value as JsonValue;
use std::time::Duration;

pub(super) fn complete_on(
    connection: &rusqlite::Connection,
    prompt: &str,
) -> Result<JsonValue, String> {
    let settings = model_configuration::NormalizedModelSettings::new(
        &model_configuration::default_settings(connection),
    );
    let environment = model_configuration::model_connection(&settings)?;
    complete_with_connection(&environment, prompt)
}

fn llm_content(response: &JsonValue) -> Option<String> {
    let content = response
        .pointer("/choices/0/message/content")
        .or_else(|| response.pointer("/output/0/content/0/text"));
    match content {
        Some(JsonValue::String(value)) => Some(value.clone()),
        Some(JsonValue::Array(values)) => Some(
            values
                .iter()
                .filter_map(|value| value.get("text").and_then(JsonValue::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

fn parse_json_object(text: &str) -> Option<JsonValue> {
    let trimmed = text.trim().trim_matches('`').trim();
    serde_json::from_str(trimmed).ok().or_else(|| {
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        serde_json::from_str(&trimmed[start..=end]).ok()
    })
}

fn call_learning_llm_request(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    include_response_format: bool,
) -> Result<JsonValue, String> {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            {"role":"system","content":"你是 AppSec 技能工程师。只输出严格 JSON，不要 Markdown，不要复述目标凭据、Cookie、Token、项目名或一次性 URL。将一次扫描中可复用的方法提炼成可审核的候选，明确证据、置信度、冗余步骤和 Skill 补丁。低置信度、一次性现象、未经复现的猜测不得升级为规则。"},
            {"role":"user","content":prompt}
        ],
        "temperature": 0.1,
        "max_tokens": 5000,
        "stream": false
    });
    if include_response_format {
        body["response_format"] = serde_json::json!({"type":"json_object"});
    }
    let request_body = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .map_err(|error| format!("学习提炼模型请求失败：{error}"))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|error| format!("学习提炼模型响应读取失败：{error}"))?;
    if !status.is_success() {
        return Err(format!(
            "学习提炼模型 HTTP {}：{}",
            status.as_u16(),
            text.chars().take(600).collect::<String>()
        ));
    }
    let response: JsonValue = serde_json::from_str(&text)
        .map_err(|error| format!("学习提炼模型响应不是 JSON：{error}"))?;
    let content = llm_content(&response).ok_or("学习提炼模型未返回 content")?;
    parse_json_object(&content).ok_or_else(|| "学习提炼模型 content 不是合法 JSON".into())
}

pub(crate) fn complete_with_connection(
    environment: &ModelConnection,
    prompt: &str,
) -> Result<JsonValue, String> {
    let base = if environment.api_base.trim().is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        environment.api_base.trim_end_matches('/').to_string()
    };
    let endpoint = format!("{base}/chat/completions");
    let model = openai_chat_completion_model(&environment.llm);
    if model.is_empty() {
        return Err("当前 Agent 模型名为空，无法生成学习候选".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| error.to_string())?;
    match call_learning_llm_request(
        &client,
        &endpoint,
        &environment.api_key,
        &model,
        prompt,
        true,
    ) {
        Ok(value) => Ok(value),
        Err(first_error)
            if first_error.contains("HTTP 400")
                || first_error.contains("HTTP 401")
                || first_error.contains("HTTP 404")
                || first_error.contains("HTTP 422")
                || first_error.contains("content 不是合法 JSON")
                || first_error.contains("未返回 content") =>
        {
            call_learning_llm_request(
                &client,
                &endpoint,
                &environment.api_key,
                &model,
                prompt,
                false,
            )
            .map_err(|retry_error| {
                format!("{first_error}；去掉 response_format 后重试仍失败：{retry_error}")
            })
        }
        Err(error) => Err(error),
    }
}
