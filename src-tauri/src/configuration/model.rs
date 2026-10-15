//! Configuration transport contracts. Compatibility exports live in `crate::models`.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProfile {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub settings: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProfileInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_default: bool,
    pub settings: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileTestInput {
    pub llm: String,
    pub deployment: String,
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileTestResult {
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub model: String,
    pub deployment: String,
}
