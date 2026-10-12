// Compare every application table's untouched physical scope. Original supplier
// closure (and, after real SDK I/O, Client local fees/cleanup) are separate facts.
fn client_hook_untouched_scope(f: &ClientHookFixture) -> Vec<(String, String)> {
    let db = &f.db;
    let session = f.session.as_ref().unwrap();
    let root = format!("'{}'", f.root.replace('\'', "''"));
    let supplier = format!("'{}'", session.executor.run_id.replace('\'', "''"));
    let assignment = format!("'{}'", session.executor.assignment_id.replace('\'', "''"));
    let clients = format!("SELECT child_run_id FROM agent_assignments WHERE coordinator_run_id={root} AND role='client_side'");
    let client_assignments = format!(
        "SELECT id FROM agent_assignments WHERE coordinator_run_id={root} AND role='client_side'"
    );
    let sdk_owners = format!("SELECT o.owner_id FROM native_sdk_log_owners o
        JOIN agent_assignments a ON a.id=o.assignment_id AND a.child_run_id=o.run_id
        JOIN agent_assignment_attempts x ON x.id=o.lease_attempt_id AND x.assignment_id=a.id
          AND x.child_run_id=o.run_id AND x.worker_id=o.worker_id
        WHERE o.domain='specialist' AND o.root_run_id={root} AND a.coordinator_run_id={root}
          AND a.role='client_side' AND x.root_run_id={root}
          AND EXISTS(SELECT 1 FROM agent_runs r WHERE r.id=o.run_id AND r.scan_id=o.scan_id AND r.attempt_number=o.attempt_number)");
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let excluded = match table.as_str() {
                "agent_runs" => format!("id={supplier} OR id IN ({clients})"),
                "agent_assignments" => format!("id={assignment} OR id IN ({client_assignments})"),
                "agent_assignment_attempts"
                | "agent_lane_leases"
                | "agent_capability_leases"
                | "agent_messages" => {
                    format!("assignment_id={assignment} OR assignment_id IN ({client_assignments})")
                }
                "agent_contract_owners" => format!("root_run_id={root} AND assignment_id IN ({client_assignments})"),
                "native_sdk_log_owners" => format!("owner_id IN ({sdk_owners})"),
                "native_sdk_log_rows" => format!("owner_id IN ({sdk_owners})"),
                "agent_specialist_calls" => format!("assignment_id IN ({client_assignments})"),
                "agent_budget_ledger" => format!("root_run_id={root}"),
                "agent_budget_entries" => format!("rowid>{} AND root_run_id={root}", f.entry_floor),
                "agent_snapshots" => format!("run_id={root} OR run_id={supplier} OR run_id IN ({clients})"),
                "agent_events" => {
                    format!("rowid>{} AND (run_id={root} OR run_id={supplier} OR run_id IN ({clients}))",f.event_floor)
                }
                "agent_collaboration_events" => format!("sequence>{} AND (entity_id={root} OR entity_id={supplier} OR entity_id={assignment} OR entity_id IN ({clients}) OR entity_id IN ({client_assignments}) OR entity_id IN (SELECT id FROM agent_messages WHERE assignment_id={assignment} OR assignment_id IN ({client_assignments})))",f.collaboration_floor),
                "agent_coordinator_leases" => format!("root_run_id={root}"),
                "sqlite_sequence" => format!("name IN ('agent_events','agent_collaboration_events') OR
                    (name='native_sdk_log_rows' AND seq=(SELECT MAX(sequence) FROM native_sdk_log_rows)
                    AND EXISTS(SELECT 1 FROM native_sdk_log_owners WHERE owner_id IN ({sdk_owners}))
                    AND NOT EXISTS(SELECT 1 FROM native_sdk_log_rows WHERE owner_id NOT IN ({sdk_owners})))"),
                _ => "0".into(),
            };
            let quoted = format!("\"{}\"", table.replace('"', "\"\""));
            let without_rowid: bool = db.query_row("SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1", [&table], |r| r.get(0)).unwrap();
            let sql = if without_rowid {
                let primary_keys: Vec<String> = db.prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk").unwrap()
                    .query_map([&table], |r| r.get::<_, String>(0)).unwrap()
                    .collect::<rusqlite::Result<_>>().unwrap();
                assert!(!primary_keys.is_empty(), "WITHOUT ROWID table must have its original primary key");
                let order = primary_keys.iter().map(|column| format!("\"{}\"", column.replace('"', "\"\""))).collect::<Vec<_>>().join(",");
                format!("SELECT * FROM {quoted} WHERE NOT ({excluded}) ORDER BY {order}")
            } else {
                format!("SELECT rowid,* FROM {quoted} WHERE NOT ({excluded}) ORDER BY rowid")
            };
            let mut q = db.prepare(&sql).unwrap();
            let n = q.column_count();
            let rows = q
                .query_map([], |r| {
                    (0..n)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, format!("{rows:?}"))
        })
        .collect()
}
fn client_hook_assert_supplier_closure(f: &ClientHookFixture) {
    let supplier = &f.session.as_ref().unwrap().executor;
    assert!(f.db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=r.id WHERE a.id=?1 AND r.id=?2
        AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0
        AND r.status='terminal' AND r.terminal_state='completed' AND r.used_tokens=0 AND r.used_requests=0
        AND x.state='completed' AND x.finished_at<>'')",params![supplier.assignment_id,supplier.run_id],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
            [&supplier.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&supplier.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'
        AND delivery_attempts=1 AND delivered_at<>'' AND acknowledged_at<>''",
            [&supplier.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
fn client_hook_assert_no_new_grant(f: &ClientHookFixture) {
    for table in ["agent_assignments", "agent_runs", "agent_specialist_calls"] {
        let count = match table {
            "agent_assignments" => "SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'",
            "agent_runs" => "SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND role='client_side'",
            _ => "SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND role='client_side'",
        };
        assert_eq!(
            f.db.query_row(count, [&f.root], |r| r.get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(f.db.query_row("SELECT reserved_tokens,reserved_requests,spent_tokens,spent_requests FROM agent_budget_ledger
        WHERE root_run_id=?1",[&f.root],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap(),(0,0,0,0));
    assert_eq!(f.model_seen.lock().unwrap().len(), 0);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
fn client_hook_original_supplier_rows(f: &ClientHookFixture) -> Vec<String> {
    let supplier = &f.session.as_ref().unwrap().executor;
    [
        ("agent_runs", "id", supplier.run_id.as_str()),
        ("agent_assignments", "id", supplier.assignment_id.as_str()),
        (
            "agent_assignment_attempts",
            "assignment_id",
            supplier.assignment_id.as_str(),
        ),
    ]
    .into_iter()
    .map(|(table, column, key)| {
        let mut q =
            f.db.prepare(&format!(
                "SELECT rowid,* FROM {table} WHERE {column}=?1 ORDER BY rowid"
            ))
            .unwrap();
        let n = q.column_count();
        let rows = q
            .query_map([key], |r| {
                (0..n)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        format!("{table}:{rows:?}")
    })
    .collect()
}
fn client_hook_add_foreign_historical_root(f: &ClientHookFixture) {
    // An unrelated historical Native row is a rollback witness only. No owner,
    // clock, limits, lease or execution grant is added to this old row.
    f.db.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,root_run_id,
        status,plan_hash,plan_json,orchestration_policy,hard_token_budget,used_tokens,created_at)
        VALUES('client-foreign-historical',?1,1,'https://other.example.invalid','native','coordinator',
        'client-foreign-historical','prepared','old-hash','{}','multi',777,7,'2000-01-01 00:00:00')",
        [&f.context.scan_id]).unwrap();
}
