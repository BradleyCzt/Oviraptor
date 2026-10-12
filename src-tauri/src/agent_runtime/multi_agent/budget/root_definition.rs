//! Explicit budget semantics for a newly inserted Native Web Root only.
//! A declaration is budget authority, never target/write capability or evidence.
use super::DIMENSIONS;
use crate::agent_runtime::{multi_agent::lease, store};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod guard;
pub(crate) use guard::NewRootWriter;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NewRootBudgetDeclaration {
    pub schema_version: u8,
    pub declaration_id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_url: String,
    pub plan_hash: String,
    pub execution_slot_capacity: u8,
    pub limits: [Option<i64>; 10],
}

impl NewRootBudgetDeclaration {
    pub(crate) fn validate(
        &self,
        scan: &str,
        attempt: i64,
        target: &str,
        plan_hash: &str,
        plan: &Value,
    ) -> Result<(), String> {
        if self.schema_version != 2
            || uuid::Uuid::parse_str(&self.declaration_id).is_err()
            || self.scan_id != scan
            || self.attempt_number != attempt
            || self.target_url != target
            || self.plan_hash != plan_hash
            || self.execution_slot_capacity != 3
            || plan["owner"] != "oviraptor"
            || plan["backend"] != "native"
            || plan["schemaVersion"] != 2
            || plan["executionSurface"] != "web_only"
            || plan["surface"] == "source"
            || plan["targetUrl"] != target
            || plan["attemptNumber"] != attempt
        {
            return Err("budget_declaration_scope_invalid".into());
        }
        // Keep the existing Native hash algorithm, including its excluded identity.
        let mut frozen = plan.clone();
        let fields = frozen
            .as_object_mut()
            .ok_or("budget_declaration_plan_invalid")?;
        fields.remove("attemptNumber");
        fields.remove("targetUrl");
        if store::stable_hash(&frozen.to_string()) != plan_hash {
            return Err("budget_declaration_plan_hash_invalid".into());
        }
        let tokens = plan["budgets"]["hardTotalTokens"]
            .as_i64()
            .filter(|v| *v >= 0)
            .ok_or("budget_declaration_tokens_invalid")?;
        let requests = plan["budgets"]["hardModelRequests"]
            .as_i64()
            .filter(|v| *v >= 0)
            .ok_or("budget_declaration_requests_invalid")?;
        let finite = |n| (n > 0).then_some(n);
        let target_limit = requests.max(1).saturating_mul(4).min(400);
        let batch = self.limits[8]
            .filter(|n| (0..=128).contains(n))
            .ok_or("budget_declaration_batch_limit_invalid")?;
        let inherited = [
            finite(tokens),
            finite(tokens),
            finite(tokens),
            finite(requests),
            Some(target_limit),
            Some(0),
            Some(0),
            Some(0),
            Some(batch),
            Some(super::limits::frozen_wall_time(plan)?),
        ];
        if self.limits != inherited {
            return Err("budget_declaration_native_ceiling_conflict".into());
        }
        Ok(())
    }
}

/// Caller must have proved absence before inserting this exact Root in this
/// same transaction. Existing roots, including untouched historical rows, have
/// no path through this function and are never upgraded on read.
pub(crate) fn freeze_new(
    fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    declaration: &NewRootBudgetDeclaration,
) -> Result<(), String> {
    freeze_new_for_mode(
        fresh,
        declaration,
        crate::agent_runtime::web_mode::WebMode::Single,
    )
}

pub(crate) fn freeze_new_for_mode(
    fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    declaration: &NewRootBudgetDeclaration,
    mode: crate::agent_runtime::web_mode::WebMode,
) -> Result<(), String> {
    let tx = fresh.transaction();
    let root = fresh.id();
    lease::require_active_attempt(tx, &declaration.scan_id, declaration.attempt_number)?;
    let (scan,attempt,target,hash,text,pristine): (String,i64,String,String,String,bool) = tx.query_row(
        "SELECT scan_id,attempt_number,target_url,plan_hash,plan_json,
        status='prepared' AND parent_run_id IS NULL AND root_run_id=CASE WHEN ?2='multi' THEN id ELSE '' END AND assignment_id=''
        AND orchestration_policy=?2 AND started_at='' AND finished_at='' AND cancel_requested_at=''
        AND used_tokens=0 AND used_cached_tokens=0 AND used_requests=0
        AND reserved_tokens=0 AND reserved_requests=0
        FROM agent_runs WHERE id=?1 AND backend='native' AND role='coordinator'",
        params![root,mode.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .map_err(|_|"budget_declaration_root_missing")?;
    let plan = serde_json::from_str(&text).map_err(|_| "budget_declaration_plan_invalid")?;
    declaration.validate(&scan, attempt, &target, &hash, &plan)?;
    let history: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_runs child JOIN agent_runs root ON root.id=?1
            WHERE child.scan_id=root.scan_id AND child.attempt_number=root.attempt_number
                AND child.target_url=root.target_url AND child.id<>root.id)
        OR EXISTS(SELECT 1 FROM sentinel_checkpoints p JOIN agent_runs r
            ON p.scan_id=r.scan_id AND p.url=r.target_url WHERE r.id=?1
            AND p.stage='native_agent_state'
            AND (NOT json_valid(p.raw_json) OR json_extract(p.raw_json,'$.attemptNumber') IS NULL
                OR json_extract(p.raw_json,'$.attemptNumber')=r.attempt_number))
        OR EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_budget_definitions WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_financial_root_budget_definitions WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_financial_continuations WHERE new_root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !pristine || history {
        return Err("budget_declaration_requires_new_root".into());
    }
    let payload = serde_json::to_string(declaration).map_err(|e| e.to_string())?;
    let plan_text_hash = store::stable_hash(&text);

    let changed = tx
        .execute(
            "INSERT INTO agent_root_budget_definitions(root_run_id,declaration_id,
             definition_json,native_plan_text_hash) VALUES(?1,?2,?3,?4)",
            params![root, declaration.declaration_id, payload, plan_text_hash],
        )
        .map_err(|e| e.to_string())?;
    let exact: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_definitions WHERE root_run_id=?1
             AND declaration_id=?2 AND definition_json=?3 AND native_plan_text_hash=?4)",
            params![root, declaration.declaration_id, payload, plan_text_hash],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 || !exact {
        return Err("budget_declaration_persistence_conflict".into());
    }
    Ok(())
}

/// A missing row keeps the legacy contract. A present but corrupt row denies;
/// no historical plan, limit, row id, slot entry or fee is rewritten.
pub(crate) fn read(
    db: &Connection,
    root: &str,
) -> Result<Option<NewRootBudgetDeclaration>, String> {
    let saved: Option<(String, String, String)> = db
        .query_row(
            "SELECT declaration_id,definition_json,native_plan_text_hash
         FROM agent_root_budget_definitions WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, payload, text_hash)) = saved else {
        return super::continuation::migration::read(db, root);
    };
    let migrated: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_financial_root_budget_definitions WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_financial_continuations WHERE new_root_run_id=?1)",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    if migrated {
        return Err("budget_declaration_duplicate_contract".into());
    }
    let declaration: NewRootBudgetDeclaration =
        serde_json::from_str(&payload).map_err(|_| "budget_declaration_corrupt")?;
    let (scan, attempt, target, hash, text): (String, i64, String, String, String) = db
        .query_row(
            "SELECT scan_id,attempt_number,target_url,plan_hash,plan_json FROM agent_runs
         WHERE id=?1 AND backend='native' AND role='coordinator'
         AND assignment_id='' AND parent_run_id IS NULL",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| "budget_declaration_root_missing")?;
    if declaration.declaration_id != id || store::stable_hash(&text) != text_hash {
        return Err("budget_declaration_binding_conflict".into());
    }
    let plan = serde_json::from_str(&text).map_err(|_| "budget_declaration_plan_invalid")?;
    declaration.validate(&scan, attempt, &target, &hash, &plan)?;
    if declaration.limits.len() != DIMENSIONS.len() {
        return Err("budget_declaration_dimensions_invalid".into());
    }
    Ok(Some(declaration))
}
