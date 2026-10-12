// Layered Stage R2 negatives use actual finish and actual source/SDK production.
#[test]
fn client_side_actual_finish_original_source_update_and_replace_roll_back_new_grant() {
    for mutation in [
        "UPDATE agent_evidence_nodes SET created_at='2000-01-01 00:00:00' WHERE kind='request_record';",
        "INSERT OR REPLACE INTO agent_evidence_nodes SELECT * FROM agent_evidence_nodes WHERE kind='request_record';",
    ] {
        let mut f=client_hook_fixture("client-source-physical-fault",60000,20);
        client_hook_add_foreign_historical_root(&f);
        f.db.execute_batch(&format!("PRAGMA recursive_triggers=OFF; CREATE TRIGGER client_grant_fault AFTER INSERT ON agent_assignments
            WHEN NEW.role='client_side' BEGIN {mutation} UPDATE projects SET name='forbidden business mutation'; UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical'; END;")).unwrap();
        let before=client_hook_untouched_scope(&f);
        let costs=client_hook_target_costs(&f.db);
        let error=f.finish().unwrap_err();
        assert!(error.contains("client_side_grant_write_rejected:"),"{mutation}: {error}");
        assert_eq!(client_hook_untouched_scope(&f),before,"grant must roll back physical source and business rows");
        assert_eq!(client_hook_target_costs(&f.db),costs);
        client_hook_assert_supplier_closure(&f);
        client_hook_assert_no_new_grant(&f);
    }
}
#[test]
fn client_side_actual_finish_preserves_reviewer_headroom_from_original_fresh_limits() {
    for (tokens, requests) in [(15000, 20), (60000, 1)] {
        let mut f = client_hook_fixture("client-source-headroom", tokens, requests);
        let before = client_hook_untouched_scope(&f);
        let costs = client_hook_target_costs(&f.db);
        let error = f.finish().unwrap_err();
        assert!(
            error.contains("client_side_budget_headroom_missing_reviewer_floor_preserved"),
            "{tokens}/{requests}: {error}"
        );
        assert_eq!(client_hook_untouched_scope(&f), before);
        assert_eq!(client_hook_target_costs(&f.db), costs);
        client_hook_assert_supplier_closure(&f);
        client_hook_assert_no_new_grant(&f);
    }
}

#[test]
fn client_side_actual_finish_uses_reviewer_floor_15000_not_legacy_8000() {
    // 22k leaves room under a legacy 8k floor+8k grant (16k) but not under 15k+8k (23k).
    let mut f = client_hook_fixture("client-source-floor-15k", 22_000, 20);
    let before = client_hook_untouched_scope(&f);
    let costs = client_hook_target_costs(&f.db);
    let error = f.finish().unwrap_err();
    assert!(
        error.contains("client_side_budget_headroom_missing_reviewer_floor_preserved"),
        "22k must preserve 15k Reviewer floor: {error}"
    );
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    client_hook_assert_supplier_closure(&f);
    client_hook_assert_no_new_grant(&f);
}

#[test]
fn client_side_actual_finish_cross_coordinator_cannot_adopt_original_supplier_or_control() {
    use crate::agent_runtime::multi_agent::lease;
    let mut f = client_hook_fixture("client-source-cross-c", 60000, 20);
    // Join C1 before replacing its actual lease; do not race its renewal thread.
    let old = f.session.take().unwrap();
    let original = old.lease.clone();
    let mapper = old.mapper.clone();
    let executor = old.executor.clone();
    drop(old);
    f.db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&f.root]).unwrap();
    let actor = lease::acquire_coordinator_lease(
        &f.db,
        &original.scan_id,
        original.attempt_number,
        &original.target_key,
        &f.root,
        600,
    )
    .unwrap();
    assert_eq!(actor.lease_epoch, original.lease_epoch + 1);
    let supervisor = start_original_fixture_supervisor(&f.context.db_path, &actor).unwrap();
    f.context.supervision = Some(supervisor.ticket());
    f.session = Some(MultiAgentSession {
        supervisor,
        lease: actor,
        mapper,
        executor,
    });
    let before = client_hook_untouched_scope(&f);
    let control = client_hook_rows(&f.db, "agent_root_budget_attempts");
    let costs = client_hook_target_costs(&f.db);
    let supplier_rows = client_hook_original_supplier_rows(&f);
    assert!(
        f.finish().is_err(),
        "new C cannot use old supplier or issue ClientSide work"
    );
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    assert_eq!(client_hook_original_supplier_rows(&f), supplier_rows);
    assert_eq!(
        client_hook_rows(&f.db, "agent_root_budget_attempts"),
        control
    );
    assert_eq!(f.model_seen.lock().unwrap().len(), 0);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE role='client_side'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn client_side_actual_finish_body_tamper_stops_before_new_grant_without_refunding_http() {
    let mut f = client_hook_fixture("client-source-body-tamper", 60000, 20);
    fs::write(
        f.context.target_dir.join("agent-http/0001.body"),
        b"changed captured body",
    )
    .unwrap();
    let before = client_hook_untouched_scope(&f);
    let costs = client_hook_target_costs(&f.db);
    let error = f.finish().unwrap_err();
    assert!(
        error.contains("client_side_original_http_fact_invalid"),
        "{error}"
    );
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    client_hook_assert_supplier_closure(&f);
    client_hook_assert_no_new_grant(&f);
}
#[test]
fn client_side_actual_finish_closure_trigger_rolls_back_source_and_business_preserving_sdk_fee() {
    for boundary in [
        "AFTER INSERT ON agent_messages WHEN NEW.from_agent='client_side'",
        "AFTER UPDATE OF acknowledged_at ON agent_messages WHEN NEW.from_agent='client_side' AND NEW.acknowledged_at<>''",
        "AFTER UPDATE OF status ON agent_runs WHEN NEW.role='client_side' AND NEW.terminal_state='completed'",
    ] {
        let mut f=client_hook_fixture("client-source-delivery-fault",60000,20);
        client_hook_add_foreign_historical_root(&f);
        *f.model_boundary_sql.lock().unwrap() = Some(format!("CREATE TRIGGER client_delivery_fault {boundary} BEGIN
            UPDATE agent_evidence_nodes SET created_at='2000-01-01 00:00:00' WHERE kind='request_record';
            UPDATE projects SET name='forbidden delivery mutation'; UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical'; END;"));
        let before=client_hook_untouched_scope(&f);
        let costs=client_hook_target_costs(&f.db);
        let error=f.finish().unwrap_err();
        assert!(error.contains("client_side_delivery_write_rejected:"),"{boundary}: {error}");
        client_delivery_assert_paid_and_running(&f);
        assert_eq!(client_hook_untouched_scope(&f),before);
        assert_eq!(client_hook_target_costs(&f.db),costs);
        client_hook_assert_supplier_closure(&f);
        assert_eq!(f.model_seen.lock().unwrap().len(),1,"receipt delivery cannot repeat provider I/O");
        assert_eq!(f.site_seen.lock().unwrap().len(),1);
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE role='client_side' AND state='received'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert!(f.db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries e JOIN agent_assignments a ON a.id=e.assignment_id
            JOIN agent_assignment_attempts x ON x.id=e.lease_attempt_id AND x.assignment_id=a.id AND x.child_run_id=a.child_run_id
            WHERE a.role='client_side' AND e.dimension='model_requests' AND e.kind='consume' AND e.amount=1)",[],|r|r.get::<_,bool>(0)).unwrap());
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_messages WHERE from_agent='client_side'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_assignments WHERE role='client_side' AND state='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(f.db.query_row("SELECT count(*) FROM sentinel_findings",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}
