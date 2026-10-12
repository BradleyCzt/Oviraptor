// Run before09 fix: actual producer → bounded real SDK → failed local delivery.
// Business/foreign invocation witnesses never modify the frozen source graph.
#[test]
fn client_side_actual_delivery_writer_rejects_unrelated_writes_and_preserves_original_grant() {
    for write in [
        "UPDATE projects SET name='forbidden delivery business write';",
        "UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical';",
        "UPDATE tool_invocations SET error_class='forbidden delivery invocation write' WHERE run_id='client-foreign-historical';",
    ] {
        for boundary in [
            "AFTER INSERT ON agent_messages WHEN NEW.from_agent='client_side'",
            "AFTER UPDATE OF acknowledged_at ON agent_messages WHEN NEW.from_agent='client_side' AND NEW.acknowledged_at<>''",
            "AFTER UPDATE OF status ON agent_runs WHEN NEW.role='client_side' AND NEW.terminal_state='completed'",
        ] {
            let mut f=client_hook_fixture("client-delivery-exact-writer",60000,20);
            client_delivery_foreign_invocation(&f);
            *f.model_boundary_sql.lock().unwrap() = Some(format!("CREATE TRIGGER client_delivery_fault {boundary} BEGIN {write} END;"));
            let before=client_hook_untouched_scope(&f);
            let costs=client_hook_target_costs(&f.db);
            let error=f.finish().unwrap_err();
            assert!(error.contains("client_side_delivery_write_rejected:"),"{boundary}: {error}");
            assert_eq!(client_hook_untouched_scope(&f),before);
            assert_eq!(client_hook_target_costs(&f.db),costs);
            client_hook_assert_supplier_closure(&f);
            client_delivery_assert_paid_and_running(&f);
        }
    }
}

#[test]
fn client_side_actual_delivery_event_ignore_rolls_back_only_local_publication() {
    let mut f = client_hook_fixture("client-delivery-event-ignore", 60000, 20);
    f.db.execute_batch(
        "CREATE TRIGGER client_delivery_event_ignore BEFORE INSERT ON agent_collaboration_events
        WHEN NEW.entity_type='mailbox_message' AND EXISTS(SELECT 1 FROM agent_messages m
          WHERE m.id=NEW.entity_id AND m.from_agent='client_side')
        BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let before = client_hook_untouched_scope(&f);
    let costs = client_hook_target_costs(&f.db);
    let error = f.finish().unwrap_err();
    assert!(
        error.contains("client_side_delivery_event_count_conflict"),
        "{error}"
    );
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    client_hook_assert_supplier_closure(&f);
    client_delivery_assert_paid_and_running(&f);
}

#[test]
fn client_side_actual_delivery_failure_does_not_enter_unprotected_pause_cleanup() {
    let mut f = client_hook_fixture("client-delivery-no-unwrapped-cleanup", 60000, 20);
    f.db.execute_batch("CREATE TRIGGER client_delivery_failure BEFORE INSERT ON agent_messages
        WHEN NEW.from_agent='client_side' BEGIN SELECT RAISE(ABORT,'local delivery fixture failure'); END;
        ").unwrap();
    *f.model_boundary_sql.lock().unwrap() = Some(
        "CREATE TRIGGER client_delivery_cleanup_business AFTER UPDATE OF status ON agent_runs
        WHEN NEW.role='client_side' AND NEW.status='paused'
        BEGIN UPDATE projects SET name='forbidden old cleanup write'; END;".into());
    let before = client_hook_untouched_scope(&f);
    let costs = client_hook_target_costs(&f.db);
    let error = f.finish().unwrap_err();
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert!(!f.db.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE name='forbidden old cleanup write')",
        [], |r| r.get::<_, bool>(0)).unwrap(), "actual paid delivery failure entered cleanup and changed business rows");
    let local_error = error.strip_prefix("executor_followup_failed:client_side:")
        .expect("original executor follow-up stage must remain explicit");
    assert!(local_error.contains("local delivery fixture failure")
        || local_error.starts_with("client_side_delivery_write_rejected:"), "{error}");
    assert!(
        !error.contains("specialist_cleanup:") && !error.contains("readonly_local_recovery:"),
        "{error}"
    );
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    client_hook_assert_supplier_closure(&f);
    client_delivery_assert_paid_and_running(&f);
}

#[test]
fn client_side_original_paid_local_delivery_has_four_events_and_pure_completed_replay() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler::ScheduledChild, specialist},
    };
    let mut f = client_hook_fixture("client-delivery-original-receipt-replay", 60000, 20);
    f.db.execute_batch("CREATE TRIGGER client_delivery_failure BEFORE INSERT ON agent_messages
        WHEN NEW.from_agent='client_side' BEGIN SELECT RAISE(ABORT,'local delivery fixture failure'); END;").unwrap();
    assert!(f
        .finish()
        .unwrap_err()
        .contains("local delivery fixture failure"));
    client_delivery_assert_paid_and_running(&f);
    f.db.execute_batch("DROP TRIGGER client_delivery_failure;")
        .unwrap();
    let (assignment,run,task):(String,String,String)=f.db.query_row(
        "SELECT id,child_run_id,task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'",
        [&f.root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let child = ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: run.clone(),
        role: AgentRole::ClientSide,
    };
    let lease = &f.session.as_ref().unwrap().lease;
    let received = {
        let tx = rusqlite::Transaction::new_unchecked(&f.db, rusqlite::TransactionBehavior::Immediate).unwrap();
        let receipt = specialist::received_for_reconciliation(&tx, lease, &child).unwrap();
        tx.rollback().unwrap();
        receipt
    };
    let payload = json!({"summary":received.text,"clientSideTask":serde_json::from_str::<JsonValue>(&task).unwrap()});
    let fees = client_delivery_rows(&f.db, "agent_budget_entries");
    assert!(!fees.is_empty()); // Original physical fee witness, not a fabricated late fact.
    let worker =
        crate::agent_runtime::multi_agent::attempts::current(&f.db, lease, &assignment).unwrap();
    let before_worker = (
        worker.id.clone(),
        worker.worker_id.clone(),
        worker.fencing_token.clone(),
        worker.expires_at.clone(),
        worker.lease_epoch,
    );
    let floor: i64 =
        f.db.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    deliver_readonly_assessment(
        &f.db,
        lease,
        &child,
        &received.usage,
        &payload,
        Some(&f.context.target_dir),
    )
    .unwrap();
    let worker =
        crate::agent_runtime::multi_agent::attempts::current(&f.db, lease, &assignment).unwrap();
    assert_eq!(
        (
            worker.id,
            worker.worker_id,
            worker.fencing_token,
            worker.expires_at,
            worker.lease_epoch
        ),
        before_worker
    );
    assert_eq!(worker.state, "completed");
    let events:Vec<(String,String,JsonValue)>=f.db.prepare("SELECT entity_type,entity_id,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence")
        .unwrap().query_map([floor],|r|Ok((r.get(0)?,r.get(1)?,serde_json::from_str(&r.get::<_,String>(2)?).unwrap())))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(
        events[0],
        (
            "assignment".into(),
            assignment.clone(),
            json!({"role":"client_side","state":"completed"})
        )
    );
    assert_eq!(
        events[1],
        (
            "agent_run".into(),
            run.clone(),
            json!({"role":"client_side","status":"terminal","terminalState":"completed"})
        )
    );
    assert_eq!(events[2].0, "mailbox_message");
    assert_eq!(events[2].1, events[3].1);
    assert_eq!(
        events[2].2,
        json!({"kind":"evidence_summary","deliveredAt":"","acknowledgedAt":""})
    );
    let at = events[3].2["acknowledgedAt"].as_str().unwrap();
    assert!(!at.is_empty());
    assert_eq!(
        events[3].2,
        json!({"kind":"evidence_summary","deliveredAt":at,"acknowledgedAt":at})
    );
    let table = client_delivery_rows(&f.db, "agent_budget_entries");
    for row in fees {
        assert!(
            table.contains(&row),
            "original paid fee row/rowid cannot change"
        );
    }
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    let before = client_delivery_all_rows(&f.db);
    deliver_readonly_assessment(
        &f.db,
        lease,
        &child,
        &received.usage,
        &payload,
        Some(&f.context.target_dir),
    )
    .unwrap();
    assert_eq!(client_delivery_all_rows(&f.db), before);
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn client_side_original_paid_delivery_rejects_replaced_canonical_message_emitter() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler::ScheduledChild, specialist},
    };
    for body in [
        "UPDATE projects SET name='forbidden canonical delivery write';",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
            SELECT scan_id,attempt_number,'mailbox_message',NEW.id,'mailbox_message',json_object('kind','wrong')
              FROM agent_runs WHERE id=NEW.root_run_id;",
    ] {
        let mut f=client_hook_fixture("client-delivery-canonical-message",60000,20);
        f.db.execute_batch("CREATE TRIGGER client_delivery_failure BEFORE INSERT ON agent_messages
            WHEN NEW.from_agent='client_side' BEGIN SELECT RAISE(ABORT,'local delivery fixture failure'); END;").unwrap();
        assert!(f.finish().is_err()); // Supplier actually closed and SDK actually received first.
        client_hook_assert_supplier_closure(&f);
        f.db.execute_batch(&format!("DROP TRIGGER client_delivery_failure;
            DROP TRIGGER agent_collaboration_message_insert;
            CREATE TRIGGER agent_collaboration_message_insert AFTER INSERT ON agent_messages BEGIN {body} END;")).unwrap();
        let (assignment,run,task):(String,String,String)=f.db.query_row(
            "SELECT id,child_run_id,task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'",
            [&f.root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let child=ScheduledChild{assignment_id:assignment,run_id:run,role:AgentRole::ClientSide};
        let lease=&f.session.as_ref().unwrap().lease;
        let receipt={
        let tx = rusqlite::Transaction::new_unchecked(&f.db, rusqlite::TransactionBehavior::Immediate).unwrap();
        let receipt = specialist::received_for_reconciliation(&tx, lease, &child).unwrap();
        tx.rollback().unwrap();
        receipt
    };
        let payload=json!({"summary":receipt.text,"clientSideTask":serde_json::from_str::<JsonValue>(&task).unwrap()});
        let before=client_delivery_all_rows(&f.db);
        let error=deliver_readonly_assessment(&f.db,lease,&child,&receipt.usage,&payload,Some(&f.context.target_dir)).unwrap_err();
        assert!(error.contains("client_side_delivery_emitter_conflict"),"{error}");
        assert_eq!(client_delivery_all_rows(&f.db),before);
        assert_eq!(f.model_seen.lock().unwrap().len(),1);assert_eq!(f.site_seen.lock().unwrap().len(),1);
    }
}
