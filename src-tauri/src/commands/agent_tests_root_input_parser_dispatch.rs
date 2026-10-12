// U24: a paid Root `dispatch:input_parser` choice must be refused zero-write and never
// fake a child when there is no eligible frozen frame backing it.
// U181 (pin updated, reason: U181 按计划只翻 InputParser): with the flag flipped the gate
// now reaches the validated mint, which rebuilds the U175 frozen frame — this minimal
// fixture has no observed controllable input, so the mint fails closed at the frame with
// `root_input_parser_trigger_not_eligible` (still zero-write, still no child). Under the
// test-only simulated flag-off the gate is the original honest `input_parser_dispatch_not_ready`.
fn input_parser_dispatch_fixture(
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
fn coordinator_root_input_parser_dispatch_selected_returns_not_ready_zero_write() {
    let (f, decision, seen) =
        input_parser_dispatch_fixture("input-parser-selected", &["dispatch:input_parser"]);
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let task = json!({"target":f.context.target_url,"objective":"frozen input inventory"});
    for _ in 0..2 {
        // Flag flipped (U181): the mint runs but fails closed at the frozen frame.
        let error = native_coordinator_prepare_input_parser(&f.context, &f.actor, &decision, &task)
            .unwrap_err();
        assert_eq!(error, "root_input_parser_trigger_not_eligible");
        web_mode_assert_rows(&db, &before);
        // Simulated flag-off: the original honest not_ready gate, still zero-write.
        let off = {
            let _off = InputParserProductionFlagOff::enter();
            native_coordinator_prepare_input_parser(&f.context, &f.actor, &decision, &task).unwrap_err()
        };
        assert_eq!(off, "input_parser_dispatch_not_ready");
        web_mode_assert_rows(&db, &before);
    }
    let children: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE role='input_parser'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(children, 0, "no InputParser assignment may be issued");
    // The same paid choice does not silently fall through to Mapper either.
    assert_eq!(
        native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap_err(),
        "root_bootstrap_did_not_select_mapper"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1, "no child SDK call or retry");
}

#[test]
fn coordinator_root_input_parser_dispatch_unselected_rejects_zero_write() {
    let (f, decision, seen) =
        input_parser_dispatch_fixture("input-parser-unselected", &["dispatch:spa_api_mapper"]);
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let task = json!({"target":f.context.target_url,"objective":"frozen input inventory"});
    assert_eq!(
        native_coordinator_prepare_input_parser(&f.context, &f.actor, &decision, &task)
            .unwrap_err(),
        "root_did_not_select_input_parser"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
