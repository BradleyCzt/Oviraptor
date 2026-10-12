//! Original shared SDK lifetime. A local exit never refunds a remote request.
use super::*;
use crate::agent_runtime::{execution_owner, multi_agent::budget};

pub(crate) fn invocation_kind(request: &Value) -> &'static str {
    if request["humanDirectiveDispatch"]["kind"] == "human_ordered_readonly_assessment_dispatch"
        && request["humanDirectiveDispatch"]["schemaVersion"] == 3
    {
        "ordered-assessment-sdk"
    } else {
        "specialist-sdk"
    }
}

// Only this verifier can construct local exit proofs; guards stay held through closure.
pub(crate) struct IdleSpecialists {
    _guards: Vec<execution_owner::NativeInvocationOwner>,
    readonly: Vec<PendingCall>,
    mapper: Vec<(
        CoordinatorLease,
        ScheduledChild,
        super::mapper_loop::VerifiedMapperIdle,
    )>,
}
impl IdleSpecialists {
    pub(crate) fn close_readonly(&self, tx: &rusqlite::Transaction<'_>) -> Result<(), String> {
        for call in &self.readonly {
            super::readonly_closure::pause(tx, call)?;
        }
        for (actor, child, proof) in &self.mapper {
            super::readonly_closure::pause_mapper_loop(tx, actor, child, proof)?;
        }
        Ok(())
    }
}

pub(crate) fn require_idle_for_root(
    db: &Connection,
    scope: &CoordinatorLease,
) -> Result<IdleSpecialists, String> {
    if db.is_autocommit() {
        return Err("specialist_lifetime_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|path| !path.is_empty())
        .ok_or("specialist_lifetime_database_missing")?;
    let mut q = db.prepare("SELECT assignment_id,child_run_id,role,request_hash,request_json FROM agent_specialist_calls
        WHERE root_run_id=?1 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([&scope.root_run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let calls = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let (mapper_guards, mapper) = mapper_idle_for_root(db, scope)?;
    if calls.is_empty() {
        return Ok(IdleSpecialists {
            _guards: mapper_guards,
            readonly: vec![],
            mapper,
        });
    }
    let root = budget::root::RootOwner::load_original(db, &scope.root_run_id)?;
    root.require_original_coordinator(db, scope)?;
    let mut idle = mapper_guards;
    let mut readonly = vec![];
    for (assignment_id, run_id, role, request_hash, raw) in calls {
        let parsed = AgentRole::try_parse(&role)
            .filter(|r| r.as_str() == role)
            .ok_or("specialist_lifetime_role_invalid")?;
        let actual_role: String = db
            .query_row("SELECT role FROM agent_runs WHERE id=?1", [&run_id], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if actual_role != role
            || !matches!(
                parsed,
                AgentRole::SpaApiMapper
                    | AgentRole::IdentitySession
                    | AgentRole::DeepInvestigator
                    | AgentRole::RepoMapper
                    | AgentRole::SourceAnalyst
                    | AgentRole::EvidenceReviewer
                    | AgentRole::ExternalSurface
                    | AgentRole::ClientSide
                    | AgentRole::InputParser
            )
        {
            return Err("specialist_lifetime_role_invalid".into());
        }
        let call = PendingCall {
            lease: scope.clone(),
            child: ScheduledChild {
                assignment_id,
                run_id,
                role: parsed,
            },
            request_hash,
        };
        let owner = budget::receipts::original_owner(
            db,
            &call.child.run_id,
            scope,
            &call.child.assignment_id,
        )?;
        owner.verify(db)?;
        let row = load(db, &call)?.ok_or("specialist_lifetime_original_call_missing")?;
        if !matches!(row.state.as_str(), "executing" | "received" | "uncertain") {
            return Err("specialist_lifetime_original_call_state_invalid".into());
        }
        let request: Value = serde_json::from_str(&raw)
            .map_err(|_| "specialist_lifetime_original_request_invalid")?;
        let kind = invocation_kind(&request);
        let ordered = kind == "ordered-assessment-sdk";
        let guard = execution_owner::probe_native_invocation(
            std::path::Path::new(path),
            &scope.scan_id,
            scope.attempt_number,
            kind,
            &call.child.run_id,
        )
        .map_err(|e| {
            format!(
                "{}:{e}",
                if ordered {
                    "ordered_transport_not_idle"
                } else {
                    "specialist_transport_not_idle"
                }
            )
        })?
        .ok_or(if ordered {
            "ordered_transport_original_exit_proof_missing"
        } else {
            "specialist_transport_original_exit_proof_missing"
        })?;
        // The ordered closure independently probes and verifies its full plan.
        // Drop that probe while keeping the Root terminal row/write lock; a
        // new owner can acquire a file but cannot authorize another SDK call.
        if kind == "ordered-assessment-sdk" {
            drop(guard);
        } else {
            idle.push(guard);
            // These ordinary model assessments have no tool subprocess phase.
            // Human jobs retain their existing directive-specific closure.
            if matches!(
                parsed,
                AgentRole::SpaApiMapper
                    | AgentRole::IdentitySession
                    | AgentRole::DeepInvestigator
                    | AgentRole::ClientSide
            ) || (parsed==AgentRole::EvidenceReviewer && crate::agent_runtime::multi_agent::homogeneous::execution_policy::read_original_on(db,&scope.root_run_id)?.is_some()) {
                if request.get("humanDirectiveDispatch").is_none()
            {
                readonly.push(call);
                }
            }
        }
    }
    Ok(IdleSpecialists {
        _guards: idle,
        readonly,
        mapper,
    })
}

type MapperIdle = (
    CoordinatorLease,
    ScheduledChild,
    super::mapper_loop::VerifiedMapperIdle,
);
fn mapper_idle_for_root(
    db: &Connection,
    actor: &CoordinatorLease,
) -> Result<(Vec<execution_owner::NativeInvocationOwner>, Vec<MapperIdle>), String> {
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?;
    let declares = |p: &Value| p["schemaVersion"] == 15 || p.get("mapperLoopRule").is_some();
    let original: Option<String> = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let original = original
        .map(|raw| {
            serde_json::from_str::<Value>(&raw)
                .map_err(|_| "mapper_loop_lifetime_original_contract_invalid")
        })
        .transpose()?;
    let declared = mode
        .as_ref()
        .and_then(|m| m.bootstrap_dispatch())
        .as_ref()
        .is_some_and(declares)
        || original
            .as_ref()
            .is_some_and(|v| declares(&v["root"]["bootstrapDispatch"]));
    if declared {
        budget::model::mapper_loop_contract::require_protocol_on(db, actor)?;
    }
    let present: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='table' AND name='agent_mapper_loop_journal')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !present {
        if declared {
            return Err("mapper_loop_schema_missing".into());
        }
        // Old private audit arenas do not acquire a new journal or inode.
        return Ok((vec![], vec![]));
    }
    let mut query=db.prepare("SELECT assignment_id FROM main.agent_mapper_loop_journal WHERE root_run_id=?1
      UNION SELECT j.assignment_id FROM main.agent_mapper_loop_journal j JOIN agent_runs r ON r.id=j.child_run_id WHERE r.root_run_id=?1
      UNION SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND json_type(task_slice_json,'$.mapperLoop') IS NOT NULL
      UNION SELECT a.id FROM agent_assignments a JOIN agent_budget_entries b ON b.assignment_id=a.id AND b.root_run_id=a.coordinator_run_id
        WHERE a.coordinator_run_id=?1 AND ?2 AND b.kind='reserve' AND b.dimension='model_requests' AND b.amount=2
          AND (a.lane='read_only_analysis' OR a.role='spa_api_mapper' OR a.trigger_code='frontend_evidence_ready'
            OR EXISTS(SELECT 1 FROM agent_capability_leases c WHERE c.assignment_id=a.id AND c.capability='evidence.read'))
      UNION SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND ?2 AND (role='spa_api_mapper' OR trigger_code='frontend_evidence_ready')
      ORDER BY 1").map_err(|e|e.to_string())?;
    let ids = query
        .query_map(params![actor.root_run_id, declared], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("mapper_loop_lifetime_database_missing")?;
    let mut guards = vec![];
    let mut proofs = vec![];
    for assignment_id in ids {
        let run_id: String = db
            .query_row(
                "SELECT child_run_id FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2",
                params![assignment_id, actor.root_run_id],
                |r| r.get(0),
            )
            .map_err(|_| "mapper_loop_lifetime_original_assignment_missing")?;
        let child = ScheduledChild {
            assignment_id,
            run_id,
            role: AgentRole::SpaApiMapper,
        };
        let proof = super::mapper_loop::verify_for_idle_on(db, actor, &child)?;
        if proof.has_transport() {
            let guard = execution_owner::probe_native_invocation(
                std::path::Path::new(path),
                &actor.scan_id,
                actor.attempt_number,
                super::mapper_loop::INVOCATION_KIND,
                &child.run_id,
            )
            .map_err(|e| format!("mapper_loop_transport_not_idle:{e}"))?
            .ok_or("mapper_loop_original_transport_exit_missing")?;
            guards.push(guard);
        }
        proof.require_on(db, actor, &child)?;
        proofs.push((actor.clone(), child, proof));
    }
    Ok((guards, proofs))
}

// Called only under the original dispatch transaction after binding/history checks.
// A local replay can probe an existing inode, never CREATE a missing historical one.
pub(super) fn transport_owner(
    db: &Connection,
    call: &PendingCall,
    request: &Value,
    existing: bool,
) -> Result<execution_owner::NativeInvocationOwner, String> {
    if db.is_autocommit() {
        return Err("specialist_lifetime_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("specialist_lifetime_database_missing")?;
    let owner = budget::receipts::original_owner(
        db,
        &call.child.run_id,
        &call.lease,
        &call.child.assignment_id,
    )?;
    owner.verify(db)?;
    let kind = invocation_kind(request);
    if existing {
        execution_owner::probe_native_invocation(
            std::path::Path::new(path),
            &call.lease.scan_id,
            call.lease.attempt_number,
            kind,
            &call.child.run_id,
        )
        .map_err(|e| format!("specialist_transport_not_idle:{e}"))?
        .ok_or_else(|| "specialist_transport_original_exit_proof_missing".into())
    } else {
        execution_owner::claim_native_invocation(
            std::path::Path::new(path),
            &call.lease.scan_id,
            call.lease.attempt_number,
            kind,
            &call.child.run_id,
        )
    }
}
