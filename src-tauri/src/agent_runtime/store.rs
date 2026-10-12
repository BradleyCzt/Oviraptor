//! Event, run, message and artifact persistence (§11, §12).
//!
//! Model rounds and tool executions may run concurrently, but they only ever
//! append here. The terminal state is written by `reducer` alone.
use super::contract::{
    AgentBackendKind, AgentEventKind, AgentLane, AgentMessageKind, AgentRole, AgentRunStatus,
    MultiAgentPolicy, TerminalState,
};
use super::secrets::redact_json;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::BTreeSet;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const RUNTIME_EVENT_LIMIT: i64 = 20_000;
mod financial_continuation;
pub(crate) use financial_continuation::insert_financial_continuation_root_in;

/// Only this store can construct proof of an insert in the current transaction.
/// Sidecar writers cannot take an arbitrary existing Root id and upgrade it.
pub(crate) struct NewlyInsertedNativeRoot<'tx, 'connection> {
    transaction: &'tx Transaction<'connection>,
    id: &'tx str,
}
impl<'tx, 'connection> NewlyInsertedNativeRoot<'tx, 'connection> {
    pub(crate) fn transaction(&self) -> &'tx Transaction<'connection> {
        self.transaction
    }
    pub(crate) fn id(&self) -> &str {
        self.id
    }
}

/// §3.3: a persisted JSON column that no longer parses is an integrity error. The
/// table, column and record id travel with the message — a caller cannot tell a
/// corrupt capability lease from a corrupt task slice any other way.
pub fn decode_json<T: serde::de::DeserializeOwned>(
    column: &str,
    table: &str,
    id: &str,
    text: &str,
) -> Result<T, String> {
    serde_json::from_str(text)
        .map_err(|error| format!("表 {table} 的 {column} 无法解析（记录 {id}）：{error}"))
}

pub fn stable_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageDelta {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub model_requests: i64,
}

impl UsageDelta {
    pub fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "inputTokens": self.input_tokens,
            "cachedInputTokens": self.cached_input_tokens,
            "outputTokens": self.output_tokens,
            "totalTokens": self.total_tokens,
            "modelRequests": self.model_requests,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRunRow {
    pub id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_url: String,
    pub backend: AgentBackendKind,
    pub role: AgentRole,
    pub parent_run_id: Option<String>,
    pub status: AgentRunStatus,
    pub plan_hash: String,
    pub evidence_hash: String,
    pub soft_token_budget: i64,
    pub hard_token_budget: i64,
    // Read by the Phase 3 status surface; the reducer writes them in Phase 0.
    #[allow(dead_code)]
    pub used_tokens: i64,
    #[allow(dead_code)]
    pub used_cached_tokens: i64,
    pub soft_request_budget: i64,
    pub hard_request_budget: i64,
    #[allow(dead_code)]
    pub used_requests: i64,
    pub lease_expires_at: String,
    pub terminal_reason: String,
    pub terminal_code: String,
    pub terminal_state: Option<TerminalState>,
    // Where this run sits in the active orchestration. A single-agent run is its
    // own root, carries no assignment and reserves nothing.
    pub root_run_id: String,
    pub assignment_id: String,
    pub lane: Option<AgentLane>,
    pub orchestration_policy: MultiAgentPolicy,
    pub capability_lease: Vec<String>,
    pub reserved_tokens: i64,
    pub reserved_requests: i64,
    pub heartbeat_at: String,
    pub cancel_requested_at: String,
}

impl AgentRunRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        scan_id: impl Into<String>,
        attempt_number: i64,
        target_url: impl Into<String>,
        backend: AgentBackendKind,
        role: AgentRole,
        plan_hash: impl Into<String>,
        evidence_hash: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            scan_id: scan_id.into(),
            attempt_number,
            target_url: target_url.into(),
            backend,
            role,
            parent_run_id: None,
            status: AgentRunStatus::Prepared,
            plan_hash: plan_hash.into(),
            evidence_hash: evidence_hash.into(),
            soft_token_budget: 0,
            hard_token_budget: 0,
            used_tokens: 0,
            used_cached_tokens: 0,
            soft_request_budget: 0,
            hard_request_budget: 0,
            used_requests: 0,
            lease_expires_at: String::new(),
            terminal_reason: String::new(),
            terminal_code: String::new(),
            terminal_state: None,
            root_run_id: String::new(),
            assignment_id: String::new(),
            lane: None,
            orchestration_policy: MultiAgentPolicy::Single,
            capability_lease: Vec::new(),
            reserved_tokens: 0,
            reserved_requests: 0,
            heartbeat_at: String::new(),
            cancel_requested_at: String::new(),
        }
    }

    pub fn with_budget(
        mut self,
        soft: i64,
        hard: i64,
        soft_requests: i64,
        hard_requests: i64,
    ) -> Self {
        self.soft_token_budget = soft;
        self.hard_token_budget = hard;
        self.soft_request_budget = soft_requests;
        self.hard_request_budget = hard_requests;
        self
    }

    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// §Stage 3: the seal that forbids resume and forbids re-registration.
    pub fn is_legacy_sealed(&self) -> bool {
        self.status == AgentRunStatus::LegacyBackendRemoved
    }
}

pub fn create_run(connection: &Connection, row: &AgentRunRow) -> Result<(), String> {
    if row.backend != AgentBackendKind::Native || row.status == AgentRunStatus::LegacyBackendRemoved
    {
        return Err("agent_runs_backend_unsupported".into());
    }
    let lease = redact_json(&serde_json::json!(row.capability_lease)).to_string();
    let changed = connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,parent_run_id,status,plan_hash,evidence_hash,soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget,lease_expires_at,root_run_id,assignment_id,lane,orchestration_policy,capability_lease_json,reserved_tokens,reserved_requests,heartbeat_at,cancel_requested_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24) ON CONFLICT(id) DO UPDATE SET status=excluded.status,plan_hash=excluded.plan_hash,evidence_hash=excluded.evidence_hash,updated_at=datetime('now','localtime') WHERE agent_runs.backend='native' AND agent_runs.status<>'legacy_backend_removed' AND agent_runs.scan_id=excluded.scan_id AND agent_runs.attempt_number=excluded.attempt_number AND agent_runs.target_url=excluded.target_url AND agent_runs.role=excluded.role",
        params![
            row.id,
            row.scan_id,
            row.attempt_number,
            row.target_url,
            row.backend.as_str(),
            row.role.as_str(),
            row.parent_run_id,
            row.status.as_str(),
            row.plan_hash,
            row.evidence_hash,
            row.soft_token_budget,
            row.hard_token_budget,
            row.soft_request_budget,
            row.hard_request_budget,
            row.lease_expires_at,
            row.root_run_id,
            row.assignment_id,
            row.lane.map(AgentLane::as_str).unwrap_or(""),
            row.orchestration_policy.as_str(),
            lease,
            row.reserved_tokens,
            row.reserved_requests,
            row.heartbeat_at,
            row.cancel_requested_at,
        ],
    )
    .map_err(|error| format!("无法创建 agent run：{error}"))?;
    if changed != 1 {
        return Err("agent_runs_identity_or_backend_conflict".into());
    }
    Ok(())
}

/// Phase 2 §2.2: the frozen execution plan belongs to the attempt that ran it, not
/// to the URL. `agent_runs` already carries the attempt dimension, so the plan is
/// recorded there; the per-URL checkpoint row stays a compatibility projection and
/// nothing is ever deleted.
pub fn record_attempt_plan(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
    backend: AgentBackendKind,
    plan_hash: &str,
    plan: &JsonValue,
) -> Result<(), String> {
    record_attempt_plan_with_budget(
        connection,
        scan_id,
        attempt_number,
        target_url,
        backend,
        plan_hash,
        plan,
        None,
    )
}

/// Explicit opt-in is accepted only during a genuinely new Root insert.
/// Ordinary plan publication and every historical Native run keep old semantics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_attempt_plan_with_budget(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
    backend: AgentBackendKind,
    plan_hash: &str,
    plan: &JsonValue,
    declaration: Option<&super::multi_agent::budget::root_definition::NewRootBudgetDeclaration>,
) -> Result<(), String> {
    record_attempt_plan_with_declarations(connection,scan_id,attempt_number,target_url,backend,plan_hash,plan,declaration,None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_attempt_plan_with_declarations(
    connection:&Connection, scan_id:&str, attempt_number:i64, target_url:&str,
    backend:AgentBackendKind, plan_hash:&str, plan:&JsonValue,
    declaration:Option<&super::multi_agent::budget::root_definition::NewRootBudgetDeclaration>,
    mode_declaration:Option<&super::web_mode::root::NewRootModeDeclaration>,
) -> Result<(),String> {
    record_attempt_plan_with_declarations_and_tool_configuration(connection,scan_id,attempt_number,target_url,
        backend,plan_hash,plan,declaration,mode_declaration,None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_attempt_plan_with_declarations_and_tool_configuration(
    connection:&Connection, scan_id:&str, attempt_number:i64, target_url:&str,
    backend:AgentBackendKind, plan_hash:&str, plan:&JsonValue,
    declaration:Option<&super::multi_agent::budget::root_definition::NewRootBudgetDeclaration>,
    mode_declaration:Option<&super::web_mode::root::NewRootModeDeclaration>,
    configuration:Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> Result<(),String> {
    record_attempt_plan_with_declarations_on(connection,scan_id,attempt_number,target_url,
        backend,plan_hash,plan,declaration,mode_declaration,ModeFinancialCreation::New(configuration))
}

#[derive(Clone,Copy)]
enum ModeFinancialCreation<'a> {New(Option<&'a crate::tool_supply::ToolAdministratorConfiguration>), #[cfg(test)] HistoricalFixture}

#[allow(clippy::too_many_arguments)]
fn record_attempt_plan_with_declarations_on(
    connection:&Connection, scan_id:&str, attempt_number:i64, target_url:&str,
    backend:AgentBackendKind, plan_hash:&str, plan:&JsonValue,
    declaration:Option<&super::multi_agent::budget::root_definition::NewRootBudgetDeclaration>,
    mode_declaration:Option<&super::web_mode::root::NewRootModeDeclaration>,
    financial_creation:ModeFinancialCreation<'_>,
) -> Result<(),String> {
    if let Some(mode)=mode_declaration {mode.validate_scope(scan_id,attempt_number,target_url,plan_hash)?;}
    if backend != AgentBackendKind::Native {
        return Err("agent_runs_backend_unsupported".into());
    }
    if let Some(declaration) = declaration {
        declaration.validate(scan_id, attempt_number, target_url, plan_hash, plan)?;
    }
    let payload = plan.to_string();
    // IMMEDIATE serializes the read/compare/write across SQLite connections. A
    // failed insert or update rolls back the whole plan, including a new run row.
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| format!("无法锁定 attempt 计划：{error}"))?;
    let existing = {
        let mut statement = transaction
            .prepare("SELECT id,plan_json,plan_hash,backend,status FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role=?4")
            .map_err(|error| format!("无法读取 attempt 计划：{error}"))?;
        let rows = statement
            .query_map(
                params![
                    scan_id,
                    attempt_number,
                    target_url,
                    AgentRole::Coordinator.as_str()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(|error| format!("无法读取 attempt 计划：{error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法读取 attempt 计划：{error}"))?
    };
    if mode_declaration.is_some() && !existing.is_empty() {return Err("web_mode_requires_new_root".into());}
    if declaration.is_some() && !existing.is_empty() {
        return Err("budget_declaration_requires_new_root".into());
    }
    let mut mode_writer=mode_declaration.map(|mode|match financial_creation {
        ModeFinancialCreation::New(_)=>super::web_mode::writer::ModeRootWriter::install(&transaction,mode,declaration),
        #[cfg(test)] ModeFinancialCreation::HistoricalFixture=>super::web_mode::writer::ModeRootWriter::install_historical_for_test(&transaction,mode,declaration),
    }).transpose()?;
    // authorizer is one setter: choose exactly one writer, never nest them.
    let new_root_writer = declaration.filter(|_|mode_declaration.is_none())
        .map(|declaration| {
            super::multi_agent::budget::root_definition::NewRootWriter::install(
                &transaction,
                declaration,
            )
        })
        .transpose()?;
    for (_, stored_json, stored_hash, stored_backend, stored_status) in &existing {
        if stored_backend != "native" || stored_status == "legacy_backend_removed" {
            return Err("agent_runs_backend_unsupported".into());
        }
        let initialized = stored_json != "{}" && !stored_json.is_empty();
        let same_plan =
            serde_json::from_str::<JsonValue>(stored_json).is_ok_and(|stored| stored == *plan);
        if stored_backend != backend.as_str()
            || (!stored_hash.is_empty() && stored_hash != plan_hash)
            || (initialized && (!same_plan || stored_hash != plan_hash))
        {
            return Err("agent_attempt_plan_frozen_conflict".into());
        }
    }
    if existing.is_empty() {
        let id = format!(
            "plan-{scan_id}-{attempt_number}-{}",
            &stable_hash(target_url)[..12]
        );
        let born_ceilings=if mode_declaration.is_some() && matches!(financial_creation,ModeFinancialCreation::New(_)) {
            ["softUncachedTokens","hardTotalTokens","softModelRequests","hardModelRequests"]
                .map(|key|plan["budgets"][key].as_i64().filter(|v|*v>=0).ok_or("web_finance_original_ceiling_invalid"))
                .into_iter().collect::<Result<Vec<_>,_>>()?
        }else {vec![0;4]};
        // Do not use create_run's upsert here: an id collision must fail closed,
        // never rewrite another run. The plan and its row are inserted together.
        transaction
            .execute(
                "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,plan_hash,plan_json,orchestration_policy,root_run_id,soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![id, scan_id, attempt_number, target_url, backend.as_str(), AgentRole::Coordinator.as_str(), plan_hash, payload,
                    mode_declaration.map(|m|m.mode().as_str()).unwrap_or("single"),
                    if mode_declaration.is_some_and(|m|m.mode()==super::web_mode::WebMode::Multi) {id.as_str()}else {""},born_ceilings[0],born_ceilings[1],born_ceilings[2],born_ceilings[3]],
            )
            .map_err(|error| format!("无法写入 attempt 计划：{error}"))?;
        let fresh=NewlyInsertedNativeRoot {transaction:&transaction,id:&id};
        if let Some(writer)=mode_writer.as_mut() {writer.root_inserted(&fresh)?;}
        if let Some(declaration)=declaration {
            if let Some(mode)=mode_declaration {
                super::multi_agent::budget::root_definition::freeze_new_for_mode(&fresh,declaration,mode.mode())?;
            }else {super::multi_agent::budget::root_definition::freeze_new(&fresh,declaration)?;}
        }
        if let Some(mode)=mode_declaration {super::web_mode::root::freeze_new(&fresh,mode)?;}
        if let Some(writer)=mode_writer.as_mut() {writer.publish_finance(&fresh)?;}
        if let ModeFinancialCreation::New(Some(configuration)) = financial_creation {
            mode_writer.as_mut().ok_or("tool_supply_private_mode_required")?
                .publish_tool_supply(&fresh, configuration)?;
        }
        #[cfg(test)]
        if let Some(writer)=mode_writer.as_mut().filter(|_| matches!(financial_creation,ModeFinancialCreation::New(None))) {
            crate::tool_supply::frozen_plan::tests::capture_original_fresh_root_for_test(&fresh,writer)?;
        }
    } else {
        for (id, stored_json, _, _, _) in &existing {
            if stored_json == "{}" || stored_json.is_empty() {
                transaction
                    .execute(
                        "UPDATE agent_runs SET plan_json=?1,plan_hash=?2,updated_at=datetime('now','localtime') WHERE id=?3 AND backend='native' AND status<>'legacy_backend_removed'",
                        params![payload, plan_hash, id],
                    )
                    .map_err(|error| format!("无法写入 attempt 计划：{error}"))?;
                if transaction.changes() != 1 {
                    return Err("agent_runs_backend_unsupported".into());
                }
            }
        }
    }
    // The per-URL checkpoint is a compatibility projection, never the
    // authority. Keep it on the newest recorded attempt and commit it with the
    // authoritative row: a failed projection must not leave a half-frozen plan.
    let newest_attempt: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(attempt_number),0) FROM agent_runs WHERE scan_id=?1 AND target_url=?2 AND role=?3 AND backend='native' AND status<>'legacy_backend_removed' AND plan_json<>'{}'",
            params![scan_id, target_url, AgentRole::Coordinator.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法确定最新 attempt 计划：{error}"))?;
    if newest_attempt == attempt_number {
        transaction
            .execute(
                "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,'agent_execution_plan',?3) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                params![scan_id, target_url, payload],
            )
            .map_err(|error| format!("无法投影 attempt 计划：{error}"))?;
    }
    mode_writer.map(super::web_mode::writer::ModeRootWriter::finish).transpose()?;
    new_root_writer
        .map(super::multi_agent::budget::root_definition::NewRootWriter::finish)
        .transpose()?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 attempt 计划：{error}"))
}

/// The plan one attempt ran on. `None` means that attempt never recorded one: either
/// it predates this column, or it never started.
pub fn attempt_plan(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
) -> Option<JsonValue> {
    connection
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role=?4 AND plan_json <> '{}' ORDER BY created_at DESC LIMIT 1",
            params![scan_id, attempt_number, target_url, AgentRole::Coordinator.as_str()],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// Bring an existing run row up to the frozen plan it is about to execute: the four
/// ceilings and the plan hash must describe the same plan the loop reads, even when
/// the row was first created by plan recording rather than by `open_run`.
pub fn apply_run_plan(
    connection: &Connection,
    run_id: &str,
    plan_hash: &str,
    soft_token_budget: i64,
    hard_token_budget: i64,
    soft_request_budget: i64,
    hard_request_budget: i64,
) -> Result<(), String> {
    let changed = connection
        .execute(
            "UPDATE agent_runs SET plan_hash=?2,soft_token_budget=?3,hard_token_budget=?4,soft_request_budget=?5,hard_request_budget=?6,updated_at=datetime('now','localtime') WHERE id=?1 AND backend='native' AND status<>'legacy_backend_removed'",
            params![run_id, plan_hash, soft_token_budget, hard_token_budget, soft_request_budget, hard_request_budget],
        )
        .map_err(|error| format!("无法写入运行预算：{error}"))?;
    if changed == 1 {
        Ok(())
    } else {
        Err("agent_run_not_active_native".into())
    }
}

pub fn set_run_status(
    connection: &Connection,
    run_id: &str,
    status: AgentRunStatus,
) -> Result<(), String> {
    if status == AgentRunStatus::LegacyBackendRemoved {
        return Err("agent_runs_backend_unsupported".into());
    }
    let changed = connection
        .execute(
            "UPDATE agent_runs SET status=?1,updated_at=datetime('now','localtime'),started_at=CASE WHEN ?1='running' AND started_at='' THEN datetime('now','localtime') ELSE started_at END WHERE id=?2 AND backend='native' AND status<>'legacy_backend_removed'",
            params![status.as_str(), run_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        let stored: Option<(String, String)> = connection
            .query_row(
                "SELECT backend,status FROM agent_runs WHERE id=?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if stored.as_ref().is_some_and(|(backend, state)| {
            backend != "native" || state == "legacy_backend_removed"
        }) {
            return Err(format!(
                "legacy_backend_removed/非 Native 历史 run 不能再转为 {status:?}，只能全新 Native 重试"
            ));
        }
        return Err("agent_run_not_found".into());
    }
    Ok(())
}

pub fn load_run(connection: &Connection, run_id: &str) -> Result<Option<AgentRunRow>, String> {
    // Orchestration columns are read as text and decoded afterwards, so a corrupt
    // lane word or lease never becomes a plausible-looking default (§3.2, §3.3).
    let read = connection
        .query_row(
            "SELECT id,scan_id,attempt_number,target_url,backend,role,parent_run_id,status,plan_hash,evidence_hash,soft_token_budget,hard_token_budget,used_tokens,used_cached_tokens,soft_request_budget,hard_request_budget,used_requests,lease_expires_at,terminal_reason,terminal_code,terminal_state,root_run_id,assignment_id,lane,orchestration_policy,capability_lease_json,reserved_tokens,reserved_requests,heartbeat_at,cancel_requested_at FROM agent_runs WHERE id=?1",
            [run_id],
            |row| {
                Ok((
                    AgentRunRow {
                        id: row.get(0)?,
                        scan_id: row.get(1)?,
                        attempt_number: row.get(2)?,
                        target_url: row.get(3)?,
                        // Replaced by a strict decode below before this row can escape.
                        backend: AgentBackendKind::LegacyRemoved,
                        role: AgentRole::Coordinator,
                        parent_run_id: row.get::<_, Option<String>>(6)?,
                        status: AgentRunStatus::parse(&row.get::<_, String>(7)?),
                        plan_hash: row.get(8)?,
                        evidence_hash: row.get(9)?,
                        soft_token_budget: row.get(10)?,
                        hard_token_budget: row.get(11)?,
                        used_tokens: row.get(12)?,
                        used_cached_tokens: row.get(13)?,
                        soft_request_budget: row.get(14)?,
                        hard_request_budget: row.get(15)?,
                        used_requests: row.get(16)?,
                        lease_expires_at: row.get(17)?,
                        terminal_reason: row.get(18)?,
                        terminal_code: row.get(19)?,
                        // Judged outside the reader: an unrecognisable non-empty
                        // terminal state is corruption, not "no terminal state yet".
                        terminal_state: None,
                        root_run_id: row.get(21)?,
                        assignment_id: row.get(22)?,
                        lane: None,
                        orchestration_policy: MultiAgentPolicy::Single,
                        capability_lease: Vec::new(),
                        reserved_tokens: row.get(26)?,
                        reserved_requests: row.get(27)?,
                        heartbeat_at: row.get(28)?,
                        cancel_requested_at: row.get(29)?,
                    },
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(23)?,
                    row.get::<_, String>(24)?,
                    row.get::<_, String>(25)?,
                    row.get::<_, String>(20)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((mut run, role, lane, policy, lease, terminal, backend)) = read else {
        return Ok(None);
    };
    run.backend = AgentBackendKind::parse(&backend)
        .ok_or_else(|| "agent_runs_backend_unsupported".to_string())?;
    run.role = AgentRole::try_parse(&role).ok_or_else(|| {
        format!(
            "表 agent_runs 的 role 不是已知角色（记录 {}）：{role}",
            run.id
        )
    })?;
    run.terminal_state = match terminal.trim() {
        "" => None,
        word => Some(TerminalState::parse(word).ok_or_else(|| {
            format!(
                "表 agent_runs 的 terminal_state 不是已知终态词汇（记录 {}）：{terminal}",
                run.id
            )
        })?),
    };
    run.lane = match lane.trim() {
        "" => None,
        word => Some(AgentLane::try_parse(word).ok_or_else(|| {
            format!(
                "表 agent_runs 的 lane 不是已知泳道（记录 {}）：{lane}",
                run.id
            )
        })?),
    };
    run.orchestration_policy = MultiAgentPolicy::parse(&policy);
    run.capability_lease =
        decode_json::<Vec<String>>("capability_lease_json", "agent_runs", &run.id, &lease)?;
    Ok(Some(run))
}

pub fn find_run(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
    role: AgentRole,
) -> Result<Option<AgentRunRow>, String> {
    connection
        .query_row(
            "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role=?4 ORDER BY created_at DESC LIMIT 1",
            params![scan_id, attempt_number, target_url, role.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
        .and_then(|id| match id {
            Some(id) => load_run(connection, &id),
            None => Ok(None),
        })
}

/// Additive only: budgets and usage are never overwritten by a backend.
/// Write the cumulative spend of a run. The close-out can be replayed after a
/// resume, so the row converges on the reported total instead of adding the same
/// spend a second time.
#[cfg(test)]
pub fn settle_usage(
    connection: &Connection,
    run_id: &str,
    usage: &UsageDelta,
) -> Result<(), String> {
    let changed = connection.execute(
        "UPDATE agent_runs SET used_tokens=?1,used_cached_tokens=?2,used_requests=?3,updated_at=datetime('now','localtime') WHERE id=?4 AND backend='native' AND status<>'legacy_backend_removed'",
        params![usage.total_tokens, usage.cached_input_tokens, usage.model_requests, run_id],
    )
    .map_err(|error| error.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("agent_run_not_active_native".into())
    }
}

#[allow(dead_code)]
pub fn renew_lease(connection: &Connection, run_id: &str, minutes: i64) -> Result<(), String> {
    let changed = connection.execute(
        "UPDATE agent_runs SET lease_expires_at=datetime('now','localtime',?2),updated_at=datetime('now','localtime') WHERE id=?1 AND backend='native' AND status<>'legacy_backend_removed'",
        params![run_id, format!("+{minutes} minutes")],
    )
    .map_err(|error| error.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("agent_run_not_active_native".into())
    }
}

/// The full row is kept because the Phase 2 broker reads the refs and the
/// debug view shows the timestamps; replay itself only needs the payload.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct AgentEventRow {
    pub id: i64,
    pub run_id: String,
    pub sequence: i64,
    pub event_type: AgentEventKind,
    pub payload: JsonValue,
    pub artifact_refs: Vec<String>,
    pub created_at: String,
}

/// Append one event and return its sequence. `UNIQUE(run_id,sequence)` rejects
/// a concurrent writer, so the caller retries a few times instead of racing.
pub fn append_event(
    connection: &Connection,
    run_id: &str,
    kind: AgentEventKind,
    payload: &JsonValue,
    artifact_refs: &[String],
) -> Result<i64, String> {
    let payload = redact_json(payload);
    let refs = redact_json(&serde_json::json!(artifact_refs));
    let refs_text = refs.to_string();
    for attempt in 0..5i64 {
        let next: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM agent_events WHERE run_id=?1",
                [run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let result = connection.execute(
            "INSERT INTO agent_events(run_id,sequence,event_type,payload_json,artifact_refs_json) VALUES(?1,?2,?3,?4,?5)",
            params![run_id, next, kind.as_str(), payload.to_string(), refs_text],
        );
        match result {
            Ok(_) => return Ok(next),
            Err(error) if error.to_string().contains("UNIQUE") && attempt < 4 => {
                continue;
            }
            Err(error) => return Err(format!("无法写入 agent event：{error}")),
        }
    }
    Err("agent event 序号竞争失败".to_string())
}

pub fn read_events_after(
    connection: &Connection,
    run_id: &str,
    sequence: i64,
) -> Result<Vec<AgentEventRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id,run_id,sequence,event_type,payload_json,artifact_refs_json,created_at FROM agent_events WHERE run_id=?1 AND sequence> ?2 ORDER BY sequence ASC LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id, sequence, RUNTIME_EVENT_LIMIT], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    for row in rows {
        let (id, run_id, sequence, kind, payload_text, refs_text, created_at) =
            row.map_err(|error| error.to_string())?;
        events.push(AgentEventRow {
            id,
            run_id,
            sequence,
            event_type: AgentEventKind::parse(&kind),
            // Recovery replays these rows to rebuild the run, so a payload that no
            // longer parses is an integrity error rather than an empty object.
            payload: decode_json(
                "payload_json",
                "agent_events",
                &id.to_string(),
                &payload_text,
            )?,
            artifact_refs: decode_json::<Vec<String>>(
                "artifact_refs_json",
                "agent_events",
                &id.to_string(),
                &refs_text,
            )?,
            created_at,
        });
    }
    Ok(events)
}

pub fn latest_sequence(connection: &Connection, run_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_events WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn parse_json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or(JsonValue::Null)
}

pub fn write_snapshot(
    connection: &Connection,
    run_id: &str,
    last_sequence: i64,
    snapshot: &JsonValue,
) -> Result<(), String> {
    let snapshot = redact_json(snapshot);
    connection.execute(
        "INSERT INTO agent_snapshots(run_id,last_sequence,schema_version,snapshot_json) VALUES(?1,?2,?3,?4) ON CONFLICT(run_id) DO UPDATE SET last_sequence=excluded.last_sequence,schema_version=excluded.schema_version,snapshot_json=excluded.snapshot_json,updated_at=datetime('now','localtime')",
        params![run_id, last_sequence, super::contract::RUNTIME_SCHEMA_VERSION, snapshot.to_string()],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[derive(Clone, Debug)]
pub struct SnapshotRow {
    #[allow(dead_code)]
    pub run_id: String,
    pub last_sequence: i64,
    pub schema_version: i64,
    pub snapshot: JsonValue,
}

pub fn read_snapshot(connection: &Connection, run_id: &str) -> Result<Option<SnapshotRow>, String> {
    connection
        .query_row(
            "SELECT run_id,last_sequence,schema_version,snapshot_json FROM agent_snapshots WHERE run_id=?1",
            [run_id],
            |row| {
                Ok(SnapshotRow {
                    run_id: row.get(0)?,
                    last_sequence: row.get(1)?,
                    schema_version: row.get(2)?,
                    snapshot: parse_json(&row.get::<_, String>(3)?),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

/// Messages are deduplicated by `(run_id, dedup_key)` so a replay after a crash
/// cannot process the same contract twice (§9). Consumed in Phase 5.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn append_message(
    connection: &Connection,
    run_id: &str,
    from_agent: &str,
    to_agent: &str,
    kind: AgentMessageKind,
    correlation_id: &str,
    dedup_key: &str,
    payload: &JsonValue,
    artifact_refs: &[String],
) -> Result<bool, String> {
    let payload = redact_json(payload);
    let inserted = connection
        .execute(
            "INSERT INTO agent_messages(id,run_id,from_agent,to_agent,kind,correlation_id,dedup_key,payload_json,artifact_refs_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(run_id,dedup_key) DO NOTHING",
            params![
                uuid::Uuid::new_v4().to_string(),
                run_id,
                from_agent,
                to_agent,
                kind.as_str(),
                correlation_id,
                dedup_key,
                payload.to_string(),
                serde_json::json!(artifact_refs).to_string()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(inserted > 0)
}

#[allow(dead_code)]
pub fn read_undelivered_messages(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<(String, JsonValue, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT kind,payload_json,dedup_key FROM agent_messages WHERE run_id=?1 AND delivered_at='' ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows
        .flatten()
        .map(|(kind, payload, key)| (kind, parse_json(&payload), key))
        .collect())
}

#[allow(dead_code)]
pub fn mark_message_delivered(connection: &Connection, dedup_key: &str) -> Result<(), String> {
    connection
        .execute(
            "UPDATE agent_messages SET delivered_at=datetime('now','localtime') WHERE dedup_key=?1 AND delivered_at=''",
            [dedup_key],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Phase 2 §4.2 step 2: the row exists in `running` state before any side effect,
/// so a crash mid-call leaves something recoverable instead of a silent gap.
/// `invocation_id` is the id the runtime hands out and the coverage ledger cites,
/// which keeps the audit row and the ledger on the same key (§9.2).
pub fn begin_tool_invocation(
    connection: &Connection,
    run_id: &str,
    invocation_id: &str,
    tool_name: &str,
    tool_version: i64,
    contract_key: &str,
    identity_handle: &str,
    input: &JsonValue,
    policy_decision: &str,
) -> Result<i64, String> {
    let summary = redact_json(input);
    let input_hash = stable_hash(&summary.to_string());
    connection
        .execute(
            "INSERT INTO tool_invocations(run_id,invocation_id,tool_name,tool_version,contract_key,identity_handle,input_hash,input_summary_json,policy_decision,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'running')",
            params![run_id, invocation_id, tool_name, tool_version, contract_key, identity_handle, input_hash, summary.to_string(), policy_decision],
        )
        .map_err(|error| error.to_string())?;
    Ok(connection.last_insert_rowid())
}

/// Phase 2 §4.2 step 5: the deterministic end state of one call, with the response
/// artifact it produced. Only after this may the result travel back to the model.
pub fn finish_tool_invocation(
    connection: &Connection,
    invocation_id: i64,
    status: &str,
    progress_signature: &str,
    response_artifact_id: &str,
    error_class: &str,
) -> Result<(), String> {
    let changed = connection
        .execute(
            "UPDATE tool_invocations SET status=?1,progress_signature=?2,response_artifact_id=?3,error_class=?4,\
             policy_decision=CASE WHEN ?1='refused' THEN 'deny' ELSE policy_decision END,\
             finished_at=datetime('now','localtime') WHERE id=?5 AND status='running'",
            params![status, progress_signature, response_artifact_id, error_class, invocation_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("tool_invocation_not_running_or_missing".into())
    }
}

/// §11 step 3: a call still marked `running` after a crash can never be trusted.
pub fn mark_interrupted_tool_invocations(
    connection: &Connection,
    run_id: &str,
) -> Result<usize, String> {
    connection
        .execute(
            "UPDATE tool_invocations SET status='interrupted',error_class='process_interrupted',finished_at=datetime('now','localtime') WHERE run_id=?1 AND status='running'",
            [run_id],
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
pub fn contract_without_result(
    connection: &Connection,
    run_id: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT contract_key FROM tool_invocations WHERE run_id=?1 AND contract_key<>'' AND status NOT IN ('confirmed','rejected','exhausted','insufficient_evidence')",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

#[cfg(test)]
pub fn mark_run_terminal(
    connection: &Connection,
    run_id: &str,
    state: TerminalState,
    code: &str,
    reason: &str,
) -> Result<bool, String> {
    let changed = connection
        .execute(
            "UPDATE agent_runs SET status='terminal',terminal_reason=?2,terminal_code=?3,terminal_state=?4,finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status<>'terminal'",
            params![run_id, reason, code, state.as_str()],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Ok(false);
    }
    let _ = append_event(
        connection,
        run_id,
        AgentEventKind::TerminalReduced,
        &serde_json::json!({"terminal": state.as_str(), "code": code, "reason": reason}),
        &[],
    );
    Ok(true)
}

/// Content-addressed artifact store (§12). Only the id is ever persisted in a
/// message or event; the bytes stay on disk. The Phase 2 broker writes the first
/// rows; the layout and permissions are fixed here and covered by tests.
#[allow(dead_code)]
pub fn artifact_id(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn artifact_path(root: &Path, id: &str) -> PathBuf {
    root.join("artifacts")
        .join(id.chars().take(2).collect::<String>())
        .join(id)
}

#[allow(dead_code)]
pub fn store_artifact(root: &Path, bytes: &[u8]) -> Result<String, String> {
    let id = artifact_id(bytes);
    let path = artifact_path(root, &id);
    if path.is_file() {
        return Ok(id);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    crate::db::write_private_file(&path, bytes)?;
    Ok(id)
}

#[allow(dead_code)]
pub fn read_artifact(root: &Path, id: &str) -> Option<Vec<u8>> {
    fs::read(artifact_path(root, id)).ok()
}

// Genuine pre-finance creator semantics, only for historical fixture contracts.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_historical_mode_plan_for_test(
    db:&Connection,scan:&str,attempt:i64,target:&str,
    backend:AgentBackendKind,hash:&str,plan:&JsonValue,
    mode:&super::web_mode::root::NewRootModeDeclaration,
)->Result<(),String> {
    record_attempt_plan_with_declarations_on(db,scan,attempt,target,backend,hash,plan,None,Some(mode),ModeFinancialCreation::HistoricalFixture)
}

mod source_root;
pub(crate) use source_root::{SourceRootInsertion,NewlyInsertedSourceRoot};
