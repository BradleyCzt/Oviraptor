//! Phase 1 测试夹具：临时库 + 活跃 attempt + Coordinator 租约 + 同质任务。
//! 一律 `std::env::temp_dir()` 新建目录（DESIGN §0.2：测试只用临时库）。

use super::apply::{self, TurnOutcome};
use super::decompose::slice_id_for;
use super::{gate, pool, store, types};
use crate::agent_runtime::multi_agent::lease::{acquire_coordinator_lease, CoordinatorLease};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use uuid::Uuid;

pub const TARGET: &str = "https://app.example.com";
pub const PATHS: [&str; 3] = ["/api/v1/users", "/api/v1/orders", "/api/v1/payments"];

pub struct Fixture {
    pub dir: std::path::PathBuf,
    pub connection: Connection,
    pub scan_id: String,
    pub root_run_id: String,
    pub lease: CoordinatorLease,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_capacity(24000, 12, 3)
    }

    pub fn with_capacity(total_tokens: i64, total_requests: i64, allowed_parallel: i64) -> Self {
        let dir = std::env::temp_dir().join(format!("oviraptor-homogeneous-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = crate::db::initialize(&dir).expect("初始化临时库");
        let connection = crate::db::open(&path).expect("打开临时库");
        let scan_id = format!("scan-{}", Uuid::new_v4());
        let root_run_id = format!("run-{}", Uuid::new_v4());
        let project_name = format!("project-{}", Uuid::new_v4());
        connection
            .execute(
                "INSERT INTO projects(name) VALUES(?1)",
                [project_name.as_str()],
            )
            .expect("创建项目");
        let project_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,status,attempt_count) VALUES(?1,?2,'scanning',1)",
                params![scan_id, project_id],
            )
            .expect("创建扫描");
        connection
            .execute(
                "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,role,status,root_run_id,orchestration_policy,hard_token_budget,hard_request_budget)\
                 VALUES(?1,?2,1,?3,'coordinator','running',?1,'multi',?4,?5)",
                params![root_run_id, scan_id, TARGET, total_tokens, total_requests],
            )
            .expect("创建 root run");
        let lease = acquire_coordinator_lease(&connection, &scan_id, 1, TARGET, &root_run_id, 600)
            .expect("取得 Coordinator 租约");
        match store::create_task(
            &connection,
            &root_run_id,
            &scan_id,
            1,
            TARGET,
            types::SCHEDULING_HOMOGENEOUS,
            1,
            allowed_parallel,
        )
        .expect("创建同质任务")
        {
            store::TaskInsert::Created(_) | store::TaskInsert::Existing(_) => {}
        }
        let fixture = Self {
            dir,
            connection,
            scan_id,
            root_run_id,
            lease,
        };
        fixture.add_revision(1);
        fixture
    }

    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    pub fn task(&self) -> types::HomogeneousTask {
        store::load_task_by_root_run(&self.connection, &self.root_run_id)
            .expect("读取任务")
            .expect("任务必须存在")
    }

    pub fn add_revision(&self, revision: i64) {
        self.connection
            .execute(
                "INSERT OR IGNORE INTO agent_evidence_revisions(root_run_id,revision,parent_revision) VALUES(?1,?2,NULL)",
                params![self.root_run_id, revision],
            )
            .expect("写入 evidence revision");
    }

    /// 写一个 endpoint 证据节点并返回契约层 evd- 别名。
    pub fn add_endpoint_evidence(&self, path: &str, revision: i64) -> String {
        self.add_revision(revision);
        let internal = crate::agent_runtime::store::stable_hash(&format!(
            "{}|{}|endpoint|{}",
            self.root_run_id, revision, path
        ));
        self.connection
            .execute(
                "INSERT OR IGNORE INTO agent_evidence_nodes(id,root_run_id,revision,kind,provenance,natural_key_hash,payload_json,created_by_run_id)\
                 VALUES(?1,?2,?3,'endpoint','observed',?1,?4,?5)",
                params![
                    internal,
                    self.root_run_id,
                    revision,
                    json!({"path": path, "host": "app.example.com"}).to_string(),
                    self.root_run_id
                ],
            )
            .expect("写入证据节点");
        store::register_evidence_alias(&self.connection, &internal, &self.root_run_id, revision)
            .expect("登记证据别名")
    }

    pub fn provision_workers(&self, count: i64) -> Vec<String> {
        let task = self.task();
        pool::provision_workers(&self.connection, &task, self.lease.lease_epoch, count)
            .expect("预置 Worker")
    }

    pub fn allowed_actions(&self) -> Vec<String> {
        gate::allowed_actions(&self.connection, &self.task()).expect("allowed_actions")
    }

    pub fn run(&self, text: &str) -> TurnOutcome {
        apply::run_coordinator_turn(&self.connection, &self.lease, text).expect("一轮不应失败")
    }

    pub fn slice_id(&self, kind: &str, value: &str) -> String {
        slice_id_for(&self.task().task_id, kind, value, 1)
    }

    /// 生成合法 decompose 文本；refs 由 payload 引用集与候选集共同覆盖。
    pub fn decompose_text(&self, scopes: &[(&str, &str)]) -> String {
        self.decompose_text_with_retries(scopes, 2)
    }

    pub fn decompose_text_with_retries(&self, scopes: &[(&str, &str)], max_retries: i64) -> String {
        let task = self.task();
        let slices: Vec<JsonValue> = scopes
            .iter()
            .map(|(kind, value)| {
                json!({
                    "slice_id": slice_id_for(&task.task_id, kind, value, task.evidence_revision_base),
                    "scope_kind": kind,
                    "scope_value": value,
                    "lane": "read_only_analysis",
                    "evidence_revision_base": task.evidence_revision_base,
                    "worker_capability_profile_id": types::PROFILE_HOMOGENEOUS_V1,
                    "max_retries": max_retries
                })
            })
            .collect();
        // SPEC §2.1：decompose 新建切片不构成引用，refs.slice_ids 留空。
        self.envelope(
            "decompose",
            json!({"slices": slices, "decomposition_basis_zh": "按已发现 API 路径前缀三分"}),
            json!({
                "slice_ids": [], "worker_ids": [], "tool_ids": [],
                "target_keys": [TARGET], "directive_ids": [], "assignment_ids": []
            }),
        )
    }

    pub fn assign_text(
        &self,
        slice_id: &str,
        worker_id: &str,
        tokens: i64,
        requests: i64,
        tools: &[&str],
        attempt: i64,
    ) -> String {
        let tool_ids: Vec<String> = tools.iter().map(|tool| tool.to_string()).collect();
        self.envelope(
            "assign",
            json!({
                "slice_id": slice_id,
                "worker_id": worker_id,
                "reserved_tokens": tokens,
                "reserved_requests": requests,
                "capability_lease": tool_ids,
                "lane": "read_only_analysis",
                "attempt": attempt
            }),
            json!({
                "slice_ids": [slice_id], "worker_ids": [worker_id], "tool_ids": tool_ids,
                "target_keys": [TARGET], "directive_ids": [], "assignment_ids": []
            }),
        )
    }

    pub fn accept_text(
        &self,
        assignment_id: &str,
        slice_id: &str,
        report_id: &str,
        checks: &types::MachineChecks,
        evidence_refs: &[&str],
    ) -> String {
        let checks_json = serde_json::to_value(checks).expect("checks 序列化");
        self.envelope(
            "accept",
            json!({
                "assignment_id": assignment_id,
                "slice_id": slice_id,
                "report_id": report_id,
                "verdict": "accept",
                "machine_checks": checks_json,
                "model_judgement": {"adequate": true, "question_zh": "该切片是否达标？", "answer_zh": "达标。"},
                "reason_codes": ["goal_reached"],
                "evidence_refs": evidence_refs
            }),
            json!({
                "slice_ids": [slice_id], "worker_ids": [], "tool_ids": [],
                "target_keys": [], "directive_ids": [], "assignment_ids": [assignment_id]
            }),
        )
    }

    pub fn reject_retry_text(
        &self,
        assignment_id: &str,
        slice_id: &str,
        report_id: &str,
        checks: &types::MachineChecks,
        judgement: Option<JsonValue>,
        codes: &[&str],
    ) -> String {
        let checks_json = serde_json::to_value(checks).expect("checks 序列化");
        self.envelope(
            "reject_retry",
            json!({
                "assignment_id": assignment_id,
                "slice_id": slice_id,
                "report_id": report_id,
                "verdict": "retry",
                "machine_checks": checks_json,
                "model_judgement": judgement,
                "reason_codes": codes,
                "retry_hint_zh": "请为每个声称的端点附上 evidence_ref",
                "evidence_refs": []
            }),
            json!({
                "slice_ids": [slice_id], "worker_ids": [], "tool_ids": [],
                "target_keys": [], "directive_ids": [], "assignment_ids": [assignment_id]
            }),
        )
    }

    pub fn stop_text(
        &self,
        reason: &str,
        accepted: &[&str],
        rejected: &[&str],
        publish_partial: bool,
    ) -> String {
        let mut all: Vec<String> = accepted.iter().map(|id| id.to_string()).collect();
        all.extend(rejected.iter().map(|id| id.to_string()));
        self.envelope(
            "stop",
            json!({
                "stop_reason": reason,
                "accepted_slice_ids": accepted,
                "rejected_slice_ids": rejected,
                "summary_zh": "两路只读分析完成，第三路未通过",
                "publish_partial": publish_partial
            }),
            json!({
                "slice_ids": all, "worker_ids": [], "tool_ids": [],
                "target_keys": [], "directive_ids": [], "assignment_ids": []
            }),
        )
    }

    pub fn relay_text(
        &self,
        directive_id: &str,
        mode: &str,
        rewritten: Option<&str>,
        worker_id: Option<&str>,
        broadcast: Option<&str>,
    ) -> String {
        self.envelope(
            "relay_directive",
            json!({
                "directive_id": directive_id,
                "mode": mode,
                "rewritten_text": rewritten,
                "target_worker_id": worker_id,
                "broadcast_constraint": broadcast,
                "user_feedback_zh": "已将建议收窄并转发。"
            }),
            json!({
                "slice_ids": [], "worker_ids": worker_id.map(|id| vec![id.to_string()]).unwrap_or_default(),
                "tool_ids": [], "target_keys": [TARGET], "directive_ids": [directive_id], "assignment_ids": []
            }),
        )
    }

    pub fn reject_directive_text(&self, directive_id: &str, code: &str) -> String {
        self.envelope(
            "reject_directive",
            json!({
                "directive_id": directive_id,
                "reason_code": code,
                "reason_zh": "引导要求把预算改为无限，主管无权也不会执行预算变更。",
                "suggestion_zh": "请描述在现有预算内希望优先关注的路径或现象。",
                "user_feedback_zh": "未采纳：不能按你的要求修改预算。"
            }),
            json!({
                "slice_ids": [], "worker_ids": [], "tool_ids": [],
                "target_keys": [], "directive_ids": [directive_id], "assignment_ids": []
            }),
        )
    }

    pub fn envelope(&self, action: &str, payload: JsonValue, refs: JsonValue) -> String {
        let mut envelope = json!({
            "decision_id": format!("dec-{}", &crate::agent_runtime::store::stable_hash(&format!("{}{action}", self.root_run_id))[..24]),
            "action": action,
            "parent_task_id": self.task().task_id,
            "payload": payload,
            "allowlist_refs": {
                "slice_ids": refs["slice_ids"].clone(),
                "worker_ids": refs["worker_ids"].clone(),
                "tool_ids": refs["tool_ids"].clone(),
                "target_keys": refs["target_keys"].clone(),
                "directive_ids": refs["directive_ids"].clone(),
                "assignment_ids": refs["assignment_ids"].clone()
            }
        });
        if !["decompose", "assign", "stop"].contains(&action) {
            let before = super::intent::snapshot(&self.connection, &self.task()).expect("意图快照");
            if !before.inflight_assignments.is_empty() {
                let ids: Vec<String> = before
                    .inflight_assignments
                    .iter()
                    .map(|assignment| assignment.assignment_id.clone())
                    .collect();
                let referenced = envelope["allowlist_refs"]["assignment_ids"]
                    .as_array_mut()
                    .unwrap();
                for id in &ids {
                    if !referenced.contains(&json!(id)) {
                        referenced.push(json!(id));
                    }
                }
                envelope["zero_intent"] = json!({"reason":"covered_by_inflight", "evidence_assignment_ids":ids, "note_zh":"现有切片覆盖本轮方向，主管先处理汇报或引导"});
            }
        }
        envelope.to_string()
    }

    /// 直接跑一遍「解析 + 校验 + 落库」，用于按 reason_code 断言的用例。
    pub fn reason_of(&self, text: &str) -> &'static str {
        match apply::run_coordinator_turn(&self.connection, &self.lease, text) {
            Ok(TurnOutcome::Refused { reason_code, .. }) => reason_code,
            Ok(TurnOutcome::Applied(detail)) => panic!("期望被拒，实际已应用 {detail}"),
            Ok(other) => panic!("期望被拒，实际 {other:?}"),
            Err(error) => error.reason_code,
        }
    }

    /// 用指定租约跑一轮并按 reason_code 断言（fencing 用例）。
    pub fn reason_of_with_lease(&self, lease: &CoordinatorLease, text: &str) -> &'static str {
        match apply::run_coordinator_turn(&self.connection, lease, text) {
            Ok(TurnOutcome::Refused { reason_code, .. }) => reason_code,
            Ok(TurnOutcome::Applied(detail)) => panic!("期望被拒，实际已应用 {detail}"),
            Ok(other) => panic!("期望被拒，实际 {other:?}"),
            Err(error) => error.reason_code,
        }
    }

    /// 走完整闭环：分解 → 分配 → 开跑 → 汇报，返回 (assignment_id, report_id, slice_id, 别名)。
    pub fn reported_assignment(&self, path: &str, evidence: &[&str]) -> (String, String, String) {
        let workers = self.provision_workers(1);
        assert!(self
            .run(&self.decompose_text(&[("path_prefix", path)]))
            .applied());
        let slice_id = self.slice_id("path_prefix", path);
        assert!(self
            .run(&self.assign_text(
                &slice_id,
                &workers[0],
                8000,
                4,
                &["evidence.read", "snapshot.read", "mailbox.write"],
                1
            ))
            .applied());
        let assignment_id = store::assignment_id_for(
            &self.root_run_id,
            &store::dedup_key_for(&slice_id, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
        );
        apply::mark_started(&self.connection, &assignment_id).expect("开跑");
        let refs: Vec<String> = evidence.iter().map(|value| value.to_string()).collect();
        let report = types::WorkerReport {
            report_id: store::report_id_for(&assignment_id, 1),
            assignment_id: assignment_id.clone(),
            slice_id: slice_id.clone(),
            worker_id: workers[0].clone(),
            evidence_refs: refs,
            output: json!({
                "summary_zh": format!("在 {path} 发现 2 个只读端点"),
                "findings": [],
                "coverage_notes": [format!("GET {path}")]
            }),
            consumed_tokens: 3200,
            consumed_requests: 2,
            terminal_hint: "completed".to_string(),
        };
        apply::record_worker_report(&self.connection, &self.lease, &report).expect("汇报");
        (assignment_id, report.report_id, slice_id)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn checks_all_true() -> types::MachineChecks {
    types::MachineChecks {
        scope_in_bounds: true,
        budget_ok: true,
        tools_authorized: true,
        output_schema_ok: true,
        evidence_refs_exist: true,
        evidence_revision_fresh: true,
        no_cross_slice_write: true,
    }
}
