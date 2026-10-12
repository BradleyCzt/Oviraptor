#[test]
fn new_evidence_input_parser_completed_replay_rejects_original_grant_invoice_surface_ack_and_usage_damage(
) {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("ne-ip-integrity");
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='input_parser'", [&f.root], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::InputParser,
    };
    let calls = f.calls();
    let before = client_delivery_all_rows(&db);
    // Deny SQL writes, including no-op INSERTs, throughout the completed audit.
    db.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
        use rusqlite::hooks::{AuthAction, Authorization};
        match ctx.action {
            AuthAction::Insert { .. }
            | AuthAction::Update { .. }
            | AuthAction::Delete { .. }
            | AuthAction::CreateTable { .. }
            | AuthAction::DropTable { .. }
            | AuthAction::CreateTrigger { .. }
            | AuthAction::DropTrigger { .. }
            | AuthAction::AlterTable { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        }
    }))
    .unwrap();

    assert!(input_parser_completed_original_replay(
        &db,
        f.lease(),
        &child,
        &f.h.context.target_dir
    )
    .unwrap());
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>)
        .unwrap();
    assert_eq!(client_delivery_all_rows(&db), before);
    for field in ["used_tokens", "used_cached_tokens", "used_requests"] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            &format!("UPDATE agent_runs SET {field}={field}+1 WHERE id=?1"),
            [&child.run_id],
        )
        .unwrap();
        let damaged = client_delivery_all_rows(&tx);
        assert_eq!(
            input_parser_completed_original_replay(&tx, f.lease(), &child, &f.h.context.target_dir)
                .err()
                .unwrap(),
            "input_parser_completed_usage_conflict"
        );
        assert_eq!(client_delivery_all_rows(&tx), damaged);
        drop(tx);
        assert_eq!(client_delivery_all_rows(&db), before);
    }
    // Cold damage only in disposable transactions; rollback restores each guard.
    for damage in [
        "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.grant.reservedModelTokens',3999) WHERE role='input_parser'",
        "DROP TRIGGER agent_specialist_call_immutable; UPDATE agent_specialist_calls SET response_hash='wrong' WHERE role='input_parser'",
        "UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.status',500) WHERE kind='request_record'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='input_assessment'",
        "UPDATE agent_messages SET delivery_attempts=2 WHERE kind='input_assessment'",
        "UPDATE agent_assignments SET finished_at='' WHERE role='input_parser'",
    ] {
        let tx=db.unchecked_transaction().unwrap();
        tx.execute_batch(damage).unwrap();
        let damaged=client_delivery_all_rows(&tx);
        assert!(input_parser_completed_original_replay(&tx,f.lease(),&child,&f.h.context.target_dir).is_err(), "{damage}");
        assert_eq!(client_delivery_all_rows(&tx),damaged);
        drop(tx);
        assert_eq!(client_delivery_all_rows(&db),before);
        assert_eq!(f.calls(),calls);
        assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
    }
}

#[test]
fn new_evidence_completed_input_parser_original_sdk_is_idle_for_root_terminal() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("ne-ip-terminal-idle");
    let db = db::open(&f.h.db_path).unwrap();
    let before = client_delivery_all_rows(&db);
    let calls = f.calls();
    let tx = db.unchecked_transaction().unwrap();
    let idle = crate::agent_runtime::multi_agent::specialist::require_idle_for_root(&tx, f.lease())
        .unwrap();
    assert_eq!(client_delivery_all_rows(&tx), before);
    drop(idle);
    drop(tx);
    assert_eq!(f.calls(), calls);
}

#[test]
fn new_evidence_completed_input_parser_all_root_financial_terminal_replays_without_sdk_or_rows() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("ne-ip-root-terminal");
    let db = db::open(&f.h.db_path).unwrap();
    crate::agent_runtime::multi_agent::budget::admission::require_settled_for_completion(
        &db, &f.root,
    )
    .unwrap();
    let calls = f.calls();
    let http = f.h.site_seen.lock().unwrap().len();
    finish_coordinator_run(&db, f.lease(), &f.outcome).unwrap();
    let terminal: (String, String, String, String, String) = db.query_row(
        "SELECT status,terminal_state,terminal_code,terminal_reason,finished_at FROM agent_runs WHERE id=?1",
        [&f.root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap();
    assert_eq!(terminal.0, "terminal");
    assert_eq!(terminal.1, f.outcome.terminal_status());
    assert_eq!(terminal.2, f.outcome.terminal_code());
    assert_eq!(
        terminal.3,
        crate::agent_runtime::secrets::redact_text_with(&f.outcome.detail(), None)
    );
    assert!(!terminal.4.is_empty());
    let before = client_delivery_all_rows(&db);
    finish_coordinator_run(&db, f.lease(), &f.outcome).unwrap();
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), http);
}
