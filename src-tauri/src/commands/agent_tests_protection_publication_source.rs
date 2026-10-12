// Re-arm the one-shot paid checkpoint until the actual protection invoice
// is committed; the preceding HTTP assessment must remain untouched.
fn protection_arm_original_publication_fault(
    path: PathBuf,
    root: String,
    sql: String,
    installed: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| {
        assert!(slot.borrow().is_none(),"no prior fixture paid boundary");
        *slot.borrow_mut()=Some(Box::new(move || {
            let db=db::open(&path).unwrap();
            let received:i64=db.query_row("SELECT count(*) FROM agent_root_model_journal j JOIN agent_root_tick_receipts q ON q.root_run_id=j.root_run_id AND q.call_id=j.call_id AND q.phase='request' WHERE j.root_run_id=?1 AND j.phase='received' AND json_extract(q.fact_json,'$.request.basis.phase')='protection-changed'",[&root],|r|r.get(0)).unwrap();
            if received==0 { protection_arm_original_publication_fault(path,root,sql,installed); return; }
            assert_eq!(received,1);
            assert_eq!(protection_paid_phases(&db,&root,"decision"),1);
            db.execute_batch(&sql).unwrap();
            installed.store(true,std::sync::atomic::Ordering::SeqCst);
        }));
    });
}

// Actual paid protection publication rejects trigger-origin source DML at its
// existing limited authorizer. No source mutation is executed: these tests do
// not claim to reach the later source-revalidation guard with a changed source.
#[derive(Clone, Copy)]
enum ProtectionPublicationSourceWrite {
    ChildCached,
    ClosingAckEvent,
}

fn protection_publication_source_write_denied_case(mode: ProtectionPublicationSourceWrite) {
    let mut f = ProtectionOriginalFixture::new(match mode {
        ProtectionPublicationSourceWrite::ChildCached => {
            "protection-publication-cached-write-denied"
        }
        ProtectionPublicationSourceWrite::ClosingAckEvent => {
            "protection-publication-ack-event-delete-denied"
        }
    });
    let db = db::open(&f.h.db_path).unwrap();
    let signed = f.fact(&db);
    let author = signed["runId"].as_str().unwrap();
    let assignment = signed["assignmentId"].as_str().unwrap();
    // Values come from this fixture's actual immutable first-response receipt.
    let root_literal: String = db
        .query_row("SELECT quote(?1)", [&f.actor.root_run_id], |r| r.get(0))
        .unwrap();
    let author_literal: String = db
        .query_row("SELECT quote(?1)", [author], |r| r.get(0))
        .unwrap();
    let assignment_literal: String = db
        .query_row("SELECT quote(?1)", [assignment], |r| r.get(0))
        .unwrap();
    let mutation = match mode {
        ProtectionPublicationSourceWrite::ChildCached => format!(
            "UPDATE agent_runs SET used_cached_tokens=used_cached_tokens+1 WHERE id={author_literal};"),
        ProtectionPublicationSourceWrite::ClosingAckEvent => format!(
            "DELETE FROM agent_collaboration_events WHERE entity_type='mailbox_message'
             AND entity_id IN (SELECT id FROM agent_messages WHERE assignment_id={assignment_literal}
               AND kind='execution_result') AND json_extract(payload_json,'$.acknowledgedAt')<>'';"),
    };
    // Keep production guards untouched. Install only after the actual SDK bill
    // and terminal capture commit: SQLite compiles trigger programs even when
    // their WHEN condition is false, so pre-SDK installation could deny admission.
    let trigger_sql = format!(
        "CREATE TRIGGER fixture_protection_publication_source_write
        AFTER INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication'
        AND NEW.root_run_id={root_literal} AND EXISTS(SELECT 1 FROM agent_root_tick_receipts q
          WHERE q.root_run_id=NEW.root_run_id AND q.call_id=NEW.call_id AND q.phase='request'
          AND json_extract(q.fact_json,'$.request.basis.phase')='protection-changed')
        BEGIN {mutation} END;"
    );
    let source_triggers = protection_physical_triggers(
        &db,
        match mode {
            ProtectionPublicationSourceWrite::ChildCached => "agent_runs",
            ProtectionPublicationSourceWrite::ClosingAckEvent => "agent_collaboration_events",
        },
    );
    let source_before = {
        let tx = db.unchecked_transaction().unwrap();
        native_original_protection_response_on(&tx, &f.actor, &f.receipt)
            .unwrap()
            .bounded_fact()
    };
    let sdk = f.h.model_seen.lock().unwrap().len();
    let calls = protection_paid_root_calls(&f);
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    let path = f.h.db_path.clone();
    let installed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let boundary_installed = installed.clone();
    let before_phases = followup_original_root_phase_counts(
        &db,
        &f.actor.root_run_id,
        &f.h.model_seen.lock().unwrap(),
    );
    protection_arm_original_publication_fault(
        path,
        f.actor.root_run_id.clone(),
        trigger_sql,
        boundary_installed,
    );
    let result =
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome);
    let leftover = ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| slot.borrow_mut().take());
    assert!(
        leftover.is_none() && installed.load(std::sync::atomic::Ordering::SeqCst),
        "the trigger was installed at the actual received/paid checkpoint"
    );
    let error = result.unwrap_err();
    assert!(
        error.to_ascii_lowercase().contains("not authorized"),
        "actual authorizer denial: {error}"
    );
    let added = protection_assert_original_finish_phase_delta(&f, &db, before_phases);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + added);
    assert_eq!(protection_paid_root_calls(&f), calls + added);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        cost + i64::try_from(added).unwrap()
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "request"),
        1
    );
    let received: i64 = db.query_row("SELECT count(*) FROM agent_root_model_journal j JOIN agent_root_tick_receipts q ON q.root_run_id=j.root_run_id AND q.call_id=j.call_id AND q.phase='request' WHERE j.root_run_id=?1 AND j.phase='received' AND json_extract(q.fact_json,'$.request.basis.phase')='protection-changed'", [&f.actor.root_run_id], |r|r.get(0)).unwrap();
    assert_eq!(
        received, 1,
        "actual received invoice belongs to the model journal"
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "decision"),
        1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    let (call,raw): (String,String)=db.query_row(
        "SELECT call_id,fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'
         AND json_extract(fact_json,'$.request.basis.phase')='protection-changed'",
        [&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let published_events:i64=db.query_row(
        "SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'
         AND json_extract(payload_json,'$.callId')=?2",params![f.actor.root_run_id,call],|r|r.get(0)).unwrap();
    assert_eq!(
        published_events, 0,
        "tentative Root publication event rolls back"
    );
    let timeline:i64=db.query_row(
        "SELECT count(*) FROM agent_root_tick_timeline_receipts WHERE root_run_id=?1 AND call_id=?2",
        params![f.actor.root_run_id,call],|r|r.get(0)).unwrap();
    assert_eq!(timeline, 0);
    // The source closure committed before assessment; no cached/ACK corruption
    // survives the failed publication transaction and no replacement is issued.
    f.assert_closed_read_only(&db, true);
    f.assert_original_read_only(&db, true);
    let closed_before = {
        let tx = db.unchecked_transaction().unwrap();
        native_audit_closed_protection_response_on(&tx, &f.h.context, &f.actor, &f.receipt)
            .unwrap()
            .bounded
    };
    let (cached,ack_events):(i64,i64)=db.query_row(
        "SELECT r.used_cached_tokens,(SELECT count(*) FROM agent_collaboration_events e
           JOIN agent_messages m ON m.id=e.entity_id WHERE m.assignment_id=?2 AND m.kind='execution_result'
           AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type
           AND e.payload_json=json_object('kind',m.kind,'deliveredAt',m.delivered_at,'acknowledgedAt',m.acknowledged_at)
           AND m.acknowledged_at<>'') FROM agent_runs r WHERE r.id=?1",
        params![author,assignment],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((cached, ack_events), (0, 1));
    let source_rows_before = {
        let tx = db.unchecked_transaction().unwrap();
        native_audit_closed_protection_response_on(&tx, &f.h.context, &f.actor, &f.receipt)
            .unwrap()
            .rows
            .iter()
            .map(NativeCoordinatorFrozenRows::fact)
            .collect::<Vec<_>>()
    };
    let paid = super::tests::application_table_snapshot(&db);
    let fees = client_delivery_rows(&db, "agent_budget_entries");
    for _ in 0..2 {
        let error = native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt)
            .err()
            .unwrap();
        assert!(
            error.to_ascii_lowercase().contains("not authorized"),
            "{error}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), paid);
        assert_eq!(client_delivery_rows(&db, "agent_budget_entries"), fees);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + added);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        let tx = db.unchecked_transaction().unwrap();
        let source = native_original_protection_response_on(&tx, &f.actor, &f.receipt).unwrap();
        assert_eq!(source.bounded_fact(), source_before);
        let closed =
            native_audit_closed_protection_response_on(&tx, &f.h.context, &f.actor, &f.receipt)
                .unwrap();
        assert_eq!(closed.bounded, closed_before);
        assert_eq!(
            closed
                .rows
                .iter()
                .map(NativeCoordinatorFrozenRows::fact)
                .collect::<Vec<_>>(),
            source_rows_before
        );
    }
    assert_eq!(
        protection_physical_triggers(
            &db,
            match mode {
                ProtectionPublicationSourceWrite::ChildCached => "agent_runs",
                ProtectionPublicationSourceWrite::ClosingAckEvent => "agent_collaboration_events",
            }
        ),
        source_triggers
    );
    db.execute_batch("DROP TRIGGER fixture_protection_publication_source_write")
        .unwrap();
    let decision =
        native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).unwrap();
    assert!(
        decision.replayed,
        "recover only the original actual received call"
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        1
    );
    assert_eq!(client_delivery_rows(&db, "agent_budget_entries"), fees);
    let recovered_call:String=db.query_row(
        "SELECT call_id FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='publication'
         AND call_id=?2",params![f.actor.root_run_id,call],|r|r.get(0)).unwrap();
    assert_eq!(recovered_call, call);
    assert_eq!(
        db.query_row(
            "SELECT fact_json FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='request'",
            [&call],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt)
            .unwrap()
            .replayed
    );
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(client_delivery_rows(&db, "agent_budget_entries"), fees);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + added);
    assert_eq!(protection_paid_root_calls(&f), calls + added);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    f.assert_closed_read_only(&db, true);
}

#[test]
fn protection_paid_publication_cached_source_write_denied_preserves_original_bill_and_replays() {
    protection_publication_source_write_denied_case(ProtectionPublicationSourceWrite::ChildCached);
}
#[test]
fn protection_paid_publication_closing_ack_event_source_write_denied_preserves_bill_and_replays() {
    protection_publication_source_write_denied_case(
        ProtectionPublicationSourceWrite::ClosingAckEvent,
    );
}
