// Original late invoices: private DB, real child dispatch
// and unknown-cost production functions; Root response is typed financial data,
// not a claim that the Multi Root SDK production route already exists.
#[test]
fn root_budget_known_late_unlimited_topup_does_not_lose_invoice_to_other_child_unknown_cost() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        model::gateway::ModelResponse,
        multi_agent::{
            attempts, budget, budget::root::model::RootModelCall, scheduler, specialist,
        },
        store::UsageDelta,
    };
    let (directory, path, root, lease) =
        multi_agent_test_root("root-late-topup-after-child-unknown", 0, 10);
    let db = db::open(&path).unwrap();
    root_budget_multi_fixture_owner_for_test(&db,&root);
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "original-active-child",
        &json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    let specialist::Start::Dispatch(child_call) = specialist::start_authorized(
        &db,
        &lease,
        &child,
        &json!({"messages":[{"role":"user","content":"original child request"}]}),
        |connection| attempts::require_live_for_run(connection, &child.run_id).map(|_| ()),
    )
    .unwrap() else {
        panic!("the original child must have its real durable dispatch before Root claim")
    };
    // The explicit fixture issued the original financial owner before child
    // scheduling/dispatch. The claim pure-loads that same immutable owner;
    // current usage or an active child cannot issue Root financial authority.
    let root_call = {
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 10).unwrap();
        tx.commit().unwrap();
        call
    };
    specialist::record_uncertain(&db, &child_call, "provider_outcome_unknown").unwrap();
    for dimension in budget::DIMENSIONS.iter().take(4) {
        let expected = if *dimension == "model_requests" {
            1
        } else {
            10
        };
        let b = budget::balance(&db, &root, Some(&child.assignment_id), dimension).unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, expected));
    }
    assert!(budget::admission::require_determinate(&db, &root).is_err());
    let before = super::tests::application_table_snapshot(&db);
    let invoice = ModelResponse {
        usage: UsageDelta {
            input_tokens: 20,
            cached_input_tokens: 5,
            output_tokens: 30,
            total_tokens: 50,
            model_requests: 1,
        },
        usage_reported: true,
        ..ModelResponse::default()
    };
    {
        let tx = db.unchecked_transaction().unwrap();
        let unresolved = root_call
            .terminal(&tx, "received", Some(&invoice), "")
            .expect("the already incurred known original invoice must survive other unknown debt");
        assert!(
            !unresolved,
            "this Root bill is reported and token budget is unlimited"
        );
        tx.commit().unwrap();
    }
    for (dimension, amount) in [
        ("model_input_tokens", 20),
        ("model_cached_tokens", 5),
        ("model_output_tokens", 30),
        ("model_requests", 1),
    ] {
        let b = budget::balance(&db, &root, Some(""), dimension).unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, amount, 0));
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received' \
             AND json_extract(receipt_json,'$.usageReported')=1 \
             AND json_extract(receipt_json,'$.usage.totalTokens')=50",
            [&root],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    let received: i64 = db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&root], |r|r.get(0)).unwrap();
    let exact_cache: i64 = db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance p JOIN agent_root_model_journal j
     ON p.call_id=j.call_id AND p.root_run_id=j.root_run_id AND p.lease_attempt_id=j.lease_attempt_id
     AND p.round=j.round AND p.request_hash=j.request_hash AND p.response_hash=json_extract(j.receipt_json,'$.responseHash')
     WHERE j.root_run_id=?1 AND j.phase='received' AND p.cached_usage_reported=0", [&root], |r|r.get(0)).unwrap();
    assert_eq!(exact_cache, received);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance", [], |r|r.get::<_,i64>(0)).unwrap(), received);
    let after = super::tests::application_table_snapshot(&db);
    for (table, rows) in &before {
        if !matches!(
            table.as_str(),
            "agent_budget_entries" | "agent_root_model_journal" | "agent_root_model_cache_provenance"
        ) {
            assert_eq!(
                after.iter().find(|(name, _)| name == table).map(|(_, values)| values),
                Some(rows),
                "original child/control states, projection, business and Native JSON cannot change: {table}"
            );
        }
    }
    for dimension in budget::DIMENSIONS.iter().take(4) {
        let expected = if *dimension == "model_requests" {
            1
        } else {
            10
        };
        let b = budget::balance(&db, &root, Some(&child.assignment_id), dimension).unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, expected));
    }
    // Saving this invoice neither clears another worker's unknown cost nor
    // returns permission to use the Root result or issue any fresh allocation.
    assert!(root_call.require_next_work(&db).is_err());
    assert!(budget::admission::require_determinate(&db, &root).is_err());
    let settled = super::tests::application_table_snapshot(&db);
    {
        let tx = db.unchecked_transaction().unwrap();
        assert!(RootModelCall::claim(&tx, &root, 2, &"b".repeat(64), 1).is_err());
        tx.rollback().unwrap();
    }
    {
        let tx = db.unchecked_transaction().unwrap();
        assert!(budget::append(
            &tx,
            &lease,
            &child.assignment_id,
            "model_input_tokens",
            budget::Kind::Reserve,
            1,
            "fresh-allocation-after-unknown",
            "fresh-work",
        )
        .is_err());
        tx.rollback().unwrap();
    }
    assert!(super::tests::application_table_snapshot(&db) == settled);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn root_budget_original_child_unlimited_invoice_survives_root_unknown_cost() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, budget::root::model::RootModelCall, scheduler, specialist},
    };
    let (directory, path, root, lease) =
        multi_agent_test_root("child-late-topup-root-unknown", 0, 10);
    let db = db::open(&path).unwrap();
    root_budget_multi_fixture_owner_for_test(&db,&root);
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "original-child-invoice",
        &json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    let specialist::Start::Dispatch(child_call) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!("original child dispatch required")
    };
    let tx = db.unchecked_transaction().unwrap();
    let root_call = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 10).unwrap();
    tx.commit().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(root_call
        .terminal(&tx, "uncertain", None, "original-root-outcome-unknown")
        .unwrap());
    tx.commit().unwrap();
    let root_debt = budget::DIMENSIONS[..4]
        .iter()
        .map(|d| budget::balance(&db, &root, Some(""), d).unwrap())
        .collect::<Vec<_>>();
    let usage = AgentTokenUsage {
        input_tokens: 20,
        cached_input_tokens: 5,
        output_tokens: 30,
        total_tokens: 50,
        model_requests: 1,
    };
    specialist::record_received(&db, &child_call, "known original child bill", false, &usage)
        .expect(
            "already incurred known child cost must not be lost to another owner's unknown bill",
        );
    for (dimension, amount) in budget::DIMENSIONS[..4].iter().zip([20, 5, 30, 1]) {
        let b = budget::balance(&db, &root, Some(&child.assignment_id), dimension).unwrap();
        assert_eq!((b.consumed, b.indeterminate), (amount, 0), "{dimension}");
    }
    assert_eq!(
        budget::DIMENSIONS[..4]
            .iter()
            .map(|d| budget::balance(&db, &root, Some(""), d).unwrap())
            .collect::<Vec<_>>(),
        root_debt
    );
    assert!(budget::admission::require_determinate(&db, &root).is_err());
    assert!(root_call.require_next_work(&db).is_err());
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(budget::append(
        &tx,
        &lease,
        &child.assignment_id,
        "model_input_tokens",
        budget::Kind::Reserve,
        1,
        "fresh-child-work",
        "fresh-work"
    )
    .is_err());
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
