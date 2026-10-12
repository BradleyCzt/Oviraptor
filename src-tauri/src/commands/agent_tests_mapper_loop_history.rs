fn mapper_loop_history_failure_outbox_inventory(path: &Path, root: &str) {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("actual original history fixture must remain readable");
    for (name,sql) in [
        ("handoffMessage", "SELECT json_object('id',id,'assignment',assignment_id,'revision',evidence_revision,'kind',kind,'created',created_at,'delivered',delivered_at,'ack',acknowledged_at,'attempts',delivery_attempts) FROM agent_messages WHERE root_run_id=?1 AND kind='execution_assignment' ORDER BY rowid"),
        ("handoffOutbox", "SELECT json_object('sequence',sequence,'scan',scan_id,'attempt',attempt_number,'entityType',entity_type,'entityId',entity_id,'eventType',event_type,'payload',json(payload_json),'created',created_at) FROM agent_collaboration_events WHERE entity_id IN (SELECT id FROM agent_messages WHERE root_run_id=?1 AND kind='execution_assignment') ORDER BY sequence"),
    ] {
        let rows=db.prepare(sql).and_then(|mut query|query.query_map([root],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>());
        match rows {Ok(rows)=>eprintln!("actual history failure {name}: count={} {rows:?}",rows.len()),Err(error)=>eprintln!("actual history failure {name}: query error {error}")}
    }
}

// The production QueryOnly API reads original prepared history, never a live session.
#[test]
fn mapper_loop_history_actual_two_round_handoff_scope_and_configuration_drift_are_readonly() {
    use crate::agent_runtime::multi_agent::budget;
    let (mut f, session, seen) = mapper_loop_full_production_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = seen.lock().unwrap().len();
    let read = || {
        let result = read_native_mapper_prepared_history_at(
            &f.context.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.actor.root_run_id,
        );
        if result.is_err() {
            mapper_loop_history_failure_outbox_inventory(&f.context.db_path, &f.actor.root_run_id);
        }
        result.unwrap().unwrap()
    };
    let history = read();
    assert_eq!(history["stage"], "OriginalHandoffIssued");
    assert_eq!(history["executionAllowed"], false);
    assert_eq!(history["rootFinalOutcomeVerified"], false);
    assert_eq!(history["rootTerminal"], false);
    assert_eq!(history["assignmentId"], session.mapper.assignment_id);
    assert_eq!(history["childRunId"], session.mapper.run_id);
    assert_eq!(history["usage"]["modelRequests"], 2);
    let tx = db.unchecked_transaction().unwrap();
    let grant = budget::model::original_web_grant_inventory_on(
        &tx,
        &session.lease,
        &session.executor.assignment_id,
    )
    .unwrap();
    assert_eq!(
        history["originalHandoff"]["originalModelTokens"],
        grant.tokens()
    );
    assert_eq!(
        history["originalHandoff"]["originalModelRequests"],
        grant.requests()
    );
    assert_eq!(
        history["originalHandoff"]["workerAttemptId"],
        grant.worker_attempt_id()
    );
    let revision: i64 = tx
        .query_row(
            "SELECT evidence_revision FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history["originalHandoff"]["revision"], revision);
    assert_eq!(
        history["originalHandoff"]["assignmentId"],
        session.executor.assignment_id
    );
    let receipt=crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::read_final_original_on(&tx,&session.lease,&session.mapper).unwrap().unwrap();
    assert_eq!(history["mapperFinalReceiptId"], receipt.receipt_id());
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND acknowledged_at<>'' AND delivery_attempts=1",[receipt.message_id()],|row|row.get::<_,i64>(0)).unwrap(),1);
    tx.commit().unwrap();
    assert_eq!(read(), history);
    // Current actual SDK configuration is now unusable. The history API receives
    // no model configuration and must still return the original signed proof.
    f.context.environment.api_base = "http://127.0.0.1:1/current-config-drift".into();
    f.context.environment.api_key = "current-credential-drift".into();
    assert_eq!(
        read_native_mapper_prepared_history_at(
            &f.context.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.actor.root_run_id
        )
        .unwrap()
        .unwrap(),
        history
    );
    for (scan, attempt, root) in [
        (
            "other-original-scan",
            f.actor.attempt_number,
            f.actor.root_run_id.as_str(),
        ),
        (
            f.actor.scan_id.as_str(),
            f.actor.attempt_number + 1,
            f.actor.root_run_id.as_str(),
        ),
        (
            f.actor.scan_id.as_str(),
            f.actor.attempt_number,
            "other-original-root",
        ),
    ] {
        assert_eq!(
            read_native_mapper_prepared_history_at(&f.context.db_path, scan, attempt, root)
                .unwrap_err(),
            "mapper_prepared_history_original_scope_mismatch"
        );
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), calls);
}

#[test]
fn mapper_loop_history_actual_original_scope_signature_handoff_and_grant_cold_damage_refuse_without_effects(
) {
    let (f, session, seen) = mapper_loop_full_production_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let original = super::tests::application_table_snapshot(&db);
    let calls = seen.lock().unwrap().len();
    for (index,(table,sql,key,expected)) in [
        ("agent_runs","UPDATE agent_runs SET attempt_number=attempt_number+1 WHERE id=?1",f.actor.root_run_id.as_str(),"mapper_prepared_history_original_scope_mismatch"),
        ("agent_mapper_loop_final_receipts","UPDATE agent_mapper_loop_final_receipts SET signature=zeroblob(32) WHERE child_run_id=?1",session.mapper.run_id.as_str(),"mapper_loop_final_signature_invalid"),
        ("agent_messages","UPDATE agent_messages SET payload_json=json_set(payload_json,'$.mapperSummary.summary','cold invalid handoff') WHERE assignment_id=?1 AND kind='execution_assignment'",session.executor.assignment_id.as_str(),"mailbox_assignment_history_original_conflict"),
        ("agent_budget_entries","UPDATE agent_budget_entries SET amount=amount+1 WHERE assignment_id=?1 AND dimension='model_requests' AND kind='reserve'",session.executor.assignment_id.as_str(),"mapper_prepared_history_executor_original_budget_changed"),
    ].into_iter().enumerate() {
        // Each disposable copy starts from the complete original DB, including
        // WAL-committed facts. No attack is introduced into the fixture itself.
        let attack=f.context.db_path.with_file_name(format!("mapper-history-cold-{index}.sqlite3"));
        assert!(!attack.exists());
        db.execute("VACUUM INTO ?1",[attack.to_str().unwrap()]).unwrap();
        let cold=rusqlite::Connection::open(&attack).unwrap();
        let triggers=cold.prepare("SELECT name,sql FROM main.sqlite_master WHERE type='trigger' AND tbl_name=?1 ORDER BY name").unwrap().query_map([table],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let tx=cold.unchecked_transaction().unwrap();
        for (name,_) in &triggers { tx.execute_batch(&format!("DROP TRIGGER \"{}\"",name.replace('"',"\"\""))).unwrap(); }
        assert_eq!(tx.execute(sql,[key]).unwrap(),1,"attack must affect one original source row");
        for (_,sql) in &triggers { tx.execute_batch(sql).unwrap(); }
        tx.commit().unwrap();
        let before=super::tests::application_table_snapshot(&cold);
        assert_eq!(read_native_mapper_prepared_history_at(&attack,&f.actor.scan_id,f.actor.attempt_number,&f.actor.root_run_id).unwrap_err(),expected,"{table}");
        assert_eq!(super::tests::application_table_snapshot(&cold),before,"read must not repair/rebill attacked original");
        assert_eq!(seen.lock().unwrap().len(),calls);
        drop(cold);
        fs::remove_file(attack).unwrap();
    }
    assert_eq!(super::tests::application_table_snapshot(&db), original);
    assert!(read_native_mapper_prepared_history_at(
        &f.context.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        &f.actor.root_run_id
    )
    .unwrap()
    .is_some());
}
