// U26: Root `dispatch:upload` / `dispatch:concurrency` stay unimplemented (U21 codes).
fn stub_role_dispatch_fixture(
    tag: &str,
    suggestions: &'static [&'static str],
) -> (
    RootTickFixture,
    NativeCoordinatorTickReceipt,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
        let body = if request.contains("You are the Root Coordinator") {
            proposal_model_response(
                &json!({"schemaVersion":1,"observed":["actual original frozen fact"],"missing":[],
                    "suggestions":suggestions,"costNotes":[],"risks":[]})
                .to_string(),
            )
        } else {
            proposal_model_response(
                r#"{"summary":"unexpected child call","priorityContracts":[],"risks":[]}"#,
            )
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(tag, &format!("http://127.0.0.1:{port}/v1"));
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1, "exactly one paid Root tick");
    (f, decision, seen)
}

#[test]
fn coordinator_root_upload_dispatch_selected_returns_not_implemented_zero_write() {
    let (f, decision, seen) =
        stub_role_dispatch_fixture("upload-selected", &["dispatch:upload"]);
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let task = json!({"target":f.context.target_url,"objective":"frozen upload inventory"});
    for _ in 0..2 {
        let error =
            native_coordinator_prepare_upload(&f.context, &f.actor, &decision, &task).unwrap_err();
        assert_eq!(error, "upload_role_not_implemented");
        web_mode_assert_rows(&db, &before);
    }
    let children: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE role='upload'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(children, 0, "no Upload assignment may be issued");
    assert_eq!(
        native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap_err(),
        "root_bootstrap_did_not_select_mapper"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1, "no child SDK call or retry");
}

#[test]
fn coordinator_root_upload_dispatch_unselected_rejects_zero_write() {
    let (f, decision, seen) =
        stub_role_dispatch_fixture("upload-unselected", &["dispatch:spa_api_mapper"]);
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let task = json!({"target":f.context.target_url,"objective":"frozen upload inventory"});
    assert_eq!(
        native_coordinator_prepare_upload(&f.context, &f.actor, &decision, &task).unwrap_err(),
        "root_did_not_select_upload"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn coordinator_root_concurrency_dispatch_selected_returns_not_implemented_zero_write() {
    let (f, decision, seen) =
        stub_role_dispatch_fixture("concurrency-selected", &["dispatch:concurrency"]);
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let task = json!({"target":f.context.target_url,"objective":"frozen concurrency inventory"});
    for _ in 0..2 {
        let error = native_coordinator_prepare_concurrency(&f.context, &f.actor, &decision, &task)
            .unwrap_err();
        assert_eq!(error, "concurrency_role_not_implemented");
        web_mode_assert_rows(&db, &before);
    }
    let children: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE role='concurrency'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(children, 0, "no Concurrency assignment may be issued");
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1, "no child SDK call or retry");
}

#[test]
fn coordinator_root_concurrency_dispatch_unselected_rejects_zero_write() {
    let (f, decision, seen) =
        stub_role_dispatch_fixture("concurrency-unselected", &["dispatch:spa_api_mapper"]);
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let task = json!({"target":f.context.target_url,"objective":"frozen concurrency inventory"});
    assert_eq!(
        native_coordinator_prepare_concurrency(&f.context, &f.actor, &decision, &task)
            .unwrap_err(),
        "root_did_not_select_concurrency"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
