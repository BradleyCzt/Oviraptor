// A03: a source observation becomes a supervision fact only after its original
// WebExecutor has paid, closed and delivered. Neither this reader nor a graph
// revision renews original C or grants another target action.
impl NativeCoordinatorFrame {
    #[cfg_attr(not(test), allow(dead_code))]
    fn new_closed_http_evidence(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        evidence_id: &str,
    ) -> Result<Self, String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return Self::new_closed_http_evidence(&tx, context, actor, evidence_id);
        }
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        let directory = resolve_agent_evidence_location(db, &actor.root_run_id)?;
        if directory != context.target_dir {
            return Err("root_new_evidence_directory_conflict".into());
        }
        // Validates signed attempt/work_dir and the unchanged original frontend.
        native_coordinator_frozen_evidence(db, &root)?;
        let mut frame = Self::new_http_evidence(db, actor, &directory, evidence_id)?;
        let author = frame.semantic["binding"]["authorRunId"]
            .as_str()
            .ok_or("root_new_evidence_author_invalid")?
            .to_string();
        let (assignment, summary, payload): (String, String, String) = db
            .query_row(
                "SELECT a.id,r.terminal_reason,m.payload_json FROM agent_runs r
             JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
             JOIN agent_messages m ON m.assignment_id=a.id AND m.from_run_id=r.id
               AND m.to_run_id=a.coordinator_run_id AND m.kind='execution_result'
             WHERE r.id=?1 AND a.coordinator_run_id=?2 AND a.state='completed'",
                params![author, actor.root_run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| "root_new_evidence_author_not_closed")?;
        let payload: JsonValue = serde_json::from_str(&payload)
            .map_err(|_| "root_new_evidence_author_receipt_invalid")?;
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: assignment.clone(),
            run_id: author.clone(),
            role: crate::agent_runtime::contract::AgentRole::WebExecutor,
        };
        if !original_executor_finished_replay_on(db, actor, &child, &payload, &summary)? {
            return Err("root_new_evidence_author_not_closed".into());
        }
        let usage = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
            db,
            actor,
            &assignment,
            &author,
        )?;
        if payload["usage"]["totalTokens"] != usage.total_tokens
            || payload["usage"]["modelRequests"] != usage.model_requests
        {
            return Err("root_new_evidence_author_cost_conflict".into());
        }
        for (sql,key) in [
            ("SELECT rowid,* FROM agent_assignments WHERE id=?1",assignment.as_str()),
            ("SELECT rowid,* FROM agent_runs WHERE id=?1",author.as_str()),
            ("SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id=?1",author.as_str()),
            ("SELECT rowid,* FROM agent_web_model_journal WHERE child_run_id=?1 ORDER BY rowid",author.as_str()),
            ("SELECT rowid,* FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result' ORDER BY rowid",assignment.as_str()),
        ] {
            frame.rows.push(NativeCoordinatorFrozenRows::capture(db,sql,&[key])?);
        }
        frame.semantic["closedAuthor"] = json!({"assignmentId":assignment,
            "runId":author,"usage":{"totalTokens":usage.total_tokens,"modelRequests":usage.model_requests},
            "directoryBindingHash":crate::agent_runtime::store::stable_hash(&directory.to_string_lossy())});
        Ok(frame)
    }
}
