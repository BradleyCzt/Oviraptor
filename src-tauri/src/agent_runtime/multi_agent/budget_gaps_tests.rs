use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE agent_runs (
            id TEXT PRIMARY KEY,scan_id TEXT,attempt_number INTEGER,target_url TEXT,root_run_id TEXT
         );
         CREATE TABLE agent_budget_ledger (
            root_run_id TEXT PRIMARY KEY,reserved_tokens INTEGER,spent_tokens INTEGER,
            reserved_requests INTEGER,spent_requests INTEGER,lease_epoch INTEGER,fencing_token TEXT
         );
         CREATE TABLE agent_budget_limits(root_run_id TEXT,dimension TEXT,hard_limit INTEGER);
         CREATE TABLE agent_budget_entries(root_run_id TEXT,assignment_id TEXT,dimension TEXT,kind TEXT,amount INTEGER);
         CREATE TABLE agent_assignments (
            id TEXT PRIMARY KEY,coordinator_run_id TEXT,reserved_tokens INTEGER,
            reserved_requests INTEGER,budget_settled_at TEXT,
            scheduling_model TEXT NOT NULL DEFAULT 'legacy_role_pipeline'
         );
         CREATE TABLE tool_invocations (
            id INTEGER PRIMARY KEY,run_id TEXT,contract_key TEXT,finished_at TEXT
         );
         CREATE TABLE agent_coordinator_leases (
            scan_id TEXT,attempt_number INTEGER,target_key TEXT,root_run_id TEXT,
            lease_epoch INTEGER,fencing_token TEXT
         );
         INSERT INTO agent_runs VALUES('root','scan',1,'https://example.test','');
         INSERT INTO agent_runs VALUES('child','scan',1,'https://example.test','root');
         INSERT INTO agent_runs VALUES('other','scan',2,'https://example.test','');",
    )
    .unwrap();
    db.execute_batch(include_str!("budget/web_model_schema.sql"))
        .unwrap();
    db
}

#[test]
fn matches_unsettled_reservations_and_counts_root_and_child_tools_only() {
    let db = fixture();
    db.execute_batch(
        "INSERT INTO agent_budget_ledger VALUES('root',11,7,2,1,3,'fence');
         INSERT INTO agent_assignments(id,coordinator_run_id,reserved_tokens,reserved_requests,budget_settled_at)
           VALUES('a','root',10,1,'');
         INSERT INTO agent_assignments(id,coordinator_run_id,reserved_tokens,reserved_requests,budget_settled_at)
           VALUES('b','root',1,1,'');
         INSERT INTO agent_assignments(id,coordinator_run_id,reserved_tokens,reserved_requests,budget_settled_at)
           VALUES('settled','root',90,90,'done');
         INSERT INTO agent_assignments(id,coordinator_run_id,reserved_tokens,reserved_requests,budget_settled_at)
           VALUES('other','other',90,90,'');
         INSERT INTO agent_assignments(id,coordinator_run_id,reserved_tokens,reserved_requests,budget_settled_at,scheduling_model)
           VALUES('homogeneous','root',99,99,'','homogeneous_workers');
         INSERT INTO tool_invocations VALUES(1,'root','','');
         INSERT INTO tool_invocations VALUES(2,'child','key','done');
         INSERT INTO tool_invocations VALUES(3,'other','','');
         INSERT INTO agent_coordinator_leases VALUES('scan',1,'https://example.test','root',3,'fence');",
    )
    .unwrap();

    let report = budget_ledger_gaps(&db, "root").unwrap();
    assert!(report.ledger_exists);
    assert_eq!((report.reserved_tokens, report.spent_tokens), (11, 7));
    assert_eq!((report.reserved_requests, report.spent_requests), (2, 1));
    assert_eq!(report.unsettled_assignment_tokens, 11);
    assert_eq!(report.unsettled_assignment_requests, 2);
    assert_eq!(report.reservation_token_delta, Some(0));
    assert_eq!(report.reservation_request_delta, Some(0));
    assert_eq!(report.indeterminate_invocations, 1);
    assert_eq!(report.unclosed_web_model_calls, 0);
    assert_eq!(report.unkeyed_invocations, 1);
    assert!(report.coordinator_lease_found);
    assert!(report.fencing_matches_coordinator);
    assert_eq!(report.missing_dimensions.len(), 3);
    assert!(report.append_journal.is_none());
    assert!(!report
        .missing_dimensions
        .contains(&"target_requests".into()));
    assert!(!report.missing_dimensions.contains(&"wall_time_ms".into()));
    assert!(!report
        .missing_dimensions
        .contains(&"concurrency_batches".into()));
    assert!(report
        .missing_dimensions
        .contains(&"browser_actions".into()));
    assert!(report
        .missing_dimensions
        .contains(&"controlled_writes".into()));
    assert!(report.missing_dimensions.contains(&"upload_bytes".into()));
}

#[test]
fn mismatch_and_missing_ledger_are_reported_without_mutation() {
    let db = fixture();
    db.execute_batch(
        "INSERT INTO agent_budget_ledger VALUES('root',3,0,4,0,1,'old');
         INSERT INTO agent_assignments(id,coordinator_run_id,reserved_tokens,reserved_requests,budget_settled_at)
           VALUES('a','root',5,2,'');
         INSERT INTO agent_coordinator_leases VALUES('scan',1,'https://example.test','root',2,'new');",
    )
    .unwrap();

    let report = budget_ledger_gaps(&db, "root").unwrap();
    assert_eq!(report.reservation_token_delta, Some(-2));
    assert_eq!(report.reservation_request_delta, Some(2));
    assert!(!report.fencing_matches_coordinator);
    let ledger: (i64, i64) = db
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id='root'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(ledger, (3, 4));

    db.execute(
        "DELETE FROM agent_budget_ledger WHERE root_run_id='root'",
        [],
    )
    .unwrap();
    let missing = budget_ledger_gaps(&db, "root").unwrap();
    assert!(!missing.ledger_exists);
    assert_eq!(missing.unsettled_assignment_tokens, 5);
    assert_eq!(missing.reservation_token_delta, None);
    assert!(!missing.fencing_matches_coordinator);
}

#[test]
fn refuses_to_report_a_child_as_a_root() {
    let db = fixture();
    assert!(budget_ledger_gaps(&db, "child").is_err());
    assert!(budget_ledger_gaps(&db, "missing").is_err());
}

#[test]
fn missing_dimensions_are_exactly_the_denied_by_contract_set() {
    assert_eq!(
        REQUIRED_DIMENSIONS.len(),
        TRACKED_DIMENSIONS.len() + DENIED_BY_CONTRACT_DIMENSIONS.len()
    );
    for dimension in DENIED_BY_CONTRACT_DIMENSIONS {
        assert!(
            REQUIRED_DIMENSIONS.contains(dimension),
            "{dimension} must stay in REQUIRED"
        );
        assert!(
            !TRACKED_DIMENSIONS.contains(dimension),
            "{dimension} must not be claimed tracked"
        );
    }
    for dimension in TRACKED_DIMENSIONS {
        assert!(REQUIRED_DIMENSIONS.contains(dimension));
        assert!(!DENIED_BY_CONTRACT_DIMENSIONS.contains(dimension));
    }
    let db = fixture();
    db.execute_batch(
        "INSERT INTO agent_budget_ledger VALUES('root',1,0,1,0,1,'fence');
         INSERT INTO agent_coordinator_leases VALUES('scan',1,'https://example.test','root',1,'fence');",
    )
    .unwrap();
    let report = budget_ledger_gaps(&db, "root").unwrap();
    let mut missing = report.missing_dimensions.clone();
    missing.sort();
    let mut expected: Vec<String> = DENIED_BY_CONTRACT_DIMENSIONS
        .iter()
        .map(|d| (*d).to_string())
        .collect();
    expected.sort();
    assert_eq!(missing, expected);
}

#[test]
fn diagnostics_marks_denied_dimensions_as_denied_by_contract() {
    for dimension in DENIED_BY_CONTRACT_DIMENSIONS {
        assert!(
            !TRACKED_DIMENSIONS.contains(dimension),
            "{dimension} must stay untracked"
        );
        assert_eq!(
            super::super::budget::diagnostics::coverage_for(dimension, false),
            "denied_by_contract"
        );
    }
    for dimension in super::super::budget::DIMENSIONS {
        let coverage = super::super::budget::diagnostics::coverage_for(dimension, false);
        if DENIED_BY_CONTRACT_DIMENSIONS.contains(&dimension) {
            assert_eq!(coverage, "denied_by_contract");
        } else {
            assert_ne!(coverage, "denied_by_contract");
        }
    }
    assert_eq!(
        super::super::budget::diagnostics::coverage_for("concurrency_batches", true),
        "race_batch_dispatch_unimplemented"
    );
}

#[test]
fn denied_dimensions_remain_declared_ledger_columns() {
    use crate::agent_runtime::multi_agent::budget::DIMENSIONS;
    assert_eq!(
        DENIED_BY_CONTRACT_DIMENSIONS,
        &["browser_actions", "controlled_writes", "upload_bytes"]
    );
    for dimension in DENIED_BY_CONTRACT_DIMENSIONS {
        assert!(
            DIMENSIONS.contains(dimension),
            "{dimension} must remain a declared ledger column"
        );
    }
}

#[test]
fn denied_budget_dimension_writers_remain_unready() {
    use crate::agent_runtime::multi_agent::budget::{
        denied_budget_dimension_writer_ready, DENIED_BY_CONTRACT_DIMENSIONS,
    };
    assert_eq!(DENIED_BY_CONTRACT_DIMENSIONS.len(), 3);
    for dimension in DENIED_BY_CONTRACT_DIMENSIONS {
        assert!(
            !denied_budget_dimension_writer_ready(dimension),
            "{dimension} must not claim a live writer"
        );
    }
    assert!(!denied_budget_dimension_writer_ready("target_requests"));
    assert!(!denied_budget_dimension_writer_ready("totally-unknown"));
}
