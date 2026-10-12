// Explicit Phase 2 execution branches (target_touching + review): every case
// drives the real production channels — coordinator apply turns, the execution
// carrier, run_target's three-side broker, the paid review transport and the
// immutable signed fact store. No hand-written receipts, no direct state UPDATE
// in place of a production transition (fixture mutations are limited to
// operator-owned inputs and lease expiry, matching existing negative suites).

#[cfg(unix)]
mod homogeneous_execution_support {
    use super::*;
    use crate::agent_runtime::multi_agent::homogeneous as h;
    use crate::tool_supply::frozen_plan::tests::TestSupplyFixture;

    pub(super) const CONTROL_KEY: &str = "idor|/api/orders";

    // Two validated identities bound through the actual draft creator; the
    // birth fixture then registers the frozen "idor|/api/orders" control group.
    pub(super) fn sessions_for_target(
        db: &rusqlite::Connection,
        target: &str,
    ) -> Option<(Vec<String>, String)> {
        let scope = Uuid::new_v4().to_string();
        for (id, cookie, authorization) in
            [("session-a", "cookie-a", "Bearer a"), ("session-b", "cookie-b", "Bearer b")]
        {
            let document = json!({
                "schemaVersion": 1, "id": id, "projectId": 9001, "entryUrl": target,
                "scopeHosts": ["127.0.0.1"], "cookies": [{"name": "session", "value": cookie}],
                "headers": {"authorization": authorization},
                "identityIsolation": {"sharedWithOtherSessions": false},
            });
            db.execute(
                "INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url,status,session_json,expires_at) \
                 VALUES(?1,9001,?2,?1,?3,'valid',?4,?5)",
                params![
                    id,
                    scope,
                    target,
                    document.to_string(),
                    (chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339()
                ],
            )
            .unwrap();
        }
        Some((vec!["session-a".into(), "session-b".into()], scope))
    }

    pub(super) struct ExecutionFixture {
        pub(super) fixture: RootTickFixture,
        pub(super) supply: TestSupplyFixture,
        pub(super) db: rusqlite::Connection,
        pub(super) task: h::types::HomogeneousTask,
        pub(super) target: String,
    }

    pub(super) fn live_target_fixture(
        make_supply: impl FnOnce(&Path) -> Result<TestSupplyFixture, String>,
        target_port: u16,
    ) -> ExecutionFixture {
        let target = format!("http://127.0.0.1:{target_port}");
        let (mut fixture, supply, _) = frozen_tool_supply_original_birth_with_task(
            Some(true),
            make_supply,
            true,
            &target,
            |db, _path, target| sessions_for_target(db, target),
        );
        fixture.context.identities = vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ];
        let db = db::open(&fixture.context.db_path).unwrap();
        // Production scans have observed evidence before the coordinator task
        // activates; the revision base drives every execution-lane evidence
        // write, so seed one observed revision exactly as activation reads it.
        db.execute(
            "INSERT INTO agent_evidence_revisions(root_run_id,revision,parent_revision) VALUES(?1,1,NULL)",
            [&fixture.actor.root_run_id],
        )
        .unwrap();
        h::store::set_run_scheduling_model(
            &db,
            &fixture.actor.root_run_id,
            h::types::SCHEDULING_HOMOGENEOUS,
        )
        .unwrap();
        let task_id = h::store::activate_task_for_lease(&db, &fixture.actor)
            .unwrap()
            .unwrap();
        let task = h::store::load_task(&db, &task_id).unwrap().unwrap();
        ExecutionFixture { fixture, supply, db, task, target }
    }

    /// Same birth path plus a second operator-registered control group, for
    /// exclusivity cases that need two independently drafted target slices.
    pub(super) fn live_two_control_fixture(
        make_supply: impl FnOnce(&Path) -> Result<TestSupplyFixture, String>,
        target_port: u16,
    ) -> ExecutionFixture {
        let target = format!("http://127.0.0.1:{target_port}");
        let second = SaveAuthorizationControlInput {
            scan_id: String::new(),
            attempt_number: 1,
            target_url: target.clone(),
            contract_key: "idor|/api/users".into(),
            owner_object_url: format!("{target}/api/users?id=owner"),
            tester_control_url: format!("{target}/api/users?id=tester"),
            object_query_key: "id".into(),
            owner_object_value: "owner".into(),
            tester_object_value: "tester".into(),
            response_object_pointer: "/order/id".into(),
            owner_identity: "session-a".into(),
            tester_identity: "session-b".into(),
        };
        let (mut fixture, supply, _) = frozen_tool_supply_original_birth_with_task_controls(
            Some(true),
            make_supply,
            true,
            &target,
            |db, _path, target| sessions_for_target(db, target),
            &[second],
        );
        fixture.context.identities = vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ];
        let db = db::open(&fixture.context.db_path).unwrap();
        db.execute(
            "INSERT INTO agent_evidence_revisions(root_run_id,revision,parent_revision) VALUES(?1,1,NULL)",
            [&fixture.actor.root_run_id],
        )
        .unwrap();
        h::store::set_run_scheduling_model(
            &db,
            &fixture.actor.root_run_id,
            h::types::SCHEDULING_HOMOGENEOUS,
        )
        .unwrap();
        let task_id = h::store::activate_task_for_lease(&db, &fixture.actor).unwrap().unwrap();
        let task = h::store::load_task(&db, &task_id).unwrap().unwrap();
        ExecutionFixture { fixture, supply, db, task, target }
    }

    impl ExecutionFixture {
        pub(super) fn provision(&self, count: i64) -> Vec<String> {
            h::pool::provision_workers(&self.db, &self.task, self.fixture.actor.lease_epoch, count)
                .unwrap()
        }

        pub(super) fn target_slice_id(&self) -> String {
            h::decompose::slice_id_for(
                &self.task.task_id,
                "asset_id",
                &format!("target-control:{CONTROL_KEY}"),
                self.task.evidence_revision_base,
            )
        }

        pub(super) fn review_slice_id(&self, candidate_fact_id: &str) -> String {
            h::decompose::slice_id_for(
                &self.task.task_id,
                "asset_id",
                &format!("review-candidate:{candidate_fact_id}"),
                self.task.evidence_revision_base,
            )
        }

        pub(super) fn turn(
            &self,
            round: usize,
            action: &str,
            payload: JsonValue,
            refs: JsonValue,
        ) -> h::apply::TurnOutcome {
            h::apply::run_coordinator_turn(
                &self.db,
                &self.fixture.actor,
                &homogeneous_product_envelope(round, &self.task.task_id, action, payload, refs)
                    .to_string(),
            )
            .expect("一轮不应失败")
        }

        /// Coordinator accept of a reported execution-lane assignment, including
        /// the mandatory zero-intent justification while it is still inflight.
        pub(super) fn accept_turn(
            &self,
            round: usize,
            assignment: &str,
            slice_id: &str,
        ) -> h::apply::TurnOutcome {
            let row = h::store::load_assignment(&self.db, assignment).unwrap().unwrap();
            let slice = h::store::load_slice(&self.db, slice_id).unwrap().unwrap();
            let report = h::store::load_report(
                &self.db,
                &h::store::report_id_for(assignment, row.attempt),
            )
            .unwrap()
            .unwrap();
            let checks =
                h::machine::recompute(&self.db, &self.task, &row, &slice, &report);
            assert!(checks.all_true(), "{checks:?}");
            let mut envelope = homogeneous_product_envelope(
                round,
                &self.task.task_id,
                "accept",
                json!({
                    "assignment_id": assignment,
                    "slice_id": slice_id,
                    "report_id": report.report_id,
                    "verdict": "accept",
                    "machine_checks": serde_json::to_value(&checks).unwrap(),
                    "model_judgement": {"adequate": true,
                        "question_zh": "三侧原文与冻结控制组是否一致？",
                        "answer_zh": "一致，且签名事实可验。"},
                    "reason_codes": ["goal_reached"],
                    "evidence_refs": report.evidence_refs
                }),
                json!({
                    "slice_ids": [slice_id], "worker_ids": [], "tool_ids": [],
                    "target_keys": [], "directive_ids": [], "assignment_ids": [assignment]
                }),
            );
            envelope["zero_intent"] = json!({
                "reason": "covered_by_inflight",
                "evidence_assignment_ids": [assignment],
                "note_zh": "主管先验收已汇报的执行事实，再决定下一步。"
            });
            h::apply::run_coordinator_turn(&self.db, &self.fixture.actor, &envelope.to_string())
                .expect("accept 一轮不应失败")
        }

        pub(super) fn target_assignment_id(&self, slice_id: &str) -> String {
            h::store::assignment_id_for(
                &self.task.root_run_id,
                &h::store::dedup_key_for(
                    slice_id,
                    &self.task.target_key,
                    self.task.evidence_revision_base,
                    h::store::HOMOGENEOUS_TRIGGER_CODE,
                ),
            )
        }

        pub(super) fn ledger(&self) -> (i64, i64, i64, i64) {
            self.db
                .query_row(
                    "SELECT reserved_tokens,reserved_requests,spent_tokens,spent_requests \
                     FROM agent_budget_ledger WHERE root_run_id=?1",
                    [&self.task.root_run_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap()
        }

        pub(super) fn run_target(&self, assignment: &str) -> Result<(), String> {
            crate::commands::homogeneous_execution::run_target(
                &self.fixture.context,
                &self.fixture.actor,
                assignment,
            )
        }
    }
}
#[cfg(unix)]
use homogeneous_execution_support::{live_target_fixture, live_two_control_fixture, sessions_for_target, CONTROL_KEY};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn target_branch_executes_registered_three_side_gets_and_freezes_signed_fact() {
    use crate::agent_runtime::multi_agent::homogeneous as h;
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, target_seen, _stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let first = wire.lines().next().unwrap_or_default();
        let object = if first.contains("id=tester") { "tester" } else { "owner" };
        let body = format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#);
        (200, "application/json", body)
    }));
    let fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    let workers = fixture.provision(2);
    let slice_id = fixture.target_slice_id();
    // decompose must only materialize the production-registered target slice.
    let decompose = fixture.turn(
        1,
        "decompose",
        json!({"slices":[{
            "slice_id": slice_id,
            "scope_kind": "asset_id",
            "scope_value": format!("target-control:{CONTROL_KEY}"),
            "lane": "target_touching",
            "evidence_revision_base": fixture.task.evidence_revision_base,
            "worker_capability_profile_id": "homogeneous_v1",
            "max_retries": 1
        }]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [],
            "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    );
    assert!(decompose.applied(), "{decompose:?}");
    let assign = fixture.turn(
        2,
        "assign",
        json!({
            "slice_id": slice_id, "worker_id": workers[0],
            "reserved_tokens": 1, "reserved_requests": 0,
            "capability_lease": ["authorization_probe"],
            "lane": "target_touching", "attempt": 1
        }),
        json!({"slice_ids": [slice_id], "worker_ids": [workers[0]],
            "tool_ids": ["authorization_probe"],
            "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    );
    assert!(assign.applied(), "{assign:?}");
    let assignment = fixture.target_assignment_id(&slice_id);
    fixture.run_target(&assignment).unwrap();
    let seen = target_seen.lock().unwrap().clone();
    assert_eq!(
        seen.iter().map(|wire| wire.lines().next().unwrap_or_default().split(' ').nth(1).unwrap_or_default()).collect::<Vec<_>>(),
        vec!["/api/orders?id=owner", "/api/orders?id=owner", "/api/orders?id=tester"],
        "the wire must carry exactly the three registered GETs"
    );
    let db = &fixture.db;
    let row = h::store::load_assignment(db, &assignment).unwrap().unwrap();
    assert_eq!(row.state, "reported");
    assert_eq!(row.lane, "target_touching");
    let report = h::store::load_report(db, &h::store::report_id_for(&assignment, row.attempt))
        .unwrap()
        .unwrap();
    assert_eq!(report.terminal_hint, "completed");
    assert_eq!((report.consumed_tokens, report.consumed_requests), (0, 0));
    // The frozen signed fact: three sides, verified, and readable via the
    // production verifier (hash + signature + scope) under a snapshot.
    let (raw, kind): (String, String) = db
        .query_row(
            "SELECT fact_json,kind FROM agent_homogeneous_execution_facts WHERE assignment_id=?1",
            [&assignment],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "target");
    let fact: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(fact["detail"]["verified"], true);
    assert_eq!(fact["detail"]["reason"], JsonValue::Null);
    let sides = fact["detail"]["sides"].as_array().unwrap();
    assert_eq!(sides.len(), 3);
    assert_eq!(sides[0]["name"], "owner");
    assert_eq!(sides[1]["name"], "cross");
    assert_eq!(sides[2]["name"], "tester");
    for side in sides {
        assert_eq!(side["status"], 200);
        assert_eq!(side["cacheState"], "miss");
        assert_eq!(side["redirectCode"], "");
        assert!(side["receiptId"].as_str().is_some_and(|v| !v.is_empty()));
        assert!(side["artifact"]["bodySha256"].as_str().is_some_and(|v| !v.is_empty()));
    }
    let tx = db.unchecked_transaction().unwrap();
    let verified = crate::commands::homogeneous_execution::read_fact(
        &tx,
        &fixture.fixture.actor,
        fact["id"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(verified, fact);
    tx.rollback().unwrap();
    let receipts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_authorization_side_receipts WHERE root_run_id=?1",
            [&fixture.fixture.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 3, "each side keeps its signed original receipt");
    // The exclusive lane is still held until the supervisor's verdict.
    let active_leases: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at=''",
            [&assignment],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active_leases, 1);
    let lane_leases: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&assignment],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lane_leases, 1);
    let accepted = fixture.accept_turn(3, &assignment, &slice_id);
    assert!(accepted.applied(), "{accepted:?}");
    assert_eq!(
        h::store::load_assignment(db, &assignment).unwrap().unwrap().state,
        "accepted"
    );
    // Verdict releases every exclusive resource the carrier acquired.
    let open_leases: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at=''",
            [&assignment],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open_leases, 0, "capability leases must all be revoked");
    let lane_leases: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&assignment],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lane_leases, 0, "lane lease must be removed");
    let released: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_slots WHERE root_run_id=?1 AND slice_id=?2 AND released_at<>''",
            [&fixture.task.root_run_id, &slice_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(released, 1);
    assert_eq!(fixture.ledger(), (0, 0, 0, 0), "target lane settles at zero fee");
    let usage = agent_request_accounting(
        db,
        &fixture.fixture.actor.scan_id,
        fixture.fixture.actor.attempt_number,
        &fixture.target,
    )
    .unwrap();
    assert_eq!((usage.authorization.received, usage.authorization.unresolved), (3, 0));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn review_branch_independent_paid_reviewer_confirms_and_publishes_single_finding() {
    use crate::agent_runtime::multi_agent::homogeneous as h;
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    // One multiplexed endpoint: target GETs by object id, the reviewer's paid
    // model round, and the parser worker's paid model round.
    let approval_cell = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let (port, wire_seen, _stop) = spawn_endpoint({
        let approval_cell = approval_cell.clone();
        std::sync::Arc::new(move |wire| {
            let first = wire.lines().next().unwrap_or_default();
            if first.starts_with("GET ") {
                let object = if first.contains("id=tester") { "tester" } else { "owner" };
                let body = format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#);
                return (200, "application/json", body);
            }
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            if input.get("candidate_bundle").is_some() {
                let review = json!({"verdict":"confirmed","reason_codes":["cross_object_read_reproduced","three_side_bundle_intact"],"missing_evidence":[],"confidence":0.97,"summary_zh":"三侧原文与冻结控制组一致，跨账号读取成立"});
                (
                    200,
                    "application/json",
                    proposal_model_response(&review.to_string()),
                )
            } else {
                let approval = approval_cell.lock().unwrap().clone();
                let refs: Vec<_> = input["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["evidence_ref"].clone())
                    .collect();
                let response = json!({"output":{"summary_zh":"按原材料生成只读解析辅助代码","findings":[],"coverage_notes":["解析不等于目标验证"]},"evidence_refs":refs,"terminal_hint":"completed","tool_request":{"name":"source_analysis","approval_id":approval,"program":"def analyze(data):\n return data","evidence_refs":refs}});
                (
                    200,
                    "application/json",
                    proposal_model_response(&response.to_string()),
                )
            }
        })
    });
    let mut fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    *approval_cell.lock().unwrap() = fixture.supply.approval_id().to_string();
    fixture.fixture.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    fixture.fixture.context.environment.api_key = "mock-key".into();
    let workers = fixture.provision(3);
    let transport = SpecialistTransportContext {
        supervision: None,
        db_path: &fixture.fixture.context.db_path,
        scan_id: &fixture.fixture.actor.scan_id,
        attempt_number: fixture.fixture.actor.attempt_number,
        target_key: &fixture.fixture.actor.target_key,
        run_id: &fixture.fixture.actor.root_run_id,
        environment: &fixture.fixture.context.environment,
        proxy: None,
        usage_dir: &fixture.fixture.context.target_dir,
        deadline: None,
    };
    // Lane 1: registered three-side target execution, accepted by the supervisor.
    let target_slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1,
        "decompose",
        json!({"slices":[{"slice_id":target_slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        2,
        "assign",
        json!({"slice_id":target_slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[target_slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let target_assignment = fixture.target_assignment_id(&target_slice);
    fixture.run_target(&target_assignment).unwrap();
    fixture.accept_turn(3, &target_assignment, &target_slice);
    let target_fact_row: String = fixture.db
        .query_row("SELECT fact_json FROM agent_homogeneous_execution_facts WHERE assignment_id=?1 AND kind='target'",[&target_assignment],|r|r.get(0))
        .unwrap();
    let target_fact: JsonValue = serde_json::from_str(&target_fact_row).unwrap();
    let target_fact_id = target_fact["id"].as_str().unwrap().to_string();
    // The accepted target fact automatically produces the review-candidate draft.
    let review_slice = fixture.review_slice_id(&target_fact_id);
    assert!(
        crate::commands::homogeneous_execution::drafts(&fixture.db, &fixture.task)
            .unwrap()
            .iter()
            .any(|x| x.slice_id == review_slice),
        "accepted target must open exactly its review-candidate draft"
    );
    // Lane 2: independent paid reviewer (a different worker), confirmed.
    assert!(fixture.turn(
        4,
        "decompose",
        json!({"slices":[{"slice_id":review_slice,"scope_kind":"asset_id","scope_value":format!("review-candidate:{target_fact_id}"),"lane":"review","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        5,
        "assign",
        json!({"slice_id":review_slice,"worker_id":workers[1],"reserved_tokens":200000,"reserved_requests":1,"capability_lease":["evidence.read","review.write"],"lane":"review","attempt":1}),
        json!({"slice_ids":[review_slice],"worker_ids":[workers[1]],"tool_ids":["evidence.read","review.write"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let review_assignment = fixture.target_assignment_id(&review_slice);
    native_homogeneous_model_worker(
        &transport,
        &fixture.fixture.actor,
        &review_assignment,
        &std::sync::Mutex::new(()),
        Some(fixture.supply.configuration()),
    )
    .unwrap();
    assert_eq!(h::store::load_assignment(&fixture.db, &review_assignment).unwrap().unwrap().state, "reported");
    fixture.accept_turn(6, &review_assignment, &review_slice);
    assert_eq!(h::store::load_assignment(&fixture.db, &review_assignment).unwrap().unwrap().state, "accepted");
    // Lane 3: paid original-SDK parser inside the real sandbox, accepted.
    {
        use crate::agent_runtime::{
            evidence_graph::{contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance}, store::{evidence_natural_key, insert_evidence_node}},
            store::stable_hash,
        };
        let identity = "review chain fixture frontend data";
        let id = stable_hash(identity);
        let tx = fixture.db.unchecked_transaction().unwrap();
        insert_evidence_node(&tx,&EvidenceNode{id:id.clone(),root_run_id:fixture.task.root_run_id.clone(),revision:1,kind:EvidenceNodeKind::Endpoint,provenance:EvidenceProvenance::Observed,natural_key_hash:evidence_natural_key(&fixture.task.root_run_id,EvidenceNodeKind::Endpoint,identity),payload:json!({"url":format!("{}/api/orders",fixture.target),"path":"/api/orders","Cookie":"session=fixture-account-a","Token":"fixture-account-b","password":"fixture-password","target_request_executed":false}),artifact_refs:vec![],created_by_run_id:fixture.task.root_run_id.clone(),supersedes_id:String::new(),created_at:String::new()}).unwrap();
        tx.commit().unwrap();
    }
    let parser_slice = h::decompose::slice_id_for(&fixture.task.task_id,"path_prefix","/api/orders",fixture.task.evidence_revision_base);
    assert!(fixture.turn(
        7,
        "decompose",
        json!({"slices":[{"slice_id":parser_slice,"scope_kind":"path_prefix","scope_value":"/api/orders","lane":"read_only_analysis","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        8,
        "assign",
        json!({"slice_id":parser_slice,"worker_id":workers[2],"reserved_tokens":20000,"reserved_requests":1,"capability_lease":["evidence.read","source_analysis"],"lane":"read_only_analysis","attempt":1}),
        json!({"slice_ids":[parser_slice],"worker_ids":[workers[2]],"tool_ids":["evidence.read","source_analysis"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let parser_assignment = fixture.target_assignment_id(&parser_slice);
    native_homogeneous_model_worker(
        &transport,
        &fixture.fixture.actor,
        &parser_assignment,
        &std::sync::Mutex::new(()),
        Some(fixture.supply.configuration()),
    )
    .unwrap();
    assert_eq!(h::store::load_assignment(&fixture.db, &parser_assignment).unwrap().unwrap().state, "reported");
    fixture.accept_turn(9, &parser_assignment, &parser_slice);
    // Goal gate: three lanes accepted, exactly one review per control.
    assert!(crate::commands::homogeneous_execution::goal_ready(&fixture.db, &fixture.task).unwrap());
    // The single publication: one confirmed cross-object finding for this scan.
    let findings: i64 = fixture.db
        .query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND record_key=?2",
            [&fixture.task.scan_id, &format!("homogeneous-authorization:{target_fact_id}")],|r|r.get(0))
        .unwrap();
    assert_eq!(findings, 1, "confirmed review must publish exactly one finding");
    let review_fee: i64 = fixture.db
        .query_row("SELECT COUNT(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND dimension='model_requests' AND kind='consume'",
            [&fixture.task.root_run_id, &review_assignment],|r|r.get(0))
        .unwrap();
    assert_eq!(review_fee, 1, "the reviewer's paid round is consumed exactly once");
    // stop(goal_reached) closes the task through the production intent gate.
    let stop = fixture.turn(
        10,
        "stop",
        json!({"stop_reason":"goal_reached","accepted_slice_ids":[target_slice,review_slice,parser_slice],"rejected_slice_ids":[],"summary_zh":"三侧执行、独立复核与沙箱解析全部验收","publish_partial":false}),
        json!({"slice_ids":[target_slice,review_slice,parser_slice],"worker_ids":[],"tool_ids":[],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    );
    assert!(stop.applied(), "{stop:?}");
    let final_task = h::store::load_task_by_root_run(&fixture.db, &fixture.task.root_run_id)
        .unwrap()
        .unwrap();
    assert!(matches!(final_task.state.as_str(), "succeeded" | "stopped"), "{}", final_task.state);
    let wire = wire_seen.lock().unwrap().clone();
    let model_posts = wire.iter().filter(|w| w.lines().next().unwrap_or_default().starts_with("POST ")).count();
    assert_eq!(model_posts, 2, "exactly the reviewer round and the parser round are paid");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn reviewer_same_as_producer_or_unaccepted_candidate_is_refused() {
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let first = wire.lines().next().unwrap_or_default();
        let object = if first.contains("id=tester") { "tester" } else { "owner" };
        (200, "application/json", format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#))
    }));
    let fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    let workers = fixture.provision(2);
    let target_slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1, "decompose",
        json!({"slices":[{"slice_id":target_slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        2, "assign",
        json!({"slice_id":target_slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[target_slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let target_assignment = fixture.target_assignment_id(&target_slice);
    fixture.run_target(&target_assignment).unwrap();
    let target_fact_row: String = fixture.db
        .query_row("SELECT fact_json FROM agent_homogeneous_execution_facts WHERE assignment_id=?1 AND kind='target'",[&target_assignment],|r|r.get(0))
        .unwrap();
    let target_fact: JsonValue = serde_json::from_str(&target_fact_row).unwrap();
    let target_fact_id = target_fact["id"].as_str().unwrap().to_string();
    let review_slice = fixture.review_slice_id(&target_fact_id);
    // An unaccepted target opens no review draft at all: the slice cannot even
    // be materialized through decompose.
    let premature = fixture.turn(
        3, "decompose",
        json!({"slices":[{"slice_id":review_slice,"scope_kind":"asset_id","scope_value":format!("review-candidate:{target_fact_id}"),"lane":"review","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    );
    assert!(
        format!("{premature:?}").contains("homogeneous_execution_slice_not_in_original_candidates"),
        "{premature:?}"
    );
    fixture.accept_turn(4, &target_assignment, &target_slice);
    assert!(fixture.turn(
        5, "decompose",
        json!({"slices":[{"slice_id":review_slice,"scope_kind":"asset_id","scope_value":format!("review-candidate:{target_fact_id}"),"lane":"review","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    // The producer itself may not review its own target fact.
    let same_worker = fixture.turn(
        6, "assign",
        json!({"slice_id":review_slice,"worker_id":workers[0],"reserved_tokens":200000,"reserved_requests":1,"capability_lease":["evidence.read","review.write"],"lane":"review","attempt":1}),
        json!({"slice_ids":[review_slice],"worker_ids":[workers[0]],"tool_ids":["evidence.read","review.write"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    );
    assert!(
        format!("{same_worker:?}").contains("homogeneous_reviewer_must_be_independent"),
        "{same_worker:?}"
    );
    // An independent worker may take it; the independence floor is the only
    // refusal here (positive control for the assign gate).
    assert!(fixture.turn(
        7, "assign",
        json!({"slice_id":review_slice,"worker_id":workers[1],"reserved_tokens":200000,"reserved_requests":1,"capability_lease":["evidence.read","review.write"],"lane":"review","attempt":1}),
        json!({"slice_ids":[review_slice],"worker_ids":[workers[1]],"tool_ids":["evidence.read","review.write"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn target_branch_without_control_group_or_wrong_scope_is_refused() {
    use crate::agent_runtime::multi_agent::homogeneous as h;
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_wire| {
        (200, "application/json", r#"{"order":{"id":"owner"}}"#.to_string())
    }));
    // Operator input path: the identities exist but the control group is
    // never saved through save_authorization_control_in.
    let (mut fixture, _supply, _) = frozen_tool_supply_original_birth_with_task(
        Some(true),
        |path| TestSupplyFixture::approve_container_on(path, runtime.digest()),
        true,
        &format!("http://127.0.0.1:{port}"),
        |db, _path, _target| {
            sessions_for_target(db, _target);
            None
        },
    );
    fixture.context.identities = vec![
        AgentIdentity::scoped("session-a"),
        AgentIdentity::scoped("session-b"),
    ];
    let db = db::open(&fixture.context.db_path).unwrap();
    h::store::set_run_scheduling_model(&db, &fixture.actor.root_run_id, h::types::SCHEDULING_HOMOGENEOUS).unwrap();
    let task_id = h::store::activate_task_for_lease(&db, &fixture.actor).unwrap().unwrap();
    let task = h::store::load_task(&db, &task_id).unwrap().unwrap();
    let _worker = h::pool::provision_workers(&db, &task, fixture.actor.lease_epoch, 1).unwrap()[0].clone();
    let registered: i64 = db
        .query_row("SELECT count(*) FROM agent_authorization_controls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(registered, 0, "fixture must not register any control group");
    // With no registered control group there is no target draft at all, so the
    // slice cannot even be materialized through decompose.
    let slice = h::decompose::slice_id_for(&task.task_id, "asset_id", &format!("target-control:{CONTROL_KEY}"), task.evidence_revision_base);
    let refused = h::apply::run_coordinator_turn(&db, &fixture.actor, &homogeneous_product_envelope(
        1, &task.task_id, "decompose",
        json!({"slices":[{"slice_id":slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.actor.target_key], "directive_ids": [], "assignment_ids": []}),
    ).to_string()).unwrap();
    assert!(
        format!("{refused:?}").contains("homogeneous_execution_slice_not_in_original_candidates"),
        "{refused:?}"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn target_lane_paid_request_lease_is_refused_by_contract() {
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_wire| {
        (200, "application/json", r#"{"order":{"id":"owner"}}"#.to_string())
    }));
    let fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    let workers = fixture.provision(1);
    let slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1, "decompose",
        json!({"slices":[{"slice_id":slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    // The target lane's contract is "zero paid model rounds": a dispatch with
    // a reserved request is refused before any three-side execution.
    let bad_lease = fixture.turn(
        2, "assign",
        json!({"slice_id":slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":1,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    );
    assert!(
        format!("{bad_lease:?}").contains("homogeneous_target_lease_contract_invalid"),
        "{bad_lease:?}"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn unverified_target_never_confirms_or_publishes() {
    
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    // The server scopes correctly: the tester identity reading the owner URL
    // receives its own object, so the control group cannot verify. The paid
    // review round still answers with a well-formed usage-bearing receipt.
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let first = wire.lines().next().unwrap_or_default();
        if first.starts_with("POST ") {
            let review = json!({"verdict":"confirmed","reason_codes":["cross_object_read_reproduced"],"missing_evidence":[],"confidence":0.97,"summary_zh":"按原文确认"});
            return (200, "application/json", proposal_model_response(&review.to_string()));
        }
        let tester_reads_owner = wire.contains("cookie: session=cookie-b") && wire.contains("id=owner");
        let object = if tester_reads_owner || wire.contains("id=tester") { "tester" } else { "owner" };
        (200, "application/json", format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#))
    }));
    let mut fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    fixture.fixture.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    fixture.fixture.context.environment.api_key = "mock-key".into();
    let workers = fixture.provision(2);
    let target_slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1, "decompose",
        json!({"slices":[{"slice_id":target_slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        2, "assign",
        json!({"slice_id":target_slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[target_slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let target_assignment = fixture.target_assignment_id(&target_slice);
    fixture.run_target(&target_assignment).unwrap();
    let target_fact_row: String = fixture.db
        .query_row("SELECT fact_json FROM agent_homogeneous_execution_facts WHERE assignment_id=?1 AND kind='target'",[&target_assignment],|r|r.get(0))
        .unwrap();
    let target_fact: JsonValue = serde_json::from_str(&target_fact_row).unwrap();
    assert_eq!(target_fact["detail"]["verified"], false);
    assert_ne!(target_fact["detail"]["reason"], JsonValue::Null);
    fixture.accept_turn(3, &target_assignment, &target_slice);
    let target_fact_id = target_fact["id"].as_str().unwrap().to_string();
    let review_slice = fixture.review_slice_id(&target_fact_id);
    assert!(fixture.turn(
        4, "decompose",
        json!({"slices":[{"slice_id":review_slice,"scope_kind":"asset_id","scope_value":format!("review-candidate:{target_fact_id}"),"lane":"review","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        5, "assign",
        json!({"slice_id":review_slice,"worker_id":workers[1],"reserved_tokens":200000,"reserved_requests":1,"capability_lease":["evidence.read","review.write"],"lane":"review","attempt":1}),
        json!({"slice_ids":[review_slice],"worker_ids":[workers[1]],"tool_ids":["evidence.read","review.write"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let review_assignment = fixture.target_assignment_id(&review_slice);
    let transport = SpecialistTransportContext {
        supervision: None,
        db_path: &fixture.fixture.context.db_path,
        scan_id: &fixture.fixture.actor.scan_id,
        attempt_number: fixture.fixture.actor.attempt_number,
        target_key: &fixture.fixture.actor.target_key,
        run_id: &fixture.fixture.actor.root_run_id,
        environment: &fixture.fixture.context.environment,
        proxy: None,
        usage_dir: &fixture.fixture.context.target_dir,
        deadline: None,
    };
    let refused = native_homogeneous_model_worker(
        &transport,
        &fixture.fixture.actor,
        &review_assignment,
        &std::sync::Mutex::new(()),
        Some(fixture.supply.configuration()),
    )
    .unwrap_err();
    assert!(
        refused.contains("homogeneous_review_confirmation_without_controls"),
        "{refused}"
    );
    let findings: i64 = fixture.db
        .query_row("SELECT count(*) FROM sentinel_findings WHERE scan_id=?1", [&fixture.task.scan_id], |r| r.get(0))
        .unwrap();
    assert_eq!(findings, 0, "an unverified target must never publish");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn tampered_fact_or_sdk_receipt_blocks_review_acceptance() {
    
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let first = wire.lines().next().unwrap_or_default();
        if first.starts_with("GET ") {
            let object = if first.contains("id=tester") { "tester" } else { "owner" };
            return (200, "application/json", format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#));
        }
        let review = json!({"verdict":"confirmed","reason_codes":["cross_object_read_reproduced"],"missing_evidence":[],"confidence":0.97,"summary_zh":"三侧原文与冻结控制组一致"});
        (200, "application/json", proposal_model_response(&review.to_string()))
    }));
    let mut fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    fixture.fixture.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    fixture.fixture.context.environment.api_key = "mock-key".into();
    let workers = fixture.provision(2);
    let target_slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1, "decompose",
        json!({"slices":[{"slice_id":target_slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        2, "assign",
        json!({"slice_id":target_slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[target_slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let target_assignment = fixture.target_assignment_id(&target_slice);
    fixture.run_target(&target_assignment).unwrap();
    fixture.accept_turn(3, &target_assignment, &target_slice);
    let target_fact_row: String = fixture.db
        .query_row("SELECT fact_json FROM agent_homogeneous_execution_facts WHERE assignment_id=?1 AND kind='target'",[&target_assignment],|r|r.get(0))
        .unwrap();
    let target_fact: JsonValue = serde_json::from_str(&target_fact_row).unwrap();
    let target_fact_id = target_fact["id"].as_str().unwrap().to_string();
    // The signed fact store is append-only: an attacker's UPDATE on a frozen
    // fact is aborted by the immutability trigger.
    let rewrite = fixture.db.execute(
        "UPDATE agent_homogeneous_execution_facts SET fact_json='{}' WHERE id=?1",
        [&target_fact_id],
    );
    assert!(rewrite.is_err(), "execution facts must be immutable");
    let review_slice = fixture.review_slice_id(&target_fact_id);
    assert!(fixture.turn(
        4, "decompose",
        json!({"slices":[{"slice_id":review_slice,"scope_kind":"asset_id","scope_value":format!("review-candidate:{target_fact_id}"),"lane":"review","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        5, "assign",
        json!({"slice_id":review_slice,"worker_id":workers[1],"reserved_tokens":200000,"reserved_requests":1,"capability_lease":["evidence.read","review.write"],"lane":"review","attempt":1}),
        json!({"slice_ids":[review_slice],"worker_ids":[workers[1]],"tool_ids":["evidence.read","review.write"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let review_assignment = fixture.target_assignment_id(&review_slice);
    let transport = SpecialistTransportContext {
        supervision: None,
        db_path: &fixture.fixture.context.db_path,
        scan_id: &fixture.fixture.actor.scan_id,
        attempt_number: fixture.fixture.actor.attempt_number,
        target_key: &fixture.fixture.actor.target_key,
        run_id: &fixture.fixture.actor.root_run_id,
        environment: &fixture.fixture.context.environment,
        proxy: None,
        usage_dir: &fixture.fixture.context.target_dir,
        deadline: None,
    };
    native_homogeneous_model_worker(
        &transport,
        &fixture.fixture.actor,
        &review_assignment,
        &std::sync::Mutex::new(()),
        Some(fixture.supply.configuration()),
    )
    .unwrap();
    // The original SDK receipt is equally append-only: forging the response
    // hash on the specialist call is aborted by its immutability trigger, so
    // no later accept can be fed a rewritten receipt.
    let sdk_forge = fixture.db.execute("UPDATE agent_specialist_calls SET response_hash='0'", []);
    let sdk_forge_detail = format!("{sdk_forge:?}");
    assert!(
        sdk_forge.is_err() && sdk_forge_detail.contains("specialist_call_immutable"),
        "specialist calls must be immutable, got {sdk_forge_detail}"
    );
    let findings: i64 = fixture.db
        .query_row("SELECT count(*) FROM sentinel_findings WHERE scan_id=?1", [&fixture.task.scan_id], |r| r.get(0))
        .unwrap();
    assert_eq!(findings, 0, "no forged publication may exist");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn replay_or_expired_attempt_blocks_late_target_delivery() {
    
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, wire_seen, _stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let first = wire.lines().next().unwrap_or_default();
        let object = if first.contains("id=tester") { "tester" } else { "owner" };
        (200, "application/json", format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#))
    }));
    let fixture =
        live_target_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    let workers = fixture.provision(1);
    let target_slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1, "decompose",
        json!({"slices":[{"slice_id":target_slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        2, "assign",
        json!({"slice_id":target_slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[target_slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let target_assignment = fixture.target_assignment_id(&target_slice);
    fixture.run_target(&target_assignment).unwrap();
    // Replaying the same attempt can never re-send the registered sides.
    let replay = fixture.run_target(&target_assignment).unwrap_err();
    assert!(
        replay.contains("homogeneous_execution_assignment_state_invalid")
            || replay.contains("authorization_side_already_claimed"),
        "{replay}"
    );
    // After the carrier's attempt lease expires, late delivery is refused and
    // the wire still carries exactly the three registered GETs.
    fixture.db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'", []).unwrap();
    let expired = fixture.run_target(&target_assignment).unwrap_err();
    assert!(
        expired.contains("homogeneous_execution_assignment_state_invalid")
            || expired.contains("expired")
            || expired.contains("live"),
        "{expired}"
    );
    let gets = wire_seen.lock().unwrap().iter().filter(|w| w.lines().next().unwrap_or_default().starts_with("GET ")).count();
    assert_eq!(gets, 3, "no fourth target request may ever leave");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn lane_exclusivity_survives_timeout_reclaim() {
    use crate::agent_runtime::multi_agent::homogeneous as h;
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_wire| {
        (200, "application/json", r#"{"order":{"id":"owner"}}"#.to_string())
    }));
    let fixture =
        live_two_control_fixture(|path| TestSupplyFixture::approve_container_on(path, runtime.digest()), port);
    let workers = fixture.provision(2);
    // Exclusivity: while one target slice is live, a second target lane assign
    // is refused; after its carrier expires and is reclaimed, it is allowed.
    let first_slice = fixture.target_slice_id();
    assert!(fixture.turn(
        1, "decompose",
        json!({"slices":[{"slice_id":first_slice,"scope_kind":"asset_id","scope_value":format!("target-control:{CONTROL_KEY}"),"lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [fixture.target], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    assert!(fixture.turn(
        2, "assign",
        json!({"slice_id":first_slice,"worker_id":workers[0],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[first_slice],"worker_ids":[workers[0]],"tool_ids":["authorization_probe"],"target_keys":[fixture.target],"directive_ids":[],"assignment_ids":[]}),
    ).applied());
    let second_slice = h::decompose::slice_id_for(&fixture.task.task_id, "asset_id", "target-control:idor|/api/users", fixture.task.evidence_revision_base);
    assert!(fixture.turn(
        3, "decompose",
        json!({"slices":[{"slice_id":second_slice,"scope_kind":"asset_id","scope_value":"target-control:idor|/api/users","lane":"target_touching","evidence_revision_base":fixture.task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    ).applied());
    let exclusive = fixture.turn(
        4, "assign",
        json!({"slice_id":second_slice,"worker_id":workers[1],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[second_slice],"worker_ids":[workers[1]],"tool_ids":["authorization_probe"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    );
    assert!(format!("{exclusive:?}").contains("lane_quota_exceeded"), "{exclusive:?}");
    fixture.db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'", []).unwrap();
    h::pool::reclaim_expired_for_task(&fixture.db, &fixture.task).unwrap();
    let reclaimed = fixture.turn(
        5, "assign",
        json!({"slice_id":second_slice,"worker_id":workers[1],"reserved_tokens":1,"reserved_requests":0,"capability_lease":["authorization_probe"],"lane":"target_touching","attempt":1}),
        json!({"slice_ids":[second_slice],"worker_ids":[workers[1]],"tool_ids":["authorization_probe"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}),
    );
    assert!(reclaimed.applied(), "after expiry reclaim the exclusive lane is free again: {reclaimed:?}");
    // Release the second carrier the same way before the read-only parser
    // slice: the execution binding keeps read-only lanes behind the exclusive
    // target/review occupation (apply's own ordering contract).
    fixture.db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'", []).unwrap();
    h::pool::reclaim_expired_for_task(&fixture.db, &fixture.task).unwrap();
    // Dead audit identities are replaced inside the same frozen capacity,
    // exactly as the root loop does each iteration; pick a live idle worker.
    fixture.provision(2);
    let _parser_worker: String = fixture.db
        .query_row("SELECT worker_id FROM agent_worker_instances WHERE root_run_id=?1 AND state='idle' ORDER BY worker_id LIMIT 1",[&fixture.task.root_run_id],|r|r.get(0))
        .unwrap();
    // Parser invocation reconciliation (no free replay after an already
    // recorded invocation) is locked by the existing parser replay suite in
    // agent_tests_homogeneous_parser.rs together with the no-fourth-GET
    // assertion in replay_or_expired_attempt_blocks_late_target_delivery.
}
