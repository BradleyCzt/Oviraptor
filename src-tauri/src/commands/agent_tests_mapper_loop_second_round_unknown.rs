// Real second dispatch failure. No synthetic terminal transition or ledger row.
#[test]
fn mapper_loop_second_actual_sdk_503_retains_first_lookup_unknown_estimate_and_paused_diagnostics_without_retry(
) {
    use crate::agent_runtime::multi_agent::{
        budget, lease::CoordinatorLease, scheduler::ScheduledChild, specialist::mapper_loop,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let sdk_rounds = Arc::new(AtomicUsize::new(0));
    let counted = sdk_rounds.clone();
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |request| {
        let wire: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        if wire["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("You are the Root Coordinator")
        {
            let input: JsonValue =
                serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(
                input["basis"]["phase"], "bootstrap",
                "unknown second call cannot reach Mapper output evaluation"
            );
            return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["frozen evidence"],"missing":[],"suggestions":["dispatch:spa_api_mapper"],"costNotes":[],"risks":[]}).to_string()));
        }
        assert_eq!(wire["max_tokens"], 512);
        match counted.fetch_add(1,Ordering::SeqCst) {
            0=>(200,"application/json",json!({"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"paid-original-lookup","type":"function","function":{"name":"evidence.lookup","arguments":"{\"evidenceId\":\"frontend-evidence\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":7,"completion_tokens":5,"total_tokens":12,"prompt_tokens_details":{"cached_tokens":2}}}).to_string()),
            1=>{
                let messages=wire["messages"].as_array().unwrap(); assert_eq!(messages.len(),4);
                assert_eq!(messages[3]["tool_call_id"],"paid-original-lookup");
                let observation:JsonValue=serde_json::from_str(messages[3]["content"].as_str().unwrap()).unwrap();
                assert_eq!(observation["ok"],true); assert_eq!(observation["evidenceId"],"frontend-evidence");
                (503,"application/json",json!({"error":{"message":"actual second provider outcome has no usage"}}).to_string())
            },
            n=>panic!("unexpected Mapper resend {n}"),
        }
    }));
    let mut f = root_tick_fixture_from_web_mode_evidence_protocol(
        "mapper-loop-second-actual-503",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence {
            mapper_v15: true,
            ..Default::default()
        },
    );
    drop(f.parent.take());
    let error = multi_agent_prepare(&mut f.context)
        .err()
        .expect("actual 503 must stop the loop");
    // Gateway Server.code() is the stable provider-failure code; the
    // transport status remains diagnostic data rather than an error contract.
    assert_eq!(error, "mapper_loop_uncertain:model_provider_failure");
    assert_eq!(sdk_rounds.load(Ordering::SeqCst), 2);
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "one Root SDK plus two original Mapper requests, no transport retry"
    );
    let db = db::open(&f.context.db_path).unwrap();
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.actor.root_run_id],|r|Ok(CoordinatorLease{scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).unwrap();
    let child=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='spa_api_mapper'",[&actor.root_run_id],|r|Ok(ScheduledChild{assignment_id:r.get(0)?,run_id:r.get(1)?,role:crate::agent_runtime::contract::AgentRole::SpaApiMapper})).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let first:String=tx.query_row("SELECT call_id FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND round_number=1 AND phase='dispatch'",[&child.run_id],|r|r.get(0)).unwrap();
    let second:String=tx.query_row("SELECT call_id FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND round_number=2 AND phase='dispatch'",[&child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(
        mapper_loop::cost_phase_original_on(&tx, &actor, &child, &first)
            .unwrap()
            .as_deref(),
        Some("received")
    );
    assert_eq!(
        mapper_loop::cost_phase_original_on(&tx, &actor, &child, &second)
            .unwrap()
            .as_deref(),
        Some("uncertain")
    );
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='lookup'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND round_number=2 AND phase IN ('invoice','received','unsent')",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let estimate:i64=tx.query_row("SELECT json_extract(fact_json,'$.estimate') FROM agent_mapper_loop_journal WHERE call_id=?1 AND phase='dispatch'",[&second],|r|r.get(0)).unwrap();
    for (dimension, known) in [
        ("model_input_tokens", 7),
        ("model_cached_tokens", 2),
        ("model_output_tokens", 5),
        ("model_requests", 1),
    ] {
        let held = budget::balance(
            &tx,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!(held.consumed, known);
        assert_eq!(
            held.indeterminate,
            if dimension == "model_requests" {
                1
            } else {
                estimate
            }
        );
        let exact:i64=tx.query_row("SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND dimension=?3 AND kind='forfeit' AND amount=?4 AND source_id=?5",params![actor.root_run_id,child.assignment_id,dimension,if dimension=="model_requests" {1}else{estimate},format!("mapper-loop:{second}")],|r|r.get(0)).unwrap();
        assert_eq!(exact, 1);
    }
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM agent_mapper_loop_final_receipts WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(mapper_loop::verified_original_output_on(&tx, &actor, &child).is_err());
    drop(tx);
    // The real terminal writer must prove all original SDKs exited before
    // revoking this worker. Unknown bills remain debts, not completed work.
    let frozen = mapper_loop_unknown_financial_rows(&db, &child);
    let outcome = AgentTargetOutcome::incomplete(
        "Original second Mapper provider outcome requires reconciliation",
    );
    finish_coordinator_run(&db, &actor, &outcome)
        .expect("actual Mapper SDK exit permits noncompletion Root closure");
    let closure:(String,String,String,String,String,String,i64,i64)=db.query_row("SELECT a.state,r.status,x.state,a.finished_at,r.finished_at,x.finished_at,(SELECT count(*) FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''),(SELECT count(*) FROM agent_lane_leases WHERE assignment_id=a.id) FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=r.id WHERE a.id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).unwrap();
    assert_eq!(
        closure,
        (
            "paused".into(),
            "paused".into(),
            "paused".into(),
            "".into(),
            "".into(),
            "".into(),
            0,
            0
        )
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_runs WHERE id=?1 AND status='terminal' AND terminal_state=?2",params![actor.root_run_id,outcome.terminal_status()],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(
        mapper_loop_unknown_financial_rows(&db, &child) == frozen,
        "Root cleanup preserves every original Mapper receipt, fee, forfeit and remaining reserve"
    );
    let terminal_snapshot = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db, &actor, &outcome).unwrap();
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        terminal_snapshot,
        "terminal closure replay is completely free"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&actor.scan_id]
        )
        .unwrap(),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        mapper_loop::cost_phase_original_on(&tx, &actor, &child, &second)
            .unwrap()
            .as_deref(),
        Some("uncertain")
    );
    mapper_loop::verify_for_idle_on(&tx, &actor, &child).unwrap();
    drop(tx);
    let page = crate::agent_runtime::model::diagnostics::replay::read(
        &f.context.db_path,
        &actor.scan_id,
        actor.attempt_number,
        None,
        None,
        0,
        300,
    )
    .unwrap();
    let page = serde_json::to_value(page).unwrap();
    let owners = page["owners"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|owner| owner["domain"] == "mapper_loop")
        .collect::<Vec<_>>();
    assert_eq!(owners.len(), 2, "two original physical dispatch owners");
    assert_ne!(owners[0]["ownerId"], owners[1]["ownerId"]);
    let all_rows = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["domain"] == "mapper_loop")
        .collect::<Vec<_>>();
    assert_eq!(
        all_rows.len(),
        11,
        "six known and five uncertain lifecycle rows"
    );
    for (round, call, stages, cost, terminal) in [
        (
            1,
            &first,
            vec![
                "prepared",
                "sent",
                "response_received",
                "cost_saved",
                "validated",
                "terminal",
            ],
            "received",
            "returned",
        ),
        (
            2,
            &second,
            vec![
                "prepared",
                "sent",
                "response_received",
                "cost_saved",
                "terminal",
            ],
            "uncertain",
            "uncertain",
        ),
    ] {
        let owner = owners.iter().find(|owner| owner["round"] == round).unwrap();
        assert_eq!(owner["dispatchKey"], *call);
        assert_eq!(owner["assignmentId"], child.assignment_id);
        assert_eq!(owner["runId"], child.run_id);
        let rows = all_rows
            .iter()
            .filter(|row| row["ownerId"] == owner["ownerId"])
            .collect::<Vec<_>>();
        assert_eq!(
            rows.iter()
                .map(|row| row["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            stages
        );
        assert!(rows
            .iter()
            .all(|row| row["dispatchKey"] == *call && row["round"] == round));
        assert_eq!(
            rows.iter()
                .find(|row| row["stage"] == "cost_saved")
                .unwrap()["costPhase"],
            cost
        );
        assert_eq!(rows.last().unwrap()["terminalState"], terminal);
    }
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "paused diagnostics cannot repair or refund original debt"
    );
    assert!(multi_agent_prepare(&mut f.context).is_err());
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "paused reentry cannot release reserves or invent completion"
    );
}

fn mapper_loop_unknown_financial_rows(
    db: &rusqlite::Connection,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Vec<crate::agent_runtime::multi_agent::attempts::audit_rows::Rows> {
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    vec![
        Rows::read(db,"SELECT rowid,* FROM agent_mapper_loop_journal WHERE child_run_id=?1 ORDER BY rowid",[&child.run_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_model_cost_facts WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT reserved_tokens,reserved_requests,budget_settled_at FROM agent_assignments WHERE id=?1",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_messages WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_mapper_loop_final_receipts WHERE child_run_id=?1 ORDER BY rowid",[&child.run_id]).unwrap(),
    ]
}
