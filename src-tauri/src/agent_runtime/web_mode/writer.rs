//! One authorizer for Mode-only or Mode+Budget, with exact pre-existing row proof.
//! Budget-only continues to use NewRootWriter; never nest authorizer setters.
use super::root::{self, NewRootModeDeclaration};
use crate::agent_runtime::{
    multi_agent::{
        attempts::audit_rows::Rows,
        budget::root_definition::{self as budget, NewRootBudgetDeclaration},
    },
    store,
};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params,
    types::Value,
    Transaction,
};

pub(crate) struct ModeRootWriter<'tx, 'db> {
    tx: &'tx Transaction<'db>,
    mode: NewRootModeDeclaration,
    budget: Option<NewRootBudgetDeclaration>,
    finance: Option<super::finance::FinanceCreation>,
    roots: Rows,
    budgets: Rows,
    modes: Rows,
    birth_seals: Rows,
    drafts: Rows,
    receipts: Rows,
    actions: Rows,
    projections: Rows,
    events: Rows,
    event_floor: i64,
    writes: u64,
    new_root: Option<(String, Rows)>,
    active: bool,
    tool_plans: Rows,
    tool_plan: Option<(
        crate::tool_supply::ToolAdministratorConfiguration,
        Option<serde_json::Value>,
    )>,
    finance_published: bool,
}
impl<'tx, 'db> ModeRootWriter<'tx, 'db> {
    pub(crate) fn install(
        tx: &'tx Transaction<'db>,
        mode: &NewRootModeDeclaration,
        budget: Option<&NewRootBudgetDeclaration>,
    ) -> Result<Self, String> {
        Self::install_on(tx, mode, budget, true)
    }
    #[cfg(test)]
    pub(crate) fn install_historical_for_test(
        tx: &'tx Transaction<'db>,
        mode: &NewRootModeDeclaration,
        budget: Option<&NewRootBudgetDeclaration>,
    ) -> Result<Self, String> {
        Self::install_on(tx, mode, budget, false)
    }
    fn install_on(
        tx: &'tx Transaction<'db>,
        mode: &NewRootModeDeclaration,
        budget: Option<&NewRootBudgetDeclaration>,
        publish_finance: bool,
    ) -> Result<Self, String> {
        crate::collaboration_events::web_creation_schema::verify(tx)?;
        let event_floor = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let result = Self {
            tx,
            mode: mode.clone(),
            budget: budget.cloned(),
            finance: publish_finance
                .then(|| super::finance::FinanceCreation::capture(tx, mode))
                .transpose()?,
            roots: Rows::read(tx, "SELECT rowid,* FROM agent_runs ORDER BY rowid", [])?,
            budgets: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_root_budget_definitions ORDER BY rowid",
                [],
            )?,
            modes: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_root_mode_definitions ORDER BY rowid",
                [],
            )?,
            birth_seals: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_root_mode_birth_seals ORDER BY rowid",
                [],
            )?,
            drafts: Rows::read(
                tx,
                "SELECT rowid,* FROM native_web_mode_drafts ORDER BY rowid",
                [],
            )?,
            receipts: Rows::read(
                tx,
                "SELECT rowid,* FROM native_web_mode_receipts ORDER BY rowid",
                [],
            )?,
            actions: Rows::read(
                tx,
                "SELECT rowid,* FROM native_web_mode_actions ORDER BY rowid",
                [],
            )?,
            projections: Self::other_projections(tx, mode)?,
            events: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
                [event_floor],
            )?,
            event_floor,
            writes: tx.total_changes(),
            new_root: None,
            active: true,
            tool_plans: Rows::read(
                tx,
                "SELECT rowid,* FROM main.agent_frozen_tool_supply_plans ORDER BY rowid",
                [],
            )?,
            tool_plan: None,
            finance_published: false,
        };
        tx.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        Ok(result)
    }
    fn other_projections(
        tx: &Transaction<'_>,
        mode: &NewRootModeDeclaration,
    ) -> Result<Rows, String> {
        Rows::read(tx,"SELECT rowid,* FROM sentinel_checkpoints WHERE NOT(scan_id=?1 AND url=?2 AND stage='agent_execution_plan') ORDER BY rowid",
            params![mode.fact.scan_id,mode.target_url])
    }
    /// Freeze the exact freshly inserted Root before either sidecar is written.
    pub(crate) fn root_inserted(
        &mut self,
        fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    ) -> Result<(), String> {
        if self.new_root.is_some() {
            return Err("web_mode_new_root_proof_reused".into());
        }
        let root = fresh.id();
        let rows = Rows::read(
            self.tx,
            "SELECT rowid,* FROM agent_runs WHERE id=?1",
            [root],
        )?;
        if rows.values.len() != 1 {
            return Err("web_mode_new_root_missing".into());
        }
        self.new_root = Some((root.into(), rows));
        Ok(())
    }
    pub(crate) fn publish_finance(
        &mut self,
        fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    ) -> Result<(), String> {
        if let Some(finance) = self.finance.as_mut() {
            finance.publish(fresh, &self.mode)?;
        }
        self.finance_published = true;
        Ok(())
    }
    pub(crate) fn publish_tool_supply(
        &mut self,
        fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
        configuration: &crate::tool_supply::ToolAdministratorConfiguration,
    ) -> Result<Option<crate::tool_supply::frozen_plan::VerifiedFrozenToolPlan>, String> {
        if !std::ptr::eq(fresh.transaction(), self.tx)
            || !self.finance_published
            || self.tool_plan.is_some()
            || self.new_root.as_ref().map(|(id, _)| id.as_str()) != Some(fresh.id())
        {
            return Err("web_mode_tool_supply_fresh_binding_changed".into());
        }
        let before = self.tx.total_changes();
        self.tx
            .authorizer(Some(authorize_tool_supply))
            .map_err(|e| e.to_string())?;
        let result = (|| {
            crate::tool_supply::frozen_plan::capture_for_new_root(fresh, configuration)?
                .map(crate::tool_supply::frozen_plan::issue_new)
                .transpose()
        })();
        self.tx
            .authorizer(Some(authorize))
            .map_err(|e| e.to_string())?;
        let verified = result?;
        if self.tx.total_changes().checked_sub(before) != Some(u64::from(verified.is_some())) {
            return Err("web_mode_tool_supply_collateral_write".into());
        }
        self.tool_plan = Some((
            configuration.clone(),
            verified.as_ref().map(|proof| proof.fact().clone()),
        ));
        Ok(verified)
    }
    pub(crate) fn finish(mut self) -> Result<(), String> {
        if let Some(finance) = &self.finance {
            finance.verify(self.tx)?;
        }
        crate::collaboration_events::web_creation_schema::verify(self.tx)?;
        let (root, original) = self.new_root.as_ref().ok_or("web_mode_new_root_missing")?;
        self.mode.verify_fresh_birth(self.tx, root)?;
        let expected_tool_writes = if let Some((configuration, expected)) = &self.tool_plan {
            let original =
                crate::tool_supply::frozen_plan::read_historical_on(self.tx, configuration, root)?;
            if original.as_ref().map(|proof| proof.fact()) != expected.as_ref() {
                return Err("web_mode_tool_supply_original_changed".into());
            }
            u64::from(expected.is_some())
        } else {
            0
        };
        let native_plan: String = self.tx.query_row(
            "SELECT plan_json FROM main.agent_runs WHERE id=?1", [root], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        let native_plan: serde_json::Value = serde_json::from_str(&native_plan)
            .map_err(|_| "tool_supply_original_native_plan_invalid")?;
        let requests_tools = crate::tool_supply::selection::FrozenToolSelection::from_plan_value(
            native_plan.get("toolSupply"),
        )?.is_some();
        if requests_tools != (expected_tool_writes == 1) {
            return Err("tool_supply_original_frozen_plan_required".into());
        }
        let new_tool_rows = Rows::read(
            self.tx,
            "SELECT rowid,* FROM main.agent_frozen_tool_supply_plans WHERE root_run_id=?1",
            [root],
        )?;
        if new_tool_rows.values.len() != expected_tool_writes as usize
            || Rows::read(self.tx,"SELECT rowid,* FROM main.agent_frozen_tool_supply_plans WHERE root_run_id<>?1 ORDER BY rowid",[root])? != self.tool_plans {
            return Err("web_mode_tool_supply_collateral_write".into());
        }
        let frozen = root::read(self.tx, root)?.ok_or("web_mode_root_receipt_missing")?;
        let frozen_budget = budget::read(self.tx, root)?;
        let text: String = self
            .tx
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let projection:String=self.tx.query_row("SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='agent_execution_plan'",
            params![self.mode.fact.scan_id,self.mode.target_url],|r|r.get(0)).map_err(|e|e.to_string())?;
        let new=Rows::read(self.tx,"SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence",[self.event_floor])?;
        let event_ok = new.values.len() == 1
            && new.values[0][..5]
                == [
                    Value::Text(self.mode.fact.scan_id.clone()),
                    Value::Integer(self.mode.fact.attempt_number),
                    Value::Text("agent_run".into()),
                    Value::Text(root.clone()),
                    Value::Text("agent_run".into()),
                ]
            && matches!(&new.values[0][5],Value::Text(json) if serde_json::from_str::<serde_json::Value>(json).ok()==Some(serde_json::json!({"role":"coordinator","status":"prepared","terminalState":""})));
        if !frozen.same_binding(&self.mode) || frozen_budget!=self.budget || projection!=text || !event_ok
            || &Rows::read(self.tx,"SELECT rowid,* FROM agent_runs WHERE id=?1",[root])?!=original
            || Rows::read(self.tx,"SELECT rowid,* FROM agent_runs WHERE id<>?1 ORDER BY rowid",[root])?!=self.roots
            || Rows::read(self.tx,"SELECT rowid,* FROM agent_root_budget_definitions WHERE root_run_id<>?1 ORDER BY rowid",[root])?!=self.budgets
            || Rows::read(self.tx,"SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id<>?1 ORDER BY rowid",[root])?!=self.modes
            || Rows::read(self.tx,"SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id<>?1 ORDER BY rowid",[root])?!=self.birth_seals
            || Rows::read(self.tx,"SELECT rowid,* FROM native_web_mode_drafts ORDER BY rowid",[]) ?!=self.drafts
            || Rows::read(self.tx,"SELECT rowid,* FROM native_web_mode_receipts ORDER BY rowid",[]) ?!=self.receipts
            || Rows::read(self.tx,"SELECT rowid,* FROM native_web_mode_actions ORDER BY rowid",[])?!=self.actions
            || Rows::read(self.tx,"SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",[self.event_floor])?!=self.events
            || Self::other_projections(self.tx,&self.mode)?!=self.projections
            || self.tx.total_changes().checked_sub(self.writes)!=Some(5+expected_tool_writes+u64::from(self.budget.is_some())+self.finance.as_ref().map_or(0,|proof|proof.expected_writes())) {
            return Err("web_mode_root_creation_collateral_write".into());
        }
        self.tx
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())?;
        self.active = false;
        Ok(())
    }
}
impl Drop for ModeRootWriter<'_, '_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .tx
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    match c.action {
        AuthAction::Insert { table_name }
            if c.database_name == Some("main")
                && c.accessor.is_none()
                && matches!(
                    table_name,
                    "agent_runs"
                        | "agent_root_mode_definitions"
                        | "agent_root_mode_birth_seals"
                        | "agent_root_budget_definitions"
                        | "sentinel_checkpoints"
                        | "agent_coordinator_leases"
                        | "agent_root_budget_attempts"
                        | "agent_budget_limits"
                        | "agent_budget_clock_origins"
                ) =>
        {
            Authorization::Allow
        }
        AuthAction::Update {
            table_name: "sentinel_checkpoints",
            column_name,
        } if c.database_name == Some("main")
            && c.accessor.is_none()
            && matches!(column_name, "raw_json" | "updated_at") =>
        {
            Authorization::Allow
        }
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main")
            && c.accessor == Some("agent_collaboration_run_insert") =>
        {
            Authorization::Allow
        }
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }
}

// The only write during this stage is the fresh plan INSERT. Even an otherwise
// legitimate Root birth write is denied here, including all trigger side effects.
fn authorize_tool_supply(c: AuthContext<'_>) -> Authorization {
    match c.action {
        AuthAction::Insert {
            table_name: "agent_frozen_tool_supply_plans",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }
}
