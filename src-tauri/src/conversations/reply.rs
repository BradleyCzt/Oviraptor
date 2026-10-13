use super::{model::*, store::*};
use crate::{
    agent_runtime::model::{
        CancelToken, GatewayProfile, ModelRequest, OneShotFailure, OpenAiCompatibleGateway,
    },
    db,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    net::IpAddr,
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;

const REPLY_TIMEOUT: Duration = Duration::from_secs(90);
const CONTEXT_BYTES: usize = 65_536;
const SYSTEM: &str="你是 Oviraptor 工作空间中的独立文字对话助手。仅依据本次提供的对话文字回答；你不能浏览、扫描、读取资产或执行工具，不能声称已经执行任何操作。调查或资产链接只是导航，未向你提供其内容。收到的历史可能仅包含最近的完整消息；缺少上下文时直接说明。";

pub(super) fn resolve_profile(db: &Connection) -> Result<GatewayProfile, String> {
    validate_profile(crate::commands::conversation_model_profile(db)?)
}
fn validate_profile(mut profile: GatewayProfile) -> Result<GatewayProfile, String> {
    let url = reqwest::Url::parse(&profile.endpoint)
        .map_err(|_| "conversation_model_endpoint_invalid")?;
    let loopback = url
        .host_str()
        .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback());
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (profile.local && url.scheme() == "http" && loopback))
    {
        return Err("conversation_model_endpoint_policy".into());
    }
    profile.max_output_tokens = Some(profile.max_output_tokens.unwrap_or(2048).min(2048));
    if profile.max_output_tokens == Some(0)
        || profile.max_context_tokens != 0
            && profile.max_context_tokens <= profile.max_output_tokens.unwrap_or(0) + 512
    {
        return Err("conversation_model_context_invalid".into());
    }
    Ok(profile)
}
fn availability_error(code: &str) -> ReplyAvailability {
    let message = match code {
        "conversation_project_archived" => {
            "工作空间已归档，可阅读历史；恢复工作空间后才能继续对话。"
        }
        "conversation_archived" => "对话已归档，可阅读历史；恢复对话后才能继续。",
        "conversation_model_endpoint_policy" => {
            "独立对话要求 HTTPS 模型地址；本地模型也可使用 127.0.0.1 或 [::1] 的 HTTP 地址。"
        }
        "conversation_model_endpoint_invalid" | "conversation_model_context_invalid" => {
            "模型地址或上下文设置无效，请检查模型配置。"
        }
        _ => "尚未配置可用模型。可以保存消息、查看和搜索历史；不会生成模拟回复。",
    };
    ReplyAvailability {
        available: false,
        code: code.into(),
        message: message.into(),
        model: None,
    }
}
pub(super) fn availability(db: &Connection, item: &Conversation) -> ReplyAvailability {
    if let Err(code) = project(db, item.project_id, true) {
        return availability_error(&code);
    }
    if item.archived {
        return availability_error("conversation_archived");
    }
    match resolve_profile(db) {
        Ok(profile) => ReplyAvailability {
            available: true,
            code: String::new(),
            message: String::new(),
            model: Some(profile.model),
        },
        Err(code) => availability_error(&code),
    }
}
pub(super) fn receipt(db: &Connection, input: &SendInput) -> Result<Option<SendReceipt>, String> {
    let item = conversation(db, input.project_id, &input.conversation_id)?;
    let message:Option<(Message,bool)>=db.query_row(&format!("SELECT {MESSAGE_COLUMNS},request_reply FROM conversation_messages WHERE conversation_id=?1 AND operation_id=?2"),params![item.id,input.operation_id],|r|Ok((message_row(r)?,r.get(7)?))).optional().map_err(err)?;
    let Some((message, request_reply)) = message else {
        return Ok(None);
    };
    if message.content != input.content.trim() || request_reply != input.request_reply {
        return Err("conversation_operation_conflict".into());
    }
    let reply=db.query_row(&format!("SELECT {REPLY_COLUMNS} FROM conversation_replies WHERE conversation_id=?1 AND user_message_id=?2"),params![item.id,message.id],reply_row).optional().map_err(err)?;
    Ok(Some(SendReceipt {
        conversation: item,
        user_message: message,
        reply,
    }))
}
fn request(db: &Connection, id: &str, profile: &GatewayProfile) -> Result<ModelRequest, String> {
    let mut history: Vec<Value> = {
        let mut stmt=db.prepare("SELECT role,content FROM conversation_messages WHERE conversation_id=?1 ORDER BY sequence DESC LIMIT 64").map_err(err)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(json!({"role":r.get::<_,String>(0)?,"content":r.get::<_,String>(1)?}))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        rows
    };
    history.reverse();
    let budget = CONTEXT_BYTES
        .min(profile.request_budget_bytes())
        .saturating_sub(2048);
    let mut messages = vec![json!({"role":"system","content":SYSTEM})];
    messages.extend(history);
    while serde_json::to_vec(&messages)
        .map_err(|_| "conversation_context_serialization")?
        .len()
        > budget
        && messages.len() > 2
    {
        messages.remove(1);
    }
    if serde_json::to_vec(&messages)
        .map_err(|_| "conversation_context_serialization")?
        .len()
        > budget
    {
        return Err("conversation_context_too_large".into());
    }
    let mut request = ModelRequest::new(messages, vec![]);
    request.request_timeout = Some(REPLY_TIMEOUT);
    request.reject_redirects = true;
    request.max_response_bytes = Some(1_048_576);
    Ok(request)
}

/// The lock is acquired before publishing queued and survives through the
/// single provider call and final commit. Replays observe receipts, not jobs.
pub(super) fn send(path: &Path, input: SendInput) -> Result<SendReceipt, String> {
    send_with(path, input, resolve_profile, |profile, request, cancel| {
        OpenAiCompatibleGateway::new(profile, &[]).complete_once_observed(request, cancel)
    })
}
fn send_with<F, G>(
    path: &Path,
    input: SendInput,
    profile_resolver: F,
    complete: G,
) -> Result<SendReceipt, String>
where
    F: FnOnce(&Connection) -> Result<GatewayProfile, String>,
    G: FnOnce(
        GatewayProfile,
        &ModelRequest,
        &CancelToken,
    ) -> Result<crate::agent_runtime::model::gateway::ModelResponse, OneShotFailure>,
{
    operation(&input.operation_id)?;
    let content = input.content.trim();
    if content.is_empty() || content.len() > 16_384 || content.contains('\0') {
        return Err("conversation_invalid_message".into());
    }
    let mut db = db::open(path)?;
    // Persist the operation identity before an external side effect. This is
    // connection-local and does not change scan or general read policies.
    db.pragma_update(None, "synchronous", "FULL").map_err(err)?;
    {
        let tx = db.transaction().map_err(err)?;
        if let Some(saved) = receipt(&tx, &input)? {
            return Ok(saved);
        }
    }
    let initial = conversation(&db, input.project_id, &input.conversation_id)?;
    let _owner = lock(path, &initial)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    if let Some(saved) = receipt(&tx, &input)? {
        return Ok(saved);
    }
    recover_in(&tx, &initial.id)?;
    let item = writable(
        &tx,
        input.project_id,
        &initial.id,
        input.expected_revision,
        false,
    )?;
    let sequence:i64=tx.query_row("SELECT COALESCE(MAX(sequence),0)+1 FROM conversation_messages WHERE conversation_id=?1",[&item.id],|r|r.get(0)).map_err(err)?;
    if sequence >= MAX_INTEGER {
        return Err("conversation_history_limit".into());
    }
    let user = Uuid::new_v4().to_string();
    let now = stamp();
    if tx.execute("INSERT INTO conversation_messages(id,conversation_id,sequence,role,content,operation_id,request_reply,created_at) VALUES(?1,?2,?3,'user',?4,?5,?6,?7)",params![user,item.id,sequence,content,input.operation_id,input.request_reply,now]).map_err(err)?!=1{return Err("conversation_write_not_persisted".into());}
    bump(&tx, &item.id, item.revision)?;
    let mut model_request = None;
    if input.request_reply {
        let id = Uuid::new_v4().to_string();
        let (profile, status, code, message) = match profile_resolver(&tx) {
            Ok(profile) => (Some(profile), "queued", "".to_string(), "".to_string()),
            Err(code) => {
                let unavailable = availability_error(&code);
                (None, "unavailable", unavailable.code, unavailable.message)
            }
        };
        let model = profile.as_ref().map(|p| p.model.as_str()).unwrap_or("");
        if tx.execute("INSERT INTO conversation_replies(id,conversation_id,user_message_id,base_revision,status,code,message,model,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",params![id,item.id,user,item.revision+1,status,code,message,model,now]).map_err(err)?!=1{return Err("conversation_write_not_persisted".into());}
        if let Some(profile) = profile {
            match request(&tx, &item.id, &profile) {
                Ok(request) => {
                    let context_hash = format!(
                        "{:x}",
                        Sha256::digest(
                            serde_json::to_vec(&request.messages)
                                .map_err(|_| "conversation_context_serialization")?
                        )
                    );
                    tx.execute(
                        "UPDATE conversation_replies SET context_hash=?2 WHERE id=?1",
                        params![id, context_hash],
                    )
                    .map_err(err)?;
                    model_request = Some((id, profile, request));
                }
                Err(code) => {
                    tx.execute("UPDATE conversation_replies SET status='failed',code=?2,message='消息超出当前模型的上下文限制，未发送模型请求。' WHERE id=?1",params![id,code]).map_err(err)?;
                }
            }
        }
    }
    let saved = receipt(&tx, &input)?.ok_or("conversation_write_not_persisted")?;
    if saved.user_message.id != user
        || saved.conversation.revision != item.revision + 1
        || input.request_reply && saved.reply.is_none()
    {
        return Err("conversation_write_not_persisted".into());
    }
    tx.commit()
        .map_err(|_| "conversation_send_commit_unconfirmed".to_string())?;
    let Some((reply_id, profile, request)) = model_request else {
        return Ok(saved);
    };
    // Publishing running occurs while the same OS lock is already held.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let changed=tx.execute("UPDATE conversation_replies SET status='running',updated_at=?2 WHERE id=?1 AND status='queued'",params![reply_id,stamp()]).map_err(err)?;
    if changed != 1 {
        let current = reply(&tx, &item.id, &reply_id)?.ok_or("conversation_reply_missing")?;
        if current.status == "cancel_requested" {
            tx.execute("UPDATE conversation_replies SET status='cancelled',code='user_cancelled',message='回复已取消，尚未发送模型请求。',updated_at=?2 WHERE id=?1",params![reply_id,stamp()]).map_err(err)?;
            tx.commit().map_err(err)?;
            return receipt(&db, &input)?.ok_or_else(|| "conversation_receipt_missing".into());
        }
        return Err("conversation_reply_state_changed".into());
    }
    if reply(&tx, &item.id, &reply_id)?.is_none_or(|value| value.status != "running") {
        return Err("conversation_write_not_persisted".into());
    }
    tx.commit()
        .map_err(|_| "conversation_reply_start_commit_unconfirmed".to_string())?;
    let started = Instant::now();
    let database = path.to_path_buf();
    let conversation_id = item.id.clone();
    let reply_token = reply_id.clone();
    let project_id = item.project_id;
    let expected = item.revision + 1;
    let cancel = CancelToken::from_checker(move || {
        started.elapsed() >= REPLY_TIMEOUT
            || !may_continue(
                &database,
                project_id,
                &conversation_id,
                &reply_token,
                expected,
            )
    });
    let result = complete(profile, &request, &cancel);
    finish(
        &mut db,
        &input,
        &reply_id,
        expected,
        started.elapsed() >= REPLY_TIMEOUT,
        result,
    )?;
    receipt(&db, &input)?.ok_or_else(|| "conversation_receipt_missing".into())
}
fn may_continue(path: &Path, project_id: i64, id: &str, reply_id: &str, revision: i64) -> bool {
    let Ok(db) = db::open(path) else {
        return false;
    };
    // A contended writer must not block the cancellation poll for ten seconds.
    if db.busy_timeout(Duration::from_millis(50)).is_err() {
        return false;
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM conversation_replies r JOIN conversations c ON c.id=r.conversation_id JOIN projects p ON p.id=c.project_id WHERE r.id=?1 AND c.id=?2 AND p.id=?3 AND c.revision=?4 AND r.base_revision=?4 AND r.status='running' AND c.archived=0 AND p.status='active')",params![reply_id,id,project_id,revision],|r|r.get::<_,bool>(0)).unwrap_or(false)
}
fn finish(
    db: &mut Connection,
    input: &SendInput,
    id: &str,
    expected: i64,
    timed_out: bool,
    result: Result<crate::agent_runtime::model::gateway::ModelResponse, OneShotFailure>,
) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let current = reply(&tx, &input.conversation_id, id)?.ok_or("conversation_reply_missing")?;
    if !matches!(current.status.as_str(), "running" | "cancel_requested") {
        return Err("conversation_reply_state_changed".into());
    }
    let item = conversation(&tx, input.project_id, &input.conversation_id)?;
    let fresh =
        item.revision == expected && !item.archived && project(&tx, input.project_id, true).is_ok();
    let (mut status, mut code, mut detail, unknown, usage, answer) = match result {
        Ok(response) => {
            let usage=json!({"usage":response.usage.as_json(),"usageReported":response.usage_reported,"cachedUsageReported":response.cached_usage_reported}).to_string();
            if !response.tool_calls.is_empty()
                || response.text.trim().is_empty()
                || response.text.len() > 65_536
            {
                (
                    "failed",
                    "reply_invalid_response".to_string(),
                    "模型未返回可用的纯文字回复；未执行任何工具。".to_string(),
                    false,
                    usage,
                    None,
                )
            } else {
                (
                    "completed",
                    String::new(),
                    String::new(),
                    false,
                    usage,
                    Some(response.text),
                )
            }
        }
        Err(failure) => {
            let (error, unknown) = match failure {
                OneShotFailure::BeforeTransport(error) => (error, false),
                OneShotFailure::TransportOutcomeUnknown(error) => (error, true),
            };
            let cancelled = matches!(error, crate::agent_runtime::model::ModelError::Cancelled);
            (
                if cancelled && !timed_out {
                    "cancelled"
                } else {
                    "failed"
                },
                if timed_out {
                    "model_timeout".into()
                } else {
                    error.code().into()
                },
                if timed_out {
                    "回复等待超时。不会自动重发；模型端结果及费用可能未知。".into()
                } else if cancelled {
                    "回复已停止；模型端结果及费用可能未知。".into()
                } else {
                    "模型请求失败。不会自动重试，请检查配置或连接；模型端结果及费用可能未知。"
                        .into()
                },
                unknown,
                "{}".into(),
                None,
            )
        }
    };
    if !fresh {
        status = "cancelled";
        code = "reply_stale_completion".into();
        detail = "对话或工作空间已变化，过期回复未写入聊天记录。".into();
    } else if current.status == "cancel_requested" {
        status = "cancelled";
        code = "user_cancelled".into();
        detail = "回复已停止；已收到的模型结果未加入聊天记录。".into();
    }
    let mut assistant = None;
    if status == "completed" {
        let content = answer.ok_or("conversation_reply_missing_content")?;
        let message_id = Uuid::new_v4().to_string();
        let sequence:i64=tx.query_row("SELECT COALESCE(MAX(sequence),0)+1 FROM conversation_messages WHERE conversation_id=?1",[&item.id],|r|r.get(0)).map_err(err)?;
        if tx.execute("INSERT INTO conversation_messages(id,conversation_id,sequence,role,content,reply_to_message_id,created_at) VALUES(?1,?2,?3,'assistant',?4,?5,?6)",params![message_id,item.id,sequence,content,current.user_message_id,stamp()]).map_err(err)?!=1{return Err("conversation_reply_write_not_persisted".into());}
        bump(&tx, &item.id, expected)
            .map_err(|_| "conversation_reply_write_not_persisted".to_string())?;
        let stored: String = tx
            .query_row(
                "SELECT content FROM conversation_messages WHERE id=?1",
                [&message_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if stored != content {
            return Err("conversation_reply_write_not_persisted".into());
        }
        assistant = Some(message_id);
    }
    if tx.execute("UPDATE conversation_replies SET status=?2,code=?3,message=?4,outcome_unknown=?5,usage_json=?6,assistant_message_id=?7,updated_at=?8 WHERE id=?1 AND status=?9",params![id,status,code,detail,unknown,usage,assistant,stamp(),current.status]).map_err(err)?!=1{return Err("conversation_reply_write_not_persisted".into());}
    let final_reply = reply(&tx, &item.id, id)?.ok_or("conversation_reply_missing")?;
    if final_reply.status != status
        || final_reply.code != code
        || final_reply.assistant_message_id != assistant
        || final_reply.outcome_unknown != unknown
    {
        return Err("conversation_reply_write_not_persisted".into());
    }
    tx.commit()
        .map_err(|_| "conversation_reply_commit_unconfirmed".to_string())
}
pub(super) fn cancel(path: &Path, input: CancelInput) -> Result<Reply, String> {
    let mut db = db::open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    conversation(&tx, input.project_id, &input.conversation_id)?;
    let current = reply(&tx, &input.conversation_id, &input.reply_id)?
        .ok_or("conversation_reply_unavailable")?;
    if matches!(current.status.as_str(), "queued" | "running") {
        if tx.execute("UPDATE conversation_replies SET status='cancel_requested',code='cancel_requested',message='正在停止回复，等待连接关闭。',updated_at=?2 WHERE id=?1 AND status=?3",params![input.reply_id,stamp(),current.status]).map_err(err)?!=1{return Err("conversation_cancel_not_persisted".into());}
        if reply(&tx, &input.conversation_id, &input.reply_id)?
            .is_none_or(|v| v.status != "cancel_requested")
        {
            return Err("conversation_cancel_not_persisted".into());
        }
    }
    let result = reply(&tx, &input.conversation_id, &input.reply_id)?
        .ok_or("conversation_reply_unavailable")?;
    tx.commit().map_err(err)?;
    Ok(result)
}
#[cfg(test)]
#[path = "reply_tests.rs"]
mod tests;
