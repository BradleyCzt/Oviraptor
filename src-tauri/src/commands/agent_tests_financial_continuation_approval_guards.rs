// Issuance guards use the real paid caller, production pause and immutable
// confirmation fixture. Every corruption is confined to its disposable DB.
// Untouched original rows/JSON are captured BEFORE fault injection; a second
// snapshot only proves that the approval adds no writes to that faulted DB.
struct FinancialGuardOriginals {
    rows: Vec<NativeCoordinatorFrozenRows>,
    json: Vec<(PathBuf, Vec<u8>)>,
}

fn financial_guard_originals(
    f: &RootTickFixture,
    db: &rusqlite::Connection,
) -> FinancialGuardOriginals {
    let root = f.actor.root_run_id.as_str();
    let mut originals = Vec::new();
    for table in [
        "agent_runs",
        "agent_root_budget_attempts",
        "agent_root_tick_receipts",
        "agent_root_model_journal",
        "agent_root_tick_terminal_proofs",
        "agent_root_cost_reconciliation_receipts",
        "agent_root_mode_definitions",
        "agent_root_budget_definitions",
    ] {
        let column = if table == "agent_runs" {
            "id"
        } else {
            "root_run_id"
        };
        originals.push(
            NativeCoordinatorFrozenRows::capture_optional(
                db,
                &format!("SELECT rowid,* FROM {table} WHERE {column}=?1 ORDER BY rowid"),
                &[root],
            )
            .unwrap(),
        );
    }
    let floor: i64 = db
        .query_row(
            "SELECT coalesce(max(rowid),0) FROM agent_budget_entries",
            [],
            |r| r.get(0),
        )
        .unwrap();
    originals.push(
        NativeCoordinatorFrozenRows::capture_optional(
            db,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1
             AND rowid<=CAST(?2 AS INTEGER) ORDER BY rowid",
            &[root, &floor.to_string()],
        )
        .unwrap(),
    );
    originals.push(
        NativeCoordinatorFrozenRows::capture_optional(
            db,
            "SELECT rowid,* FROM native_web_mode_receipts WHERE scan_id=?1
             AND attempt_number=1 ORDER BY rowid",
            &[&f.context.scan_id],
        )
        .unwrap(),
    );
    let json = [
        f.context.target_dir.join("frontend-evidence.json"),
        f.f.work.join("task.json"),
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect();
    FinancialGuardOriginals {
        rows: originals,
        json,
    }
}

fn financial_guard_verify_originals(
    f: &RootTickFixture,
    db: &rusqlite::Connection,
    seen: &FinancePreviewSeen,
    originals: &FinancialGuardOriginals,
) {
    for rows in &originals.rows {
        rows.verify(db)
            .expect("untouched original paid rows changed");
    }
    for (path, bytes) in &originals.json {
        assert_eq!(&fs::read(path).unwrap(), bytes, "{}", path.display());
    }
    assert_eq!(seen.lock().unwrap().len(), 1, "no additional SDK request");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_financial_continuations WHERE source_root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number>1",
            [&f.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number>1",
            [&f.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(!f.f.work.parent().unwrap().join("attempt-0002").exists());
}

fn financial_guard_approval_error(
    f: &RootTickFixture,
    db: &rusqlite::Connection,
    receipt: &str,
    proposal: &str,
) -> String {
    // This is the faulted input to the operation, not the original source.
    // Original preservation is checked independently by the helper above.
    let rows = web_mode_test_rows(db);
    let physical = single_finally_physical(db);
    let error = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        receipt,
        proposal,
    )
    .unwrap_err();
    web_mode_assert_rows(db, &rows);
    assert_eq!(single_finally_physical(db), physical);
    error
}

#[test]
fn financial_continuation_approval_typed_capability_rejects_cross_scope() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-cross-scope");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    let before = single_finally_physical(&db);
    for (scan, attempt, root) in [
        ("other-scan", 1, f.actor.root_run_id.as_str()),
        (f.context.scan_id.as_str(), 2, f.actor.root_run_id.as_str()),
        (f.context.scan_id.as_str(), 1, "other-root"),
    ] {
        let snapshot = db.unchecked_transaction().unwrap();
        let error = FinancialContinuationAuthorization::approve_in(
            &snapshot,
            scan,
            attempt,
            root,
            &receipt,
            preview["proposalHash"].as_str().unwrap(),
        )
        .err()
        .expect("cross-scope JSON cannot mint a typed approval");
        assert_eq!(error, "financial_continuation_current_scope_invalid");
        drop(snapshot);
        assert_eq!(single_finally_physical(&db), before);
    }
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_rejects_unrelated_confirmation_receipt() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-wrong-receipt");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &"0".repeat(64),
            preview["proposalHash"].as_str().unwrap(),
        ),
        "continuation_original_receipt_required"
    );
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_rechecks_original_policy_even_with_fresh_preview() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-policy-revoked");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    assert_eq!(db.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.instruction','authorization withdrawn')
        WHERE scan_id=?1",[&f.context.scan_id]).unwrap(),1);
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &receipt,
            preview["proposalHash"].as_str().unwrap()
        ),
        "financial_continuation_proposal_changed"
    );
    // A newly displayed read-only hash is still not original authorization.
    let changed = finance_preview_read(&f, &db, &receipt).unwrap();
    assert_ne!(changed["proposalHash"], preview["proposalHash"]);
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &receipt,
            changed["proposalHash"].as_str().unwrap()
        ),
        "financial_continuation_original_policy_changed"
    );
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_rejects_replaced_task_identity_before_issuance() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-task-identity");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    // The original task was anonymous. A later reference cannot replace that
    // task's authorization with an absent/other-project captured identity.
    assert_eq!(
        db.execute(
            "UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,
        '$.authSessionIds',json('[\"missing-task-identity\"]')) WHERE scan_id=?1",
            [&f.context.scan_id]
        )
        .unwrap(),
        1
    );
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &receipt,
            preview["proposalHash"].as_str().unwrap()
        ),
        "任务引用了不存在或其他工作空间的登录会话"
    );
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_does_not_renew_expired_original_coordinator() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-coordinator-expired");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    assert_eq!(
        db.execute(
            "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'
        WHERE root_run_id=?1",
            [&f.actor.root_run_id]
        )
        .unwrap(),
        1
    );
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &receipt,
            preview["proposalHash"].as_str().unwrap()
        ),
        "stale_coordinator_fencing_token"
    );
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_rejects_mismatched_original_budget_limits() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-limit-mismatch");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    // Corrupt one physical class cap, keeping the signed original plan/owner
    // unchanged. This is not a fixture pretending to exhaust a valid grant.
    db.execute_batch("DROP TRIGGER budget_limit_no_update")
        .unwrap();
    assert_eq!(
        db.execute(
            "UPDATE agent_budget_limits SET hard_limit=hard_limit-1
        WHERE root_run_id=?1 AND dimension='model_input_tokens'",
            [&f.actor.root_run_id]
        )
        .unwrap(),
        1
    );
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &receipt,
            preview["proposalHash"].as_str().unwrap()
        ),
        // Quiescence audits owner/limits before the proposal and wraps the
        // frozen-limit mismatch as an unverifiable original scope.
        "scan_quiescence_original_scope_unverifiable"
    );
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_retains_unknown_original_obligation() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-unknown-obligation");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    // A legal reserve→forfeit transition gives an actual unknown balance;
    // inserting forfeit alone would merely test a corrupt negative reserve.
    for (kind, key) in [
        ("reserve", "approval-original-unknown-reserve"),
        ("forfeit", "approval-original-unknown-forfeit"),
    ] {
        assert_eq!(db.execute("INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,
            dimension,kind,amount,idempotency_key,source_id)
            SELECT ?2,root_run_id,'',id,'model_requests',?3,1,?2,'original-obligation-without-receipt'
            FROM agent_root_budget_attempts WHERE root_run_id=?1",
            params![f.actor.root_run_id,key,kind]).unwrap(),1);
    }
    let usage = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (usage.reserved, usage.consumed, usage.indeterminate),
        (0, 1, 1)
    );
    assert_eq!(
        financial_guard_approval_error(
            &f,
            &db,
            &receipt,
            preview["proposalHash"].as_str().unwrap()
        ),
        // Financial admission rejects this legal unknown balance before
        // continuation's more specific remaining-balance check.
        "budget_indeterminate_requires_reconciliation"
    );
    financial_guard_verify_originals(&f, &db, &seen, &original);
}

#[test]
fn financial_continuation_approval_birth_writer_fault_rolls_back_entire_issue() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("approval-birth-writer-fault");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let original = financial_guard_originals(&f, &db);
    db.execute_batch(
        "CREATE TRIGGER approval_birth_fault BEFORE INSERT ON agent_budget_limits
        BEGIN SELECT RAISE(ABORT,'approval_birth_writer_fault'); END;",
    )
    .unwrap();
    let error = financial_guard_approval_error(
        &f,
        &db,
        &receipt,
        preview["proposalHash"].as_str().unwrap(),
    );
    assert!(error.contains("approval_birth_writer_fault"), "{error}");
    financial_guard_verify_originals(&f, &db, &seen, &original);
}
