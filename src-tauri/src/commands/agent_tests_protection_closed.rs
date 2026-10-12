// Historical closure is a separate proof from the first-response receipt.
impl ProtectionOriginalFixture {
    fn assert_closed_read_only(&self, db: &rusqlite::Connection, want_ok: bool) {
        let check = |snapshot: &rusqlite::Connection| {
            let before = super::tests::application_table_snapshot(snapshot);
            let changes = snapshot.total_changes();
            let sdk = self.h.model_seen.lock().unwrap().len();
            let gets = self.h.site_seen.lock().unwrap().len();
            match native_closed_protection_response_on(
                snapshot,
                &self.h.context,
                &self.actor,
                &self.receipt,
            ) {
                Ok(proof) => {
                    assert!(want_ok, "unclosed/corrupted author must be rejected");
                    let fact = proof.bounded_fact();
                    assert_eq!(fact["closedAuthorVerified"], true);
                    assert_eq!(fact["executionGrant"], false);
                    assert_eq!(fact["targetRequestsGranted"], 0);
                }
                Err(error) => assert!(!want_ok, "closed source rejected: {error}"),
            }
            assert_eq!(super::tests::application_table_snapshot(snapshot), before);
            assert_eq!(snapshot.total_changes(), changes);
            assert_eq!(self.h.model_seen.lock().unwrap().len(), sdk);
            assert_eq!(self.h.site_seen.lock().unwrap().len(), gets);
        };
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().unwrap();
            check(&tx);
            tx.commit().unwrap();
        } else {
            check(db);
        }
    }
}

#[test]
fn protection_closed_actual_waf_requires_settlement_and_original_ack_without_graph() {
    let mut f = ProtectionOriginalFixture::new("protection-closed-original");
    let db = db::open(&f.h.db_path).unwrap();
    f.assert_original_read_only(&db, true);
    let author = f.fact(&db)["runId"].as_str().unwrap().to_string();
    let observed: i64 = db.query_row("SELECT count(*) FROM agent_evidence_nodes WHERE root_run_id=?1 AND created_by_run_id=?2 AND kind='request_record' AND provenance='observed'", params![f.actor.root_run_id,author], |r| r.get(0)).unwrap();
    assert_eq!(
        observed, 0,
        "protected response has no normal observed request graph"
    );
    f.assert_closed_read_only(&db, false);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    f.assert_closed_read_only(&db, true);
    f.assert_closed_read_only(&db, true);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    drop(f.session.take());
    f.assert_closed_read_only(&db, true);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(tx.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1", [&f.actor.root_run_id]).unwrap(), 1);
    assert!(
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&tx, &f.actor)
            .is_err()
    );
    f.assert_closed_read_only(&tx, true);
    f.assert_original_read_only(&tx, true);
    tx.rollback().unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_closed_original_ack_usage_and_closing_event_cold_damage_are_read_only() {
    let mut f = ProtectionOriginalFixture::new("protection-closed-damage");
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    f.assert_closed_read_only(&db, true);
    let fact = f.fact(&db);
    let assignment = fact["assignmentId"].as_str().unwrap();
    let author = fact["runId"].as_str().unwrap();
    for kind in ["web-executor-sdk", "web-executor-original-execution"] {
        let held = crate::agent_runtime::execution_owner::probe_native_invocation(
            &f.h.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            kind,
            author,
        )
        .unwrap()
        .expect("original invocation inode must exist");
        f.assert_closed_read_only(&db, false);
        f.assert_original_read_only(&db, true);
        drop(held);
        f.assert_closed_read_only(&db, true);
    }
    for (table, sql, key) in [
        ("agent_messages", "UPDATE agent_messages SET acknowledged_at='' WHERE assignment_id=?1 AND kind='execution_result'", assignment),
        ("agent_runs", "UPDATE agent_runs SET used_cached_tokens=used_cached_tokens+1 WHERE id=?1", author),
        ("agent_assignments", "UPDATE agent_assignments SET budget_settled_at='' WHERE id=?1", assignment),
        ("agent_assignment_attempts", "UPDATE agent_assignment_attempts SET finished_at='' WHERE child_run_id=?1", author),
    ] {
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        let triggers = protection_physical_triggers(&tx, table);
        for (name, _) in &triggers {
            tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\""))).unwrap();
        }
        assert_eq!(tx.execute(sql, [key]).unwrap(), 1);
        for (_, ddl) in &triggers { tx.execute_batch(ddl).unwrap(); }
        assert_eq!(protection_physical_triggers(&tx, table), triggers);
        f.assert_closed_read_only(&tx, false);
        f.assert_original_read_only(&tx, true);
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        f.assert_closed_read_only(&db, true);
    }
    let tx = db.unchecked_transaction().unwrap();
    let triggers = protection_physical_triggers(&tx, "agent_collaboration_events");
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
    }
    assert_eq!(tx.execute("DELETE FROM agent_collaboration_events WHERE rowid=(SELECT rowid FROM agent_collaboration_events WHERE entity_type='mailbox_message' AND entity_id IN (SELECT id FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result') AND json_extract(payload_json,'$.acknowledgedAt')<>'' LIMIT 1)", [assignment]).unwrap(), 1);
    for (_, ddl) in &triggers {
        tx.execute_batch(ddl).unwrap();
    }
    assert_eq!(
        protection_physical_triggers(&tx, "agent_collaboration_events"),
        triggers
    );
    f.assert_closed_read_only(&tx, false);
    f.assert_original_read_only(&tx, true);
    tx.rollback().unwrap();
    f.assert_closed_read_only(&db, true);
    let tx = db.unchecked_transaction().unwrap();
    let triggers = protection_physical_triggers(&tx, "agent_collaboration_events");
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
    }
    assert_eq!(tx.execute("INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json,created_at) SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json,created_at FROM agent_collaboration_events WHERE entity_type='mailbox_message' AND entity_id IN (SELECT id FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result') AND json_extract(payload_json,'$.acknowledgedAt')<>'' LIMIT 1", [assignment]).unwrap(), 1);
    for (_, ddl) in &triggers {
        tx.execute_batch(ddl).unwrap();
    }
    assert_eq!(
        protection_physical_triggers(&tx, "agent_collaboration_events"),
        triggers
    );
    f.assert_closed_read_only(&tx, false);
    f.assert_original_read_only(&tx, true);
    tx.rollback().unwrap();
    f.assert_closed_read_only(&db, true);
}
