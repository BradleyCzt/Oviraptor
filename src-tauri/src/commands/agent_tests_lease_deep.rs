#[test]
fn lease_deep_full_review_signed_expiry_paid_hook_and_readonly_replay() {
    EXPIRE_RECEIVED_DEEP_ONCE.with(|flag|flag.set(true));
    let mut f=followup_fixture();
    EXPIRE_RECEIVED_DEEP_ONCE.with(|flag|assert!(!flag.replace(false)));
    let db=db::open(&f.context.db_path).unwrap();
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.root_run_id],|row|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {scan_id:row.get(0)?,attempt_number:row.get(1)?,target_key:row.get(2)?,root_run_id:row.get(3)?,lease_epoch:row.get(4)?,fencing_token:row.get(5)?,lease_expires_at:row.get(6)?})).unwrap();
    let child=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",[&f.root_run_id],|row|Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:row.get(0)?,run_id:row.get(1)?,role:crate::agent_runtime::contract::AgentRole::DeepInvestigator})).unwrap();
    let parent=start_original_fixture_supervisor(&f.context.db_path,&actor).unwrap();
    f.context.supervision=Some(parent.ticket());
    let root=native_coordinator_root_context(&f.context,&actor);
    let before=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let frame=NativeCoordinatorFrame::lease_closed_mapper(&tx,&root,&actor,&child).unwrap();
    frame.verify(&tx,&root,&actor).unwrap();
    assert_eq!(frame.semantic["role"],"deep_investigator");
    assert_eq!(frame.semantic["targetRequestsGranted"],0);
    tx.commit().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
    db.execute("INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) SELECT scan_id,attempt_number+1,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE entity_id=?1 LIMIT 1",[&f.assessment_id]).unwrap();
    let changed=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let error=NativeCoordinatorFrame::lease_closed_mapper(&tx,&root,&actor,&child).err().unwrap();
    assert_eq!(error,"root_lease_deep_original_message_events_conflict");
    tx.commit().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==changed);
}

#[test]
fn lease_deep_ack_drift_during_actual_root_sdk_keeps_known_bill_without_publication_or_resend() {
    EXPIRE_RECEIVED_DEEP_ONCE.with(|flag| flag.set(true));
    let _fixture = followup_fixture_with_review_audit(FollowupLeaseFault::AckDrift, |context, session, db, calls, targets, outcome| {
        use crate::agent_runtime::multi_agent::budget;
        assert!(outcome.detail().contains("review_gap_investigation_failed:"), "{}", outcome.detail());
        assert_eq!(calls.lock().unwrap().len(), 13);
        assert_eq!(targets.lock().unwrap().len(), 1);
        let root = &session.lease.root_run_id;
        let child = db.query_row(
            "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator' AND state='completed'",
            [root], |row| Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id: row.get(0)?, run_id: row.get(1)?,
                role: crate::agent_runtime::contract::AgentRole::DeepInvestigator,
            }),
        ).unwrap();
        let call: String = db.query_row(
            "SELECT call_id FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='lease-expired'",
            [root], |row| row.get(0),
        ).unwrap();
        let (received, reported, input, output, total, requests): (i64, bool, i64, i64, i64, i64) = db.query_row(
            "SELECT count(*),json_extract(receipt_json,'$.usageReported'),json_extract(receipt_json,'$.usage.inputTokens'),json_extract(receipt_json,'$.usage.outputTokens'),json_extract(receipt_json,'$.usage.totalTokens'),json_extract(receipt_json,'$.usage.modelRequests') FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='received'",
            params![root, call], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        ).unwrap();
        assert_eq!((received, reported, requests), (1, true, 1));
        assert_eq!(input.checked_add(output), Some(total));
        assert!(total > 0);
        let unpublished: i64 = db.query_row(
            "SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2 AND phase='publication'",
            params![root, call], |row| row.get(0),
        ).unwrap();
        assert_eq!(unpublished, 0);
        // Verify the real dispatch/terminal physical proofs and original actual
        // cost rows. A test flag or aggregate consumed count is not the invoice.
        let tx = db.unchecked_transaction().unwrap();
        let invoice = budget::root::model::tick::Tick::audit_cost_for_continuation(&tx, root, &call).unwrap();
        assert_eq!(invoice["phase"], "received");
        assert_eq!(invoice["originalSemanticAvailable"], false);
        tx.commit().unwrap();
        assert_eq!(budget::balance(db, root, Some(&child.assignment_id), "model_requests").unwrap().consumed, 1);
        let actual_root_calls: i64 = db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",
            [root], |row| row.get(0),
        ).unwrap();
        assert_eq!(budget::balance(db, root, Some(""), "model_requests").unwrap().consumed, actual_root_calls);
        let fees = client_delivery_rows(db, "agent_budget_entries");
        let original_c: (i64, String) = db.query_row(
            "SELECT lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1",
            [root], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(original_c, (session.lease.lease_epoch, session.lease.fencing_token.clone()));
        let (candidate, revision, missing) = paid_proposal_candidate(db);
        let paid = super::tests::application_table_snapshot(db);
        assert!(multi_agent_investigate_review_gap(context, session, &candidate, revision, &missing).is_err());
        assert!(native_coordinator_lease_expiry_feedback(context, &session.lease, &child).is_err());
        assert_eq!(calls.lock().unwrap().len(), 13);
        assert_eq!(targets.lock().unwrap().len(), 1);
        assert_eq!(client_delivery_rows(db, "agent_budget_entries"), fees);
        assert_eq!(super::tests::application_table_snapshot(db), paid);
    });
    EXPIRE_RECEIVED_DEEP_ONCE.with(|flag| assert!(!flag.replace(false)));
}

#[test]
fn lease_deep_publication_abort_recovers_same_paid_call_before_first_gap_proposal_sdk() {
    lease_deep_publication_recovery(FollowupLeaseFault::PublicationAbort);
}

#[test]
fn lease_deep_publication_ignore_recovers_same_paid_call_before_first_gap_proposal_sdk() {
    lease_deep_publication_recovery(FollowupLeaseFault::PublicationIgnore);
}

fn lease_deep_publication_counts(
    db: &rusqlite::Connection,
    root: &str,
    call: &str,
) -> (i64, i64, i64, i64) {
    db.query_row(
        "SELECT (SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2 AND phase='publication'),(SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed' AND json_extract(payload_json,'$.callId')=?2),(SELECT count(*) FROM agent_root_tick_timeline_receipts WHERE root_run_id=?1 AND call_id=?2),(SELECT count(*) FROM agent_collaboration_events WHERE (entity_type='root_decision' OR event_type='root_decision') AND json_valid(payload_json) AND json_extract(payload_json,'$.callId')=?2)",
        params![root, call], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap()
}

// The immutable financial call excludes publication, which recovery may add.
// All original physical request/decision, invoice, terminal and fee rows remain.
fn lease_deep_original_paid_facts(db: &rusqlite::Connection, root: &str, call: &str) -> JsonValue {
    let tx = db.unchecked_transaction().unwrap();
    let proof = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::audit_cost_for_continuation(&tx, root, call).unwrap();
    assert_eq!(proof["phase"], "received");
    let rows = [
        "SELECT rowid,* FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2 AND phase<>'publication' ORDER BY phase",
        "SELECT rowid,* FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 ORDER BY phase",
        "SELECT rowid,* FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1 AND call_id=?2",
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id=?2 AND assignment_id='' ORDER BY rowid",
    ].map(|sql| NativeCoordinatorFrozenRows::capture(&tx, sql, &[root, call]).unwrap().fact());
    tx.commit().unwrap();
    json!(rows)
}

fn lease_deep_publication_recovery(fault: FollowupLeaseFault) {
    EXPIRE_RECEIVED_DEEP_ONCE.with(|flag| flag.set(true));
    let _fixture = followup_fixture_with_review_audit(
        fault,
        |context, session, db, calls, targets, outcome| {
            use crate::agent_runtime::multi_agent::budget;
            assert!(
                outcome
                    .detail()
                    .contains("review_gap_investigation_failed:"),
                "{}",
                outcome.detail()
            );
            assert_eq!(calls.lock().unwrap().len(), 13);
            assert_eq!(targets.lock().unwrap().len(), 1);
            let root = &session.lease.root_run_id;
            let child = db.query_row(
            "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator' AND state='completed'",
            [root], |row| Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id: row.get(0)?, run_id: row.get(1)?,
                role: crate::agent_runtime::contract::AgentRole::DeepInvestigator,
            }),
        ).unwrap();
            let call: String = db.query_row(
            "SELECT call_id FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='lease-expired'",
            [root], |row| row.get(0),
        ).unwrap();
            let counts: (i64, i64, i64, i64, i64) = db.query_row(
            "SELECT (SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2 AND phase='request'),(SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2 AND phase='decision'),(SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='dispatch'),(SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='received'),(SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1 AND call_id=?2)",
            params![root, call], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).unwrap();
            assert_eq!(counts, (1, 1, 1, 1, 1));
            let reported: (bool, i64, i64, i64, i64) = db.query_row(
            "SELECT json_extract(receipt_json,'$.usageReported'),json_extract(receipt_json,'$.usage.inputTokens'),json_extract(receipt_json,'$.usage.outputTokens'),json_extract(receipt_json,'$.usage.totalTokens'),json_extract(receipt_json,'$.usage.modelRequests') FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='received'",
            params![root, call], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).unwrap();
            assert_eq!(reported, (true, 10, 10, 20, 1));
            assert_eq!(lease_deep_publication_counts(db, root, &call), (0, 0, 0, 0));
            let old_facts = lease_deep_original_paid_facts(db, root, &call);
            let fees = client_delivery_rows(db, "agent_budget_entries");
            let root_cost = budget::balance(db, root, Some(""), "model_requests")
                .unwrap()
                .consumed;
            assert_eq!(
                budget::balance(db, root, Some(&child.assignment_id), "model_requests")
                    .unwrap()
                    .consumed,
                1
            );
            let original_c: (i64, String) = db.query_row(
            "SELECT lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1", [root],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
            assert_eq!(
                original_c,
                (
                    session.lease.lease_epoch,
                    session.lease.fencing_token.clone()
                )
            );
            let source_tables = [
                "agent_assignments",
                "agent_assignment_attempts",
                "agent_specialist_calls",
                "agent_messages",
                "agent_capability_leases",
                "agent_lane_leases",
                "agent_worker_expiry_receipts",
                "agent_worker_expiry_issuance_seals",
                "agent_root_mode_birth_seals",
                "agent_root_mode_definitions",
                "agent_coordinator_leases",
                "agent_budget_limits",
                "agent_review_requests",
                "agent_review_decisions",
                "agent_evidence_nodes",
                "agent_evidence_edges",
                "agent_evidence_revisions",
            ];
            let original_source = source_tables.map(|table| client_delivery_rows(db, table));
            let (candidate, revision, missing) = paid_proposal_candidate(db);
            let paid = super::tests::application_table_snapshot(db);
            assert!(
                native_coordinator_lease_expiry_feedback(context, &session.lease, &child).is_err()
            );
            assert!(multi_agent_investigate_review_gap(
                context, session, &candidate, revision, &missing
            )
            .is_err());
            assert_eq!(calls.lock().unwrap().len(), 13);
            assert_eq!(targets.lock().unwrap().len(), 1);
            assert_eq!(super::tests::application_table_snapshot(db), paid);
            assert_eq!(lease_deep_publication_counts(db, root, &call), (0, 0, 0, 0));

            // Remove only this fixture's failure. Keep the live original C, worker,
            // private seal, invoice and grant; no prepare/reissue/renewal occurs.
            db.execute_batch("DROP TRIGGER deep_lease_publication_fault")
                .unwrap();
            let recovered =
                native_coordinator_lease_expiry_feedback(context, &session.lease, &child).unwrap();
            assert!(recovered.replayed);
            let published_call: String = db.query_row(
            "SELECT json_extract(fact_json,'$.event.callId') FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='publication' AND json_extract(fact_json,'$.eventSequence')=?2",
            params![root,recovered.event_sequence], |row| row.get(0),
        ).unwrap();
            assert_eq!(published_call, call);
            assert_eq!(lease_deep_publication_counts(db, root, &call), (1, 1, 1, 1));
            assert_eq!(lease_deep_original_paid_facts(db, root, &call), old_facts);
            assert_eq!(client_delivery_rows(db, "agent_budget_entries"), fees);
            assert_eq!(
                budget::balance(db, root, Some(""), "model_requests")
                    .unwrap()
                    .consumed,
                root_cost
            );
            assert_eq!(calls.lock().unwrap().len(), 13);
            assert_eq!(targets.lock().unwrap().len(), 1);
            let published = super::tests::application_table_snapshot(db);
            let replay =
                native_coordinator_lease_expiry_feedback(context, &session.lease, &child).unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.event_sequence, recovered.event_sequence);
            assert_eq!(super::tests::application_table_snapshot(db), published);
            assert_eq!(calls.lock().unwrap().len(), 13);
            let proposals: i64 = db.query_row(
            "SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='investigator-proposal'", [root], |row| row.get(0),
        ).unwrap();
            assert_eq!(proposals, 0);

            // This is the first independent gap-proposal assessment, not a second
            // lease SDK. Only that new Root call may add one paid model request.
            multi_agent_investigate_review_gap(context, session, &candidate, revision, &missing)
                .unwrap();
            assert_eq!(calls.lock().unwrap().len(), 14);
            assert_eq!(targets.lock().unwrap().len(), 1);
            let proposal_requests: i64 = db.query_row(
            "SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='investigator-proposal'", [root], |row| row.get(0),
        ).unwrap();
            assert_eq!(proposal_requests, 1);
            let (new_call, phases): (String, i64) = db.query_row(
            "SELECT r.call_id,count(t.phase) FROM agent_root_tick_receipts r JOIN agent_root_tick_receipts t ON t.root_run_id=r.root_run_id AND t.call_id=r.call_id WHERE r.root_run_id=?1 AND r.phase='request' AND json_extract(r.fact_json,'$.request.basis.phase')='investigator-proposal' GROUP BY r.call_id", [root], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
            assert_ne!(new_call, call);
            assert_eq!(phases, 3);
            assert_eq!(
                lease_deep_publication_counts(db, root, &new_call),
                (1, 1, 1, 1)
            );
            lease_deep_original_paid_facts(db, root, &new_call);
            assert_eq!(
                budget::balance(db, root, Some(""), "model_requests")
                    .unwrap()
                    .consumed,
                root_cost + 1
            );
            assert_eq!(
                budget::balance(db, root, Some(&child.assignment_id), "model_requests")
                    .unwrap()
                    .consumed,
                1
            );
            let all_fees = client_delivery_rows(db, "agent_budget_entries");
            assert_eq!(&all_fees[..fees.len()], fees.as_slice());
            assert_eq!(
                source_tables.map(|table| client_delivery_rows(db, table)),
                original_source
            );
            assert_eq!(lease_deep_original_paid_facts(db, root, &call), old_facts);
            assert_eq!(lease_deep_publication_counts(db, root, &call), (1, 1, 1, 1));
            let complete = super::tests::application_table_snapshot(db);
            multi_agent_investigate_review_gap(context, session, &candidate, revision, &missing)
                .unwrap();
            assert_eq!(calls.lock().unwrap().len(), 14);
            assert_eq!(targets.lock().unwrap().len(), 1);
            assert_eq!(super::tests::application_table_snapshot(db), complete);
        },
    );
    EXPIRE_RECEIVED_DEEP_ONCE.with(|flag| assert!(!flag.replace(false)));
}
