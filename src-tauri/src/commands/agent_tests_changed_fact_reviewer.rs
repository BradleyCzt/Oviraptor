fn changed_fact_reviewer_response(request: &str, investigate: bool) -> String {
    if specialist_original_is_web_sdk(request) {
        return proposal_model_response("actual bounded original Web exit; no target tool");
    }
    if request.contains("You are the Root Coordinator") {
        let mut wire: JsonValue =
            serde_json::from_str(&changed_fact_response(request, true)).unwrap();
        if request.contains("investigator-proposal") {
            let mut summary: JsonValue =
                serde_json::from_str(wire["choices"][0]["message"]["content"].as_str().unwrap())
                    .unwrap();
            summary["suggestions"] = json!(["assess:gap_proposal"]);
            wire["choices"][0]["message"]["content"] = summary.to_string().into();
        }
        if request.contains("reviewer-decision") && !investigate {
            let mut summary: JsonValue =
                serde_json::from_str(wire["choices"][0]["message"]["content"].as_str().unwrap())
                    .unwrap();
            summary["suggestions"] = json!([]);
            wire["choices"][0]["message"]["content"] = summary.to_string().into();
        }
        return wire.to_string();
    }
    if request.contains("独立 Evidence Reviewer") {
        return proposal_model_response(&json!({"verdict":"insufficient_evidence","reasonCodes":["missing_control"],
            "missingEvidence":["need independent control"],"confidence":0.1,"summary":"actual independent Reviewer refuses confirmation"}).to_string());
    }
    if request.contains("独立的只读 Deep Investigator") {
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        return proposal_model_response(&json!({"summary":"actual independent gap assessment","nextStep":"manual_review",
            "gapCode":"independent_control_missing","supportingFactRefs":[],"missingEvidence":input["reviewerMissingEvidence"],
            "prerequisites":["operator supplies a fresh approved control"],"proposedContracts":[],
            "expectedInformationGain":0.1,"impactCeiling":"low","estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":0},
            "sideEffectClass":"read_only","overlapKeys":["control-gap"],
            "falsificationCondition":"a verified independent control exists","stopCondition":"no new verified control"}).to_string());
    }
    changed_fact_response(request, true)
}

#[test]
fn coordinator_actual_independent_reviewer_feedback_controls_bounded_gap_dispatch_and_replay() {
    for investigate in [false, true] {
        let _real = RealSpecialistTransport::enter();
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
            (
                200,
                "application/json",
                changed_fact_reviewer_response(&request, investigate),
            )
        }));
        let mut f = root_tick_fixture_from_web_mode_turns(
            "changed-fact-reviewer",
            &format!("http://127.0.0.1:{port}/v1"),
            true, (60_000, 20), None,
            web_mode_fixture("multi", "https://authorized.example.test"), Some(1),
        );
        drop(f.parent.take());
        let mut session = multi_agent_prepare(&mut f.context).unwrap();
        stage_agent_finding(
            &f.context,
            AGENT_VULNERABILITY_STAGE,
            "vulnerability",
            "unverified-claim",
            "unverified fixture claim",
            "info",
            &json!({"claim":"requires independent control"}),
        )
        .unwrap();
        let outcome = AgentTargetOutcome::incomplete("no verified target control");
        let original_web = run_native_agent(&f.context);
        assert!(matches!(original_web, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{original_web:?}");
        multi_agent_finish_execution(&f.context, &mut session, &original_web).unwrap();
        coordinator_finished_web_replay_spends_zero(&f.context, &mut session, &original_web, || seen.lock().unwrap().len());
        let usage_db = db::open(&f.context.db_path).unwrap();
        let usage: (i64, i64, i64) = usage_db.query_row(
            "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
            [&session.executor.run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(usage, (20, 0, 1));
        assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(), 1);
        let reviewed = multi_agent_review(&f.context, &mut session, outcome);
        let db = db::open(&f.context.db_path).unwrap();
        let mut phases = vec!["bootstrap", "mapper-output", "budget-allocation", "budget-consumption", "reviewer-decision"];
        if investigate { phases.push("investigator-proposal"); }
        let assessment = coordinator_original_budget_assessment_count(&db, &f.actor.root_run_id, &seen.lock().unwrap(), &phases);
        assert_eq!(seen.lock().unwrap().len() - assessment,if investigate {9}else {7},
            "original dispatch/review SDK calls excluding the separately proven budget advisory: {reviewed:?}");
        assert_eq!(
            root_tick_count(&db, &f.actor.root_run_id, "publication") - assessment as i64,
            if investigate { 5 } else { 4 }
        );
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                Some(""),
                "model_requests"
            )
            .unwrap()
            .consumed - assessment as i64,
            if investigate { 5 } else { 4 }
        );
        let row: (String, i64, i64) = db
            .query_row(
                "SELECT (SELECT verdict FROM agent_review_decisions),
            (SELECT COUNT(*) FROM agent_assignments WHERE role='deep_investigator'),
            (SELECT COUNT(*) FROM sentinel_findings WHERE kind='vulnerability')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            ("insufficient_evidence".into(), i64::from(investigate), 0)
        );
        let (candidate, revision, missing): (String, i64, String) = db
            .query_row(
                "SELECT candidate_id,candidate_revision,missing_evidence_json
            FROM agent_review_decisions",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let missing: JsonValue = serde_json::from_str(&missing).unwrap();
        multi_agent_investigate_review_gap(&f.context, &session, &candidate, revision, &missing)
            .unwrap();
        assert_eq!(
            seen.lock().unwrap().len() - assessment,
            if investigate { 9 } else { 7 },
            "unchanged Reviewer fact cannot negotiate/execute again"
        );
        if investigate {
            let task: String = db
                .query_row(
                    "SELECT task_slice_json FROM agent_assignments WHERE role='deep_investigator'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<JsonValue>(&task).unwrap()["rootDecision"]["step"],
                "dispatch:deep_investigator"
            );
            let task: JsonValue = serde_json::from_str(&task).unwrap();
            let rust = &task["rootDecision"]["rustPolicy"];
            assert_eq!(rust["trigger"], "reviewer_decision");
            assert_eq!(rust["mergedProposalCount"], 1);
            assert_eq!(rust["features"]["sideEffectRisk"], 0);
            assert_eq!(rust["features"]["targetRequestCost"], 0);
            assert_eq!(rust["costBasis"]["reservedModelTokens"], 4000);
            assert_eq!(rust["costBasis"]["reservedModelRequests"], 1);
            assert!(rust["score"]
                .as_i64()
                .is_some_and(|n| (1..=100).contains(&n)));
            assert_eq!(rust["targetEvidenceProven"], false);
        }
        drop(session);
    }
}
