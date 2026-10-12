// Test-only isolated fixture using actual ordinary new-URL creation/startup/HMAC.
#[cfg(test)]
struct WebModeFixture {
    root: PathBuf,
    path: PathBuf,
    scan: String,
    work: PathBuf,
    target: String,
    preserve: bool,
}
#[cfg(test)]
impl Drop for WebModeFixture {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
fn web_mode_test_rows(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let mut statement = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let names = statement
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|table| {
            let quoted = table.replace('"', "\"\"");
            let without_rowid: bool = db.query_row(
                "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                [&table], |r| r.get(0),
            ).unwrap();
            let sql = if without_rowid {
                format!("SELECT * FROM \"{quoted}\"")
            } else {
                format!("SELECT * FROM \"{quoted}\" ORDER BY rowid")
            };
            let mut statement = db.prepare(&sql).unwrap();
            let count = statement.column_count();
            let mut rows = statement
                .query_map([], |r| {
                    (0..count)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            if without_rowid {
                rows.sort_by_cached_key(|row| format!("{row:?}"));
            }
            (table, rows)
        })
        .collect()
}
#[cfg(test)]
fn web_mode_assert_rows(
    db: &rusqlite::Connection,
    original: &[(String, Vec<Vec<rusqlite::types::Value>>)],
) {
    let after = web_mode_test_rows(db);
    assert_eq!(after.len(), original.len());
    for ((name, rows), (old_name, old_rows)) in after.iter().zip(original) {
        assert!(
            name == old_name && rows == old_rows,
            "application table changed: {name}"
        );
    }
}

#[cfg(test)]
fn web_mode_test_draft(
    db: &rusqlite::Connection,
    input: Option<&str>,
    target: &str,
) -> Result<SentinelScan, String> {
    create_sentinel_url_scan_with_mode_in(
        db,
        9001,
        "Mode fixture".into(),
        vec![target.into()],
        Some("standard".into()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        input.map(str::to_string),
    )
}

#[cfg(test)]
fn web_mode_test_start(
    root: &Path,
    db: &mut rusqlite::Connection,
    scan: &str,
    mode: WebStartMode,
) -> Result<PathBuf, String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let mut startup = prepare_web_startup_in(&tx, root, scan, mode)?;
    let policy: String = tx
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let policy: JsonValue = serde_json::from_str(&policy).map_err(|e| e.to_string())?;
    let targets = startup
        .targets
        .iter()
        .map(|(company, url)| serde_json::json!({"company":company,"url":url}))
        .collect::<Vec<_>>();
    fs::write(
        startup.files.path.join("task.json"),
        serde_json::to_vec(&serde_json::json!({
        "scanId":scan,"projectId":9001,"targets":targets,"effectiveWebPolicy":policy}))
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        startup.files.path.join("targets.json"),
        serde_json::to_vec(&targets).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        startup.files.path.join("targets.txt"),
        startup
            .targets
            .iter()
            .map(|(_, url)| url.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        startup.files.path.join("agent-instruction.md"),
        "isolated test input",
    )
    .map_err(|e| e.to_string())?;
    if let Some(auth) = web_dispatch_auth_document(&tx, 9001, &policy)? {
        crate::auth_session::write_session_document(
            &startup.files.path.join("auth-sessions.json"), &auth,
        )?;
    }
    persist_web_startup_in(&tx, &startup, "mode test startup", "", &policy)?;
    let runtime = serde_json::json!({"model":{"provider":"test-owned"},"runtime":{},"settings":{},"build":"mode-contract"});
    register_web_dispatch_binding_in(&tx, &startup, &runtime)?;
    register_private_web_mode_on(&tx, &startup, &runtime)?;
    let work = startup.files.path.clone();
    startup.files.preserve = true;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(work)
}

#[cfg(test)]
fn web_mode_fixture(mode: &str, target: &str) -> WebModeFixture {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("oviraptor-mode-{}", Uuid::new_v4()));
    web_mode_fixture_at_root(mode, target, root)
}

#[cfg(test)]
fn web_mode_fixture_at_root(mode: &str, target: &str, root: PathBuf) -> WebModeFixture {
    let path = db::initialize(&root).unwrap();
    let mut db = db::open(&path).unwrap();
    db.execute("INSERT INTO projects(id,name) VALUES(9001,'Mode test')", [])
        .unwrap();
    let scan = web_mode_test_draft(&db, Some(mode), target).unwrap().id;
    let work = web_mode_test_start(&root, &mut db, &scan, WebStartMode::Confirm).unwrap();
    WebModeFixture {
        root,
        path,
        scan,
        work,
        target: target.into(),
        preserve: false,
    }
}

#[cfg(test)]
fn web_mode_legacy_start_fixture() -> (PathBuf, PathBuf, rusqlite::Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-legacy-mode-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let db = db::open(&path).unwrap();
    db.execute_batch("INSERT INTO projects(id,name) VALUES(1,'Startup');
        INSERT INTO sentinel_scans(id,project_id,project_name,status,task_name) VALUES('start-test',1,'Startup','draft','Startup');
        INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES('start-test','internal','{}');
        INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'start-test','one','https://start.example.test/one','queued'),(1,'start-test','two','https://start.example.test/two','queued');").unwrap();
    (root, path, db)
}
