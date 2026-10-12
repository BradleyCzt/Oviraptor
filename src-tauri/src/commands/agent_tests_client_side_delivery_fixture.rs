// Witness has no owner, lease, capability or fee: it is not a historical grant.
fn client_delivery_foreign_invocation(f: &ClientHookFixture) {
    client_hook_add_foreign_historical_root(f);
    f.db.execute("INSERT INTO tool_invocations(run_id,tool_name,status,policy_decision,finished_at)
        VALUES('client-foreign-historical','unrelated_readonly_record','completed','allow','2000-01-01 00:00:00')",[]).unwrap();
}
fn client_delivery_assert_paid_and_running(f: &ClientHookFixture) {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler::ScheduledChild, specialist},
    };
    let (assignment, run): (String, String) =
        f.db.query_row(
            "SELECT id,child_run_id FROM agent_assignments
        WHERE coordinator_run_id=?1 AND role='client_side'",
            [&f.root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let child = ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: run.clone(),
        role: AgentRole::ClientSide,
    };
    let lease = &f.session.as_ref().unwrap().lease;
    let receipt = {
        let tx =
            rusqlite::Transaction::new_unchecked(&f.db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        let receipt = specialist::received_for_reconciliation(&tx, lease, &child).unwrap();
        tx.rollback().unwrap();
        receipt
    };
    assert_eq!(receipt.rejection, "");
    assert_eq!(receipt.usage.model_requests, 1);
    assert!(!receipt.text.is_empty());
    let worker =
        crate::agent_runtime::multi_agent::attempts::current(&f.db, lease, &assignment).unwrap();
    assert_eq!(worker.state, "running");
    assert_eq!(worker.finished_at, "");
    assert_eq!(worker.coordinator_epoch, lease.lease_epoch);
    assert_eq!(worker.coordinator_fencing_token, lease.fencing_token);
    assert_eq!(worker.child_run_id, run);
    assert!(f.db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        JOIN agent_lane_leases l ON l.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND a.state='running'
        AND a.budget_settled_at='' AND a.reserved_tokens=8000 AND a.reserved_requests=1
        AND r.status='running' AND r.finished_at='' AND l.lane='read_only_analysis')",params![assignment,run],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
            [&run],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&assignment],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(f.db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
        AND lease_attempt_id=?3 AND dimension='model_requests' AND kind='consume' AND amount=1)",params![f.root,assignment,worker.id],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
fn client_delivery_rows(db: &rusqlite::Connection, name: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let escaped = name.replace('"', "\"\"");
    let without_rowid: bool = db
        .query_row(
            "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
            [name],
            |r| r.get(0),
        )
        .unwrap();
    let sql = if without_rowid {
        let primary_keys: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
            .unwrap()
            .query_map([name], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            !primary_keys.is_empty(),
            "WITHOUT ROWID table must have its original primary key"
        );
        let order = primary_keys
            .iter()
            .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",");
        format!("SELECT * FROM \"{escaped}\" ORDER BY {order}")
    } else {
        format!("SELECT rowid,* FROM \"{escaped}\" ORDER BY rowid")
    };
    let mut q = db.prepare(&sql).unwrap();
    let n = q.column_count();
    let rows = q
        .query_map([], |r| {
            (0..n)
                .map(|i| r.get(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}
fn client_delivery_all_rows(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let mut q=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND (name NOT LIKE 'sqlite_%' OR name='sqlite_sequence') ORDER BY name").unwrap();
    let names = q
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|n| (n.clone(), client_delivery_rows(db, &n)))
        .collect()
}
