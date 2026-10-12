// Production startup admission tests. Bills below use the explicit lower-level
// financial fixture; these prove zero-write refusal, not an SDK recovery loop.
fn web_resume_finance_fixture() -> (WebModeFixture, rusqlite::Connection, String) {
    let fixture = web_mode_fixture("multi", "https://finance.example.test/app");
    let context =
        crate::commands::agent_tests::test_context(&fixture.path, &fixture.target, vec![]);
    persist_frozen_web_execution_plan(
        &fixture.path,
        &fixture.scan,
        1,
        &fixture.target,
        &context.execution_plan,
    )
    .unwrap();
    let db = db::open(&fixture.path).unwrap();
    let root = db
        .query_row(
            "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
            [&fixture.scan],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    (fixture, db, root)
}

fn web_resume_finance_call(db: &rusqlite::Connection, root: &str, terminal: Option<&str>) {
    use crate::agent_runtime::{
        model::gateway::ModelResponse, multi_agent::budget::root::model::RootModelCall,
    };
    let tx = db.unchecked_transaction().unwrap();
    let call = RootModelCall::claim(&tx, root, 1, &"a".repeat(64), 1_000).unwrap();
    tx.commit().unwrap();
    if let Some(terminal) = terminal {
        let response = ModelResponse {
            text: "bounded financial fixture".into(),
            tool_calls: vec![],
            usage: AgentTokenUsage {
                input_tokens: 12,
                cached_input_tokens: 2,
                output_tokens: 8,
                total_tokens: 20,
                model_requests: 1,
            },
            usage_reported: true,
            cached_usage_reported: false,
            finish_reason: "stop".into(),
        };
        let tx = db.unchecked_transaction().unwrap();
        call.terminal(
            &tx,
            terminal,
            (terminal == "received").then_some(&response),
            "fixture_unknown",
        )
        .unwrap();
        tx.commit().unwrap();
    }
}

fn web_resume_finance_refuses_unchanged(
    db: &rusqlite::Connection,
    fixture: &WebModeFixture,
    mode: WebStartMode,
) -> String {
    let before = web_mode_test_rows(db);
    let changes = db.total_changes();
    let task = fs::read(fixture.work.join("task.json")).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let error = prepare_web_startup_in(&tx, &fixture.root, &fixture.scan, mode)
        .err()
        .expect("financial history must not issue another attempt");
    drop(tx);
    web_mode_assert_rows(db, &before);
    assert_eq!(
        db.total_changes(),
        changes,
        "refusal must precede every write"
    );
    assert_eq!(fs::read(fixture.work.join("task.json")).unwrap(), task);
    assert!(!fixture.work.parent().unwrap().join("attempt-0002").exists());
    error
}

#[test]
fn web_resume_finance_unpaid_root_can_continue_without_inventing_any_charge() {
    let (fixture, mut db, root) = web_resume_finance_fixture();
    let original_owner: String = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&root],
            |row| row.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
        [&fixture.scan],
    )
    .unwrap();
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let startup =
        prepare_web_startup_in(&tx, &fixture.root, &fixture.scan, WebStartMode::Resume).unwrap();
    assert_eq!(startup.attempt, 2);
    assert_eq!(
        tx.query_row("SELECT COUNT(*) FROM agent_budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original_owner
    );
    drop(startup);
    drop(tx);
    assert!(!fixture.work.parent().unwrap().join("attempt-0002").exists());
}

#[test]
fn web_resume_finance_dispatch_and_unknown_cost_refuse_resume_retry_and_confirm_zero_write() {
    for terminal in [None, Some("uncertain")] {
        let (fixture, db, root) = web_resume_finance_fixture();
        web_resume_finance_call(&db, &root, terminal);
        for (mode, status) in [
            (WebStartMode::Resume, "paused"),
            (WebStartMode::Retry, "completed"),
            (WebStartMode::Confirm, "draft"),
        ] {
            db.execute(
                "UPDATE sentinel_scans SET status=?1 WHERE id=?2",
                params![status, fixture.scan],
            )
            .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    web_resume_finance_refuses_unchanged(&db, &fixture, mode),
                    "budget_indeterminate_requires_reconciliation"
                );
            }
        }
    }
}

#[test]
fn web_resume_finance_settled_bill_still_needs_explicit_remaining_budget_contract() {
    let (fixture, db, root) = web_resume_finance_fixture();
    web_resume_finance_call(&db, &root, Some("received"));
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(&db, &root).unwrap();
    for (mode, status) in [
        (WebStartMode::Resume, "paused"),
        (WebStartMode::Retry, "completed"),
    ] {
        db.execute(
            "UPDATE sentinel_scans SET status=?1 WHERE id=?2",
            params![status, fixture.scan],
        )
        .unwrap();
        assert_eq!(
            web_resume_finance_refuses_unchanged(&db, &fixture, mode),
            "web_start_paid_history_requires_explicit_financial_contract"
        );
    }
}

#[test]
fn web_resume_finance_checks_all_prior_roots_not_only_current_attempt() {
    let (fixture, db, root) = web_resume_finance_fixture();
    web_resume_finance_call(&db, &root, Some("uncertain"));
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=7,status='paused' WHERE id=?1",
        [&fixture.scan],
    )
    .unwrap();
    let before = web_mode_test_rows(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        require_web_start_financial_history_in(&tx, &fixture.scan).unwrap_err(),
        "budget_indeterminate_requires_reconciliation"
    );
    drop(tx);
    web_mode_assert_rows(&db, &before);
}

#[test]
fn web_resume_finance_missing_original_owner_or_root_scope_cannot_hide_bill() {
    for fault in ["owner", "bad_owner", "root"] {
        let (fixture, db, root) = web_resume_finance_fixture();
        web_resume_finance_call(&db, &root, Some("received"));
        if fault == "owner" {
            // Corrupt only a disposable fixture; production immutable owner stays protected.
            db.execute_batch("DROP TRIGGER root_budget_attempt_no_delete")
                .unwrap();
            db.pragma_update(None, "foreign_keys", false).unwrap();
            db.execute(
                "DELETE FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [&root],
            )
            .unwrap();
            db.pragma_update(None, "foreign_keys", true).unwrap();
        } else if fault == "bad_owner" {
            db.execute_batch("DROP TRIGGER root_budget_attempt_no_update")
                .unwrap();
            db.execute(
                "UPDATE agent_root_budget_attempts SET contract_json='{}' WHERE root_run_id=?1",
                [&root],
            )
            .unwrap();
        } else {
            // Move only the disposable Root to a real foreign scan so the
            // fixture reaches the durable-owner scope gate without an FK error.
            let foreign = web_mode_test_draft(&db, Some("multi"), &fixture.target).unwrap();
            db.execute(
                "UPDATE agent_runs SET scan_id=?1 WHERE id=?2",
                params![foreign.id, root],
            )
            .unwrap();
        }
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&fixture.scan],
        )
        .unwrap();
        let error = web_resume_finance_refuses_unchanged(&db, &fixture, WebStartMode::Resume);
        assert_eq!(
            error,
            match fault {
                "owner" => "budget_history_requires_reconciliation",
                "bad_owner" => "budget_root_original_owner_conflict",
                _ => "web_start_original_finance_scope_missing",
            }
        );
    }
}

#[test]
fn web_resume_finance_is_transaction_bound_and_independent_tasks_do_not_inherit_old_bill() {
    let (fixture, mut db, root) = web_resume_finance_fixture();
    web_resume_finance_call(&db, &root, Some("uncertain"));
    let before = web_mode_test_rows(&db);
    assert_eq!(
        require_web_start_financial_history_in(&db, &fixture.scan).unwrap_err(),
        "web_start_finance_requires_transaction"
    );
    web_mode_assert_rows(&db, &before);
    let other = web_mode_test_draft(&db, Some("multi"), &fixture.target).unwrap();
    web_mode_test_start(&fixture.root, &mut db, &other.id, WebStartMode::Confirm).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&other.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_root_model_journal WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
