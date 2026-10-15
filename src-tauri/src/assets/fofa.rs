//! Asset-source FOFA account validation and connection test.
use crate::asset_models::{FofaApiTestInput, FofaApiTestResult};
use serde_json::Value as JsonValue;
use std::{io::Read, time::Duration};

fn fofa_transport_error(error: &reqwest::Error, proxy_url: &str) -> String {
    if error.is_timeout() {
        return if proxy_url.is_empty() {
            "FOFA 连接超时，请检查当前网络后重试".into()
        } else {
            format!("FOFA 连接超时；当前配置的代理 {proxy_url} 没有正常转发请求")
        };
    }
    if !proxy_url.is_empty() {
        return format!("无法通过代理 {proxy_url} 连接 FOFA；请确认代理程序已启动且端口正确");
    }
    "无法连接 FOFA，请检查 DNS、网络或防火墙后重试".into()
}

pub(crate) fn request_fofa_account(key: &str, proxy_url: &str) -> Result<JsonValue, String> {
    if !key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("FOFA Key 格式无效".into());
    }
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));
    if !proxy_url.is_empty() {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|_| "代理地址格式无效，应类似 http://127.0.0.1:7890".to_string())?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|_| "无法初始化 FOFA 网络客户端".to_string())?;
    let mut response = client
        .get(format!("https://fofa.info/api/v1/info/my?key={key}"))
        .send()
        .map_err(|error| fofa_transport_error(&error, proxy_url))?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(|_| "读取 FOFA 响应失败".to_string())?;
    let payload: JsonValue = serde_json::from_slice(&body)
        .map_err(|_| format!("FOFA 返回了无法解析的响应（HTTP {}）", status.as_u16()))?;
    if !status.is_success()
        || payload
            .get("error")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
    {
        let message = payload
            .get("errmsg")
            .or_else(|| payload.get("message"))
            .and_then(JsonValue::as_str)
            .unwrap_or("Key 无效或 FOFA 拒绝了请求");
        return Err(format!("FOFA 鉴权失败：{message}"));
    }
    Ok(payload)
}

#[tauri::command]
pub async fn test_fofa_api(input: FofaApiTestInput) -> Result<FofaApiTestResult, String> {
    let key = input.key.trim().to_string();
    let proxy_url = input.proxy_url.trim().to_string();
    if key.is_empty() {
        return Err("请先填写 FOFA key".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let payload = request_fofa_account(&key, &proxy_url)?;
        let account = payload
            .get("email")
            .or_else(|| payload.get("username"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
        let plan = payload
            .get("vip_level")
            .or_else(|| payload.get("vipLevel"))
            .or_else(|| payload.get("level"))
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default();
        Ok(FofaApiTestResult {
            ok: true,
            status: "available".into(),
            message: if proxy_url.is_empty() {
                "FOFA API 可用，鉴权与直连网络均正常".into()
            } else {
                format!("FOFA API 可用，代理 {proxy_url} 工作正常")
            },
            account,
            plan,
        })
    })
    .await
    .map_err(|error| format!("FOFA 测试任务异常：{error}"))?
}
