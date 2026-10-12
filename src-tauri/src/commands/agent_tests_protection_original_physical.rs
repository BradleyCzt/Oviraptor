// Cold-storage corruption only in a disposable actual-transport fixture.
// Restore every original table trigger before audit; never execute signed SQL,
// sign reconstructed bytes, or use a missing guard as the rejection evidence.
fn protection_physical_triggers(db: &rusqlite::Connection, table: &str) -> Vec<(String, String)> {
    db.prepare(
        "SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name=?1 ORDER BY name",
    )
    .unwrap()
    .query_map([table], |r| Ok((r.get(0)?, r.get(1)?)))
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

fn protection_cold_physical_case(
    f: &ProtectionOriginalFixture,
    db: &rusqlite::Connection,
    label: &str,
    table: &str,
    damage: impl FnOnce(&rusqlite::Connection) -> usize,
) {
    let before = super::tests::application_table_snapshot(db);
    let tx = db.unchecked_transaction().unwrap();
    let triggers = protection_physical_triggers(&tx, table);
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
    }
    assert_eq!(
        damage(&tx),
        1,
        "{label}: damage exactly the original scoped row"
    );
    for (_, ddl) in &triggers {
        tx.execute_batch(ddl).unwrap();
    }
    assert_eq!(
        protection_physical_triggers(&tx, table),
        triggers,
        "{label}: production trigger DDL must be restored before verification"
    );
    f.assert_original_read_only(&tx, false);
    tx.rollback().unwrap();
    assert_eq!(
        super::tests::application_table_snapshot(db),
        before,
        "{label}: original fixture data restored by rollback"
    );
    f.assert_original_read_only(db, true);
}

#[test]
fn protection_original_actual_receipt_cold_physical_matrix_rejects_without_writes() {
    let f = ProtectionOriginalFixture::new("protection-original-physical");
    let db = db::open(&f.h.db_path).unwrap();
    f.assert_original_read_only(&db, true);
    let fact = f.fact(&db);
    let receipt = f.receipt.as_str();
    let root = f.actor.root_run_id.as_str();
    let run = fact["runId"].as_str().unwrap();
    let assignment = fact["assignmentId"].as_str().unwrap();
    let worker = fact["workerAttemptId"].as_str().unwrap();
    let claim_rowid = fact["claimRowid"].as_i64().unwrap();
    let claim = &fact["preSendAuthorization"]["claim"];
    let budget = claim["budgetAttempt"].as_i64().unwrap();
    let source = format!(
        "http:{run}:{}:{}:{}",
        claim["invocationId"].as_str().unwrap(),
        claim["requestIndex"].as_i64().unwrap(),
        claim["requestHash"].as_str().unwrap()
    );

    protection_cold_physical_case(
        &f,
        &db,
        "signature",
        "agent_protection_response_receipts",
        |tx| {
            tx.execute("UPDATE agent_protection_response_receipts SET signature=zeroblob(32) WHERE receipt_id=?1",
            [receipt]).unwrap()
        },
    );
    protection_cold_physical_case(
        &f,
        &db,
        "noncanonical public SHA",
        "agent_protection_response_receipts",
        |tx| {
            let raw = format!("{} ", fact);
            tx.execute("UPDATE agent_protection_response_receipts SET fact_json=?1,fact_hash=?2 WHERE receipt_id=?3",
            params![raw,crate::agent_runtime::store::stable_hash(&raw),receipt]).unwrap()
        },
    );
    protection_cold_physical_case(
        &f,
        &db,
        "canonical forged body SHA",
        "agent_protection_response_receipts",
        |tx| {
            let mut changed = fact.clone();
            let old = changed["firstWriter"]["bodySha256"].as_str().unwrap();
            let forged = "a".repeat(64);
            let replacement = if old == forged.as_str() {
                "b".repeat(64)
            } else {
                "a".repeat(64)
            };
            changed["firstWriter"]["bodySha256"] = json!(replacement);
            let raw = changed.to_string();
            // Recomputed public hash cannot replace the original private signature.
            tx.execute("UPDATE agent_protection_response_receipts SET fact_json=?1,fact_hash=?2 WHERE receipt_id=?3",
            params![raw,crate::agent_runtime::store::stable_hash(&raw),receipt]).unwrap()
        },
    );
    protection_cold_physical_case(
        &f,
        &db,
        "receipt physical storage",
        "agent_protection_response_receipts",
        |tx| {
            tx.execute("UPDATE agent_protection_response_receipts SET storage_id=storage_id+1000 WHERE receipt_id=?1",
            [receipt]).unwrap()
        },
    );

    for (label, sql) in [
        ("received status", "UPDATE agent_http_request_claims SET response_status=401 WHERE rowid=?1"),
        ("received timestamp", "UPDATE agent_http_request_claims SET received_at='2000-01-01 00:00:00' WHERE rowid=?1"),
        ("missing received proof", "UPDATE agent_http_request_claims SET received_at='' WHERE rowid=?1"),
        ("original claim identity", "UPDATE agent_http_request_claims SET identity_handle='cold-foreign-identity' WHERE rowid=?1"),
    ] {
        protection_cold_physical_case(&f,&db,label,"agent_http_request_claims",|tx|
            tx.execute(sql,[claim_rowid]).unwrap());
    }
    protection_cold_physical_case(
        &f,
        &db,
        "origin baseline",
        "agent_http_budget_origins",
        |tx| {
            tx.execute("UPDATE agent_http_budget_origins SET baseline_requests=baseline_requests+1 WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=?3",
            params![f.actor.scan_id,f.actor.target_key,budget]).unwrap()
        },
    );

    for kind in ["reserve", "forfeit", "reconcile"] {
        let id:String = db.query_row(
            "SELECT entry_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 AND kind=?5",
            params![root,assignment,worker,source,kind],|r|r.get(0)).unwrap();
        let count:i64 = db.query_row(
            "SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 AND kind=?5",
            params![root,assignment,worker,source,kind],|r|r.get(0)).unwrap();
        assert_eq!(count, 1, "actual original {kind} receipt row");
        protection_cold_physical_case(
            &f,
            &db,
            &format!("{kind} physical timestamp"),
            "agent_budget_entries",
            |tx| {
                tx.execute("UPDATE agent_budget_entries SET created_at='2000-01-01 00:00:00' WHERE entry_id=?1",
                [&id]).unwrap()
            },
        );
    }
    let cap:String = db.query_row(
        "SELECT id FROM agent_capability_leases WHERE root_run_id=?1 AND assignment_id=?2 AND child_run_id=?3 AND capability='replay_http'",
        params![root,assignment,run],|r|r.get(0)).unwrap();
    protection_cold_physical_case(
        &f,
        &db,
        "capability stable birth",
        "agent_capability_leases",
        |tx| {
            tx.execute(
                "UPDATE agent_capability_leases SET created_at='2000-01-01 00:00:00' WHERE id=?1",
                [&cap],
            )
            .unwrap()
        },
    );
    protection_cold_physical_case(&f, &db, "original frozen task", "agent_assignments", |tx| {
        tx.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.coldDrift',true) WHERE id=?1",
            [assignment]).unwrap()
    });
    for (label,sql) in [
        ("worker immutable birth", "UPDATE agent_assignment_attempts SET leased_at='2000-01-01 00:00:00' WHERE id=?1"),
        ("worker original C epoch", "UPDATE agent_assignment_attempts SET coordinator_epoch=coordinator_epoch+1 WHERE id=?1"),
        ("worker original C fence", "UPDATE agent_assignment_attempts SET coordinator_fencing_token=coordinator_fencing_token||':cold' WHERE id=?1"),
    ] {
        protection_cold_physical_case(&f,&db,label,"agent_assignment_attempts",|tx|
            tx.execute(sql,[worker]).unwrap());
    }
    protection_cold_physical_case(&f, &db, "child frozen plan hash", "agent_runs", |tx| {
        tx.execute(
            "UPDATE agent_runs SET plan_hash=plan_hash||':cold' WHERE id=?1",
            [run],
        )
        .unwrap()
    });

    // Historical source needs no renewed/live C, but must bind its exact actor.
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let get = f.h.site_seen.lock().unwrap().len();
    for change_epoch in [true, false] {
        let mut wrong = f.actor.clone();
        if change_epoch {
            wrong.lease_epoch += 1;
        } else {
            wrong.fencing_token.push_str(":cold");
        }
        let tx = db.unchecked_transaction().unwrap();
        let changes = tx.total_changes();
        assert!(native_original_protection_response_on(&tx, &wrong, receipt).is_err());
        assert_eq!(super::tests::application_table_snapshot(&tx), before);
        assert_eq!(tx.total_changes(), changes);
        tx.commit().unwrap();
    }
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), get);
    assert_eq!(get, 1);
}
