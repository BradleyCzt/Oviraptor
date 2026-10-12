// Root consumes original human facts before any queue/role/model delivery action.
struct NativeHumanAssessment {
    frame: NativeCoordinatorFrame,
    paid: NativeCoordinatorTickReceipt,
}
fn native_human_assessment_ids(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<(String, String)>, String> {
    let mut q=db.prepare("SELECT d.id,d.status FROM agent_user_directives d WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.target_key=?3 AND d.root_run_id=?4 AND d.claim_run_id=?4 AND d.claim_lease_epoch=?5 AND d.claim_fencing_token=?6 AND (d.status='accepted' OR (d.status='completed' AND EXISTS(SELECT 1 FROM agent_directive_queue_actions a WHERE a.directive_id=d.id))) ORDER BY d.rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(
            params![
                actor.scan_id,
                actor.attempt_number,
                actor.target_key,
                actor.root_run_id,
                actor.lease_epoch,
                actor.fencing_token
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
fn native_coordinator_human_assessments(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<NativeHumanAssessment>, String> {
    let db = db::open(&context.db_path)?;
    check_native_human_directive_actor_on(&db, context, actor)?;
    let root = native_coordinator_root_context(context, actor);
    let consumer = &context
        .run
        .as_ref()
        .ok_or("root_human_consumer_missing")?
        .run_id;
    let items = native_human_assessment_ids(&db, actor)?;
    let mut assessments = Vec::new();
    for (id, status) in items {
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
        check_native_human_directive_actor_on(&tx, context, actor)?;
        let frame = NativeCoordinatorFrame::human_directive(&tx, actor, consumer, &id)?;
        frame.verify(&tx, &root, actor)?;
        if status == "completed" {
            // Historical preference replay must find its original paid fact.
            // Never manufacture an assessment for an already applied action.
            let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.root_run_id=q.root_run_id AND p.round=q.round AND p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='human-directive' AND json_extract(q.fact_json,'$.request.basis.changedFact')=json(?2))",params![actor.root_run_id,frame.fact().to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !present {
                return Err("root_human_original_assessment_missing".into());
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        let paid = native_coordinator_tick_for_frame(&root, actor, &frame)?;
        if status == "completed" && !paid.replayed {
            return Err("root_human_original_assessment_changed".into());
        }
        let assessment = NativeHumanAssessment { frame, paid };
        assessment.verify(&db, &root, actor)?;
        assessments.push(assessment);
    }
    Ok(assessments)
}
/// DESIGN §5.5：这条 root 若是同质任务，引导本轮是否已由主管全部裁决。
/// `None` 表示不是同质任务，继续走 legacy 的 suggestions 判定；读取失败一律上抛，
/// 不静默退回 legacy（避免同质 run 在库异常时被当成 legacy 轮次放过）。
pub(crate) fn homogeneous_directives_decided(
    db: &rusqlite::Connection,
    root_run_id: &str,
) -> Result<Option<bool>, String> {
    use crate::agent_runtime::multi_agent::homogeneous::{apply, store};
    if !apply::task_exists_for_root(db, root_run_id)? {
        return Ok(None);
    }
    Ok(Some(store::all_directives_decided(db, root_run_id)?))
}

impl NativeHumanAssessment {
    fn verify(
        &self,
        db: &rusqlite::Connection,
        root: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        native_coordinator_tick_authority(db, root, actor)?;
        self.frame.verify(db, root, actor)?;
        self.paid.tick.require_executable(db)?;
        if self.paid.tick.published(db, &self.paid.saved)? != Some(self.paid.event_sequence) {
            return Err("root_human_original_publication_changed".into());
        }
        let summary = self.paid.summary.as_json();
        // DESIGN §5.5：同质路径的引导裁决以 gated 表为准（新决策结构独立，
        // 不复用 camelCase 的 DecisionSummary）。已裁决 → 本轮继续；未裁决 → 保持 deferred。
        if let Some(decided) = homogeneous_directives_decided(db, &actor.root_run_id)? {
            return if decided {
                Ok(())
            } else {
                Err("root_human_directive_deferred".into())
            };
        }
        let suggestions = summary["suggestions"]
            .as_array()
            .ok_or("root_decision_schema_invalid")?;
        if !suggestions
            .iter()
            .all(|s| s.as_str() == Some(self.frame.step()))
        {
            return Err("root_human_step_not_bounded".into());
        }
        if suggestions.is_empty() {
            return Err("root_human_directive_deferred".into());
        }
        Ok(())
    }
}
fn apply_assessed_human_queue_actions(
    context: &AgentRunContext,
    directives: &mut HumanDirectiveContext,
    queue: &mut Vec<String>,
) -> Result<(), String> {
    let Some(actor) = &directives.lease else {
        return Ok(());
    };
    let actor = actor.clone();
    let assessments = native_coordinator_human_assessments(context, &actor)?;
    let root = native_coordinator_root_context(context, &actor);
    apply_human_queue_actions_checked(context, directives, queue, &|db| {
        for (id, _) in native_human_assessment_ids(db, &actor)? {
            if !assessments
                .iter()
                .any(|a| a.frame.semantic["directiveId"] == id)
            {
                return Err("root_human_unassessed_confirmation".into());
            }
        }
        for assessment in &assessments {
            assessment.verify(db, &root, &actor)?;
        }
        Ok(())
    })
}
fn defer_human_directives_for_capacity(
    context: &AgentRunContext,
    directives: &mut HumanDirectiveContext,
) -> Result<(), String> {
    let actor = directives
        .lease
        .as_ref()
        .ok_or("directive_root_unavailable")?;
    let db = db::open(&context.db_path)?;
    crate::agent_runtime::multi_agent::directive::defer_for_original_parent(&db, actor, &|db| {
        check_native_human_directive_actor_on(db, context, actor)
    })?;
    directives.items.clear();
    Ok(())
}
