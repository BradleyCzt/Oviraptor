// Full original creator -> Root/Mapper -> Web SDK/Broker -> Client -> Root chain.
#[test]
fn root_budget_trigger_actual_executor_entry_assesses_original_allocation_before_target_once() {
    let _real = RealSpecialistTransport::enter();
    let mut h = client_root_fixture("root-budget-trigger-production", false);
    let db = db::open(&h.h.db_path).unwrap();
    // Before finish, bootstrap, Mapper output and the original Web allocation
    // have been paid. The Web request itself has reached the real Broker once.
    assert_eq!(h.root_cost(&db), 3);
    assert_eq!(root_tick_count(&db, &h.root, "publication"), 3);
    assert_eq!(h.h.site_seen.lock().unwrap().len(), 1);
    h.finish().unwrap();
    // Finish adds the one closed HTTP observation and closed Client output;
    // neither advisory assessment issues another target grant.
    assert_eq!(h.root_cost(&db), 5);
    assert_eq!(root_tick_count(&db, &h.root, "publication"), 5);
    assert_eq!(h.h.site_seen.lock().unwrap().len(), 1);
    let expected = ["bootstrap", "mapper-output", "budget-allocation", "new-evidence", "client-side-output"];
    let snapshot = db.unchecked_transaction().unwrap();
    let requests: Vec<(String, i64, String, i64)> = snapshot.prepare(
        "SELECT q.call_id,q.round,q.fact_json,json_extract(p.fact_json,'$.eventSequence')
         FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p
           ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.round=q.round
           AND p.lease_attempt_id=q.lease_attempt_id AND p.request_hash=q.request_hash AND p.basis_hash=q.basis_hash
         WHERE q.root_run_id=?1 AND q.phase='request' AND p.phase='publication' ORDER BY q.round",
    ).unwrap().query_map([&h.root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(root_tick_count(&snapshot,&h.root,"request"),5);
    assert_eq!(requests.len(),expected.len());
    let executor = &h.session.as_ref().unwrap().executor.run_id;
    let evidence_ids: Vec<String> = snapshot.prepare(
        "SELECT id FROM agent_evidence_nodes WHERE root_run_id=?1 AND created_by_run_id=?2
         AND kind='request_record' AND provenance='observed' ORDER BY rowid,id",
    ).unwrap().query_map(params![h.root,executor],|r|r.get(0)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(evidence_ids.len(),1);
    let node = crate::agent_runtime::evidence_graph::store::load_evidence_node(
        &snapshot,&evidence_ids[0],
    ).unwrap().unwrap();
    for (index,(call,round,raw,event)) in requests.iter().enumerate() {
        assert_eq!(*round,index as i64+1);
        let fact: JsonValue = serde_json::from_str(raw).unwrap();
        assert_eq!(*raw,fact.to_string());
        let basis = &fact["request"]["basis"];
        assert_eq!(basis["phase"],expected[index]);
        assert_eq!(basis["root"],h.root);
        let projected = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(
            &snapshot,&h.root,*event,
        ).unwrap();
        // Model publication and its later UI channel event use distinct
        // immutable sequences; verify their explicit provenance link.
        assert_eq!(projected.record["modelEventSequence"],*event);
        assert_eq!(projected.root,h.root);
        assert!(projected.sequence>0);
        let fees:(i64,i64) = snapshot.query_row(
            "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries
             WHERE root_run_id=?1 AND assignment_id='' AND dimension='model_requests'
              AND kind='consume' AND source_id=?2",
            params![h.root,call],|r|Ok((r.get(0)?,r.get(1)?)),
        ).unwrap();
        assert_eq!(fees,(1,1),"each original phase has one actual Root request fee");
        if expected[index]=="new-evidence" {
            let changed = &basis["changedFact"];
            assert_eq!(changed["kind"],"new-evidence");
            assert_eq!(changed["semantic"]["binding"],json!({"evidenceId":node.id,
                "revision":node.revision,"authorRunId":node.created_by_run_id,
                "naturalKeyHash":node.natural_key_hash,"artifactRefs":node.artifact_refs,"payload":node.payload}));
            assert_eq!(changed["semantic"]["dispatchGranted"],false);
            assert_eq!(changed["semantic"]["targetRequestsGranted"],0);
        }
    }
    snapshot.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = h.h.model_seen.lock().unwrap().len();
    h.finish().unwrap();
    h.feedback(&db).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert_eq!(h.h.model_seen.lock().unwrap().len(),calls);
    assert_eq!(h.h.site_seen.lock().unwrap().len(),1);
    assert_eq!(h.root_cost(&db),5);
    assert_eq!(root_tick_count(&db,&h.root,"publication"),5);
    let wires = h.h.model_seen.lock().unwrap();
    let root_phases: Vec<String> = wires.iter().filter(|w|w.contains("You are the Root Coordinator"))
        .map(|w| {
            let body: JsonValue = serde_json::from_str(w.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            input["basis"]["phase"].as_str().unwrap().to_string()
        }).collect();
    assert_eq!(root_phases,expected);
    let budget = wires
        .iter()
        .position(|w| w.contains("You are the Root Coordinator") && w.contains("budget-allocation"))
        .expect("actual paid Root budget-change request");
    let executor = wires
        .iter()
        .position(|w| {
            w.contains("replay_http")
                && !w.contains("You are the Root Coordinator")
                && !w.contains("SPA/API Mapper")
        })
        .unwrap();
    assert!(
        budget < executor,
        "Root budget supervision precedes any Web executor SDK"
    );
}

#[test]
fn root_budget_trigger_same_running_worker_paid_fact_replay_is_zero_write_zero_sdk() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let child = &f.session.as_ref().unwrap().executor;
    let original = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_assignments WHERE id=?1",
        [&child.assignment_id],
    )
    .unwrap();
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    assert_eq!(root_tick_count(&db, root, "publication"), 3);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap()
            .consumed,
        3
    );
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_assignments WHERE id=?1",
            [&child.assignment_id]
        )
        .unwrap()
            == original
    );
    let before = web_mode_test_rows(&db);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    // A paid assessment can never become another target-dispatch capability.
    let session = f.session.as_ref().unwrap();
    let root_context = native_coordinator_root_context(&f.h.context, &session.lease);
    let (origin, paid) =
        native_coordinator_mapper_decision(&root_context, &session.lease, &session.mapper).unwrap();
    let frame = NativeCoordinatorFrame::budget_allocation(
        &db,
        &root_context,
        &session.lease,
        child,
        origin,
        &paid,
    )
    .unwrap();
    let assessment =
        native_coordinator_tick_for_frame(&root_context, &session.lease, &frame).unwrap();
    assert!(assessment.replayed);
    assert_eq!(
        native_coordinator_rust_dispatch_policy(
            &root_context,
            &frame,
            &assessment,
            crate::agent_runtime::contract::AgentRole::WebExecutor,
            512,
            1,
            [Some(60000), Some(20), Some(32)]
        )
        .unwrap_err(),
        "root_trigger_role_conflict"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
}
