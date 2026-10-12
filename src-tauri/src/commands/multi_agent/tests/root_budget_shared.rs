// Explicit legacy accounting prototype, issued before any child/model work.
// This proves original financial algebra, not fresh production creation.
fn root_budget_multi_fixture_owner_for_test(db:&rusqlite::Connection,root:&str) {
    let tx=db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(&tx,root).unwrap();
    tx.commit().unwrap();
}

#[test]
fn root_budget_paid_root_cost_cannot_be_borrowed_by_a_child_grant() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        model::gateway::ModelResponse,
        multi_agent::{budget::root::model::RootModelCall, scheduler},
        store::UsageDelta,
    };
    let (directory, path, root, lease) = multi_agent_test_root("root-shared-quota", 1000, 10);
    let db = db::open(&path).unwrap();
    root_budget_multi_fixture_owner_for_test(&db,&root);
    let call = {
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 1000).unwrap();
        tx.commit().unwrap();
        call
    };
    let response = ModelResponse {
        usage: UsageDelta {
            input_tokens: 100,
            output_tokens: 100,
            total_tokens: 200,
            model_requests: 1,
            ..UsageDelta::default()
        },
        usage_reported: true,
        ..ModelResponse::default()
    };
    {
        let tx = db.unchecked_transaction().unwrap();
        assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
        tx.commit().unwrap();
    }
    let before = super::tests::application_table_snapshot(&db);
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "root-paid-already",
        &json!({}),
        1,
        &["evidence.read".into()],
        900,
        1,
    );
    assert!(
        child.is_err(),
        "200 already paid Root tokens plus a 900 child grant exceed the original 1000"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn root_budget_child_token_classes_do_not_double_count_one_grant() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget::root::model::RootModelCall, scheduler},
    };
    let (directory, path, root, lease) = multi_agent_test_root("root-child-first", 1000, 10);
    let db = db::open(&path).unwrap();
    root_budget_multi_fixture_owner_for_test(&db,&root);
    scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "one-child-grant",
        &json!({}),
        1,
        &["evidence.read".into()],
        600,
        1,
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let claim = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 100);
    assert!(
        claim.is_ok(),
        "one 600-token child grant leaves room for a 100-token Root call: {}",
        claim.err().unwrap_or_default()
    );
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        600
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn root_budget_corrupt_coarse_total_cannot_expand_original_frozen_root_cap() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget::root::model::RootModelCall, scheduler},
    };
    let (directory, path, root, lease) = multi_agent_test_root("root-coarse-total-fault", 1000, 10);
    let db = db::open(&path).unwrap();
    root_budget_multi_fixture_owner_for_test(&db,&root);
    let tx = db.unchecked_transaction().unwrap();
    let call = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 1000).unwrap();
    tx.commit().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let mut response = late_web_model_response();
    response.usage.input_tokens = 100;
    response.usage.output_tokens = 100;
    response.usage.total_tokens = 200;
    assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
    tx.commit().unwrap();
    db.execute("INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) VALUES(?1,2000,10,?2,?3)", params![root,lease.lease_epoch,lease.fencing_token]).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            "cannot-expand-root",
            &json!({}),
            1,
            &["evidence.read".into()],
            900,
            1
        )
        .is_err(),
        "coarse 2000 must not override original Root 1000"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn root_budget_late_original_multi_invoice_does_not_adopt_replacement_coordinator() {
    use crate::agent_runtime::multi_agent::budget::{self, root::model::RootModelCall};
    for different_root in [false, true] {
        let (directory, path, root, lease) = multi_agent_test_root("root-late-control", 1000, 10);
        let db = db::open(&path).unwrap();
    root_budget_multi_fixture_owner_for_test(&db,&root);
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 1000).unwrap();
        tx.commit().unwrap();
        let replacement = replace_target_cost_coordinator(&db, &lease, different_root);
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(!call
            .terminal(&tx, "received", Some(&late_web_model_response()), "")
            .unwrap());
        tx.commit().unwrap();
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
                "agent_root_model_journal" | "agent_budget_entries" | "agent_root_model_cache_provenance"
            ) {
                assert_eq!(
                    after
                        .iter()
                        .find(|(name, _)| name == table)
                        .map(|(_, values)| values),
                    Some(rows),
                    "{table}"
                );
            }
        }
        assert_eq!(
            budget::balance(&db, &root, Some(""), "model_requests")
                .unwrap()
                .consumed,
            1
        );
        if different_root {
            assert_eq!(
                budget::balance(&db, &replacement.root_run_id, None, "model_requests").unwrap(),
                budget::Balance::default()
            );
        }
        assert!(call.require_next_work(&db).is_err());
        let saved = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(RootModelCall::claim(&tx, &root, 2, &"b".repeat(64), 100).is_err());
        tx.rollback().unwrap();
        assert!(super::tests::application_table_snapshot(&db) == saved);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}
