// Stage S0: existing real production entry points only. No new shape/stub API.
// Parent runs these red before implementing Single financial finalization.
// Fixture model/site listeners are never sent a request by this batch.

fn single_finally_fixture(tag: &str) -> AgentHarness {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let mut harness = root_budget_harness(tag);
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    // Freeze this clock before original control/limits exist, never rewrite a
    // frozen origin or history to force a passing finalization test.
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute("UPDATE agent_runs SET created_at=strftime('%Y-%m-%d %H:%M:%f','now','-3 seconds','localtime') WHERE id=?1", [root]).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    RootOwner::initialize_financial_fixture_for_test(&tx, root).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_coordinator_leases", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    // Empty name causes actual native_model_setup/profile resolution to stop
    // deterministically, before SDK transport or Native state writes.
    harness.context.environment.deployment = "cloud".into();
    harness.context.environment.llm.clear();
    drop(db);
    harness
}

fn single_finally_lower_bound(harness: &AgentHarness) -> i64 {
    let db = db::open(&harness.db_path).unwrap();
    db.query_row("SELECT CAST((julianday('now','localtime')-julianday(started_at))*86400000 AS INTEGER) FROM agent_budget_clock_origins WHERE root_run_id=?1", [&harness.context.run.as_ref().unwrap().run_id], |r|r.get(0)).unwrap()
}

fn single_finally_original_sources(db: &rusqlite::Connection, root: &str) -> String {
    let mut output = String::new();
    for table in [
        "agent_root_budget_attempts",
        "agent_budget_limits",
        "agent_budget_clock_origins",
        "agent_root_model_journal",
        "agent_http_request_claims",
    ] {
        let filter = if table == "agent_http_request_claims" {
            "run_id"
        } else {
            "root_run_id"
        };
        let mut q = db
            .prepare(&format!(
                "SELECT rowid,* FROM {table} WHERE {filter}=?1 ORDER BY rowid"
            ))
            .unwrap();
        let count = q.column_count();
        let rows = q
            .query_map([root], |r| {
                (0..count)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        output.push_str(&format!("{table}:{rows:?}"));
    }
    agent_stable_hash(&json!(output))
}

fn single_finally_physical(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let names = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let without_rowid: bool = db
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [&name],
                    |r| r.get(0),
                )
                .unwrap();
            let quoted = name.replace('"', "\"\"");
            let sql = if without_rowid {
                let primary_keys: Vec<String> = db
                    .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
                    .unwrap()
                    .query_map([&name], |r| r.get::<_, String>(0))
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
                format!("SELECT * FROM \"{quoted}\" ORDER BY {order}")
            } else {
                format!("SELECT rowid,* FROM \"{quoted}\" ORDER BY rowid")
            };
            let mut q = db.prepare(&sql).unwrap();
            let count = q.column_count();
            let rows = q
                .query_map([], |r| {
                    (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn single_finally_verify_cost(harness: &AgentHarness, lower: i64, sources: &str) {
    use crate::agent_runtime::multi_agent::budget;
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let upper = single_finally_lower_bound(harness);
    let wall = budget::balance(&db, root, None, "wall_time_ms").unwrap();
    assert!(
        wall.consumed >= lower && wall.consumed <= upper,
        "missing original final interval: lower={lower}, wall={wall:?}, upper={upper}"
    );
    assert_eq!((wall.reserved, wall.indeterminate), (0, 0));
    assert_eq!(
        single_finally_original_sources(&db, root),
        sources,
        "old physical original owner/limits/origin/source changed"
    );
    let bad:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries e WHERE root_run_id=?1 AND dimension='wall_time_ms' AND (assignment_id<>'' OR lease_attempt_id<>(SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1)))",[root],|r|r.get(0)).unwrap();
    assert!(!bad, "no child/fake C may pay Single elapsed");
    for table in [
        "agent_coordinator_leases",
        "agent_assignments",
        "agent_assignment_attempts",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert!(harness.model_seen.lock().unwrap().is_empty());
    assert!(harness.site_seen.lock().unwrap().is_empty());
}
