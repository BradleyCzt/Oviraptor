-- Phase 1 同质 Worker + 强主管：控制面 DDL。
-- 枚举值一律照抄 SPEC §1.1 与 DESIGN §4；不新增状态、动作、角色或 reason_code。
-- DESIGN §4.3 的索引原写作 agent_assignments.attempt_number（该列不存在，现状只有
-- lease_epoch 与 attempt 语义在 attempts 表），故这里按 SPEC §1.4 Assignment.attempt
-- 新增 attempt 列后建索引；见 DESIGN §4.3 与实现说明。

-- DESIGN §4.1
CREATE TABLE IF NOT EXISTS agent_homogeneous_tasks (
  task_id TEXT PRIMARY KEY CHECK(task_id GLOB 'tsk-*'),
  root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  scan_id TEXT NOT NULL,
  attempt_number INTEGER NOT NULL,
  target_key TEXT NOT NULL,
  scheduling_model TEXT NOT NULL DEFAULT 'homogeneous_workers'
    CHECK(scheduling_model IN ('legacy_role_pipeline','homogeneous_workers')),
  state TEXT NOT NULL DEFAULT 'created'
    CHECK(state IN ('created','decomposed','running','partial_success',
                    'succeeded','failed','cancelled','stopped')),
  evidence_revision_base INTEGER NOT NULL DEFAULT 0,
  allowed_parallel INTEGER NOT NULL DEFAULT 3,
  degraded INTEGER NOT NULL DEFAULT 0,
  degrade_reason_code TEXT NOT NULL DEFAULT '',
  schema_fail_streak INTEGER NOT NULL DEFAULT 0,
  hard_reject_streak INTEGER NOT NULL DEFAULT 0,
  decided_by_lease_epoch INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  UNIQUE(root_run_id)
);

-- DESIGN §4.2
CREATE TABLE IF NOT EXISTS agent_task_slices (
  slice_id TEXT PRIMARY KEY CHECK(slice_id GLOB 'slc-*'),
  parent_task_id TEXT NOT NULL REFERENCES agent_homogeneous_tasks(task_id) ON DELETE CASCADE,
  scope_kind TEXT NOT NULL CHECK(scope_kind IN ('path_prefix','host','asset_id','source_type')),
  scope_value TEXT NOT NULL CHECK(length(scope_value) BETWEEN 1 AND 512),
  evidence_revision_base INTEGER NOT NULL DEFAULT 0,
  lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
  worker_capability_profile_id TEXT NOT NULL DEFAULT 'homogeneous_v1'
    CHECK(worker_capability_profile_id = 'homogeneous_v1'),
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK(state IN ('pending','assigned','in_progress','reported','accepted','rejected','cancelled')),
  max_retries INTEGER NOT NULL DEFAULT 2 CHECK(max_retries BETWEEN 0 AND 3),
  retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count BETWEEN 0 AND 3),
  decomposition_model TEXT NOT NULL DEFAULT 'coordinator'
    CHECK(decomposition_model IN ('coordinator','deterministic')),
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  CHECK(retry_count <= max_retries)
);
CREATE INDEX IF NOT EXISTS idx_task_slices_task_state ON agent_task_slices(parent_task_id, state);

-- DESIGN §4.4：homogeneous 不写旧 agent_lane_leases，配额与占用走这两张表。
CREATE TABLE IF NOT EXISTS agent_lane_quotas (
  root_run_id TEXT NOT NULL,
  lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
  max_parallel INTEGER NOT NULL DEFAULT 1 CHECK(max_parallel BETWEEN 1 AND 16),
  PRIMARY KEY(root_run_id, lane)
);
CREATE TABLE IF NOT EXISTS agent_lane_slots (
  root_run_id TEXT NOT NULL,
  lane TEXT NOT NULL,
  slice_id TEXT NOT NULL,
  assignment_id TEXT NOT NULL,
  acquired_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  released_at TEXT NOT NULL DEFAULT '',
  PRIMARY KEY(root_run_id, lane, slice_id)
);

-- DESIGN §4.5
CREATE TABLE IF NOT EXISTS agent_acceptance_verdicts (
  verdict_id TEXT PRIMARY KEY CHECK(verdict_id GLOB 'vrd-*'),
  assignment_id TEXT NOT NULL,
  slice_id TEXT NOT NULL,
  verdict TEXT NOT NULL CHECK(verdict IN ('accept','retry','reject')),
  machine_checks_json TEXT NOT NULL,
  model_judgement_json TEXT NOT NULL DEFAULT '',
  reason_codes_json TEXT NOT NULL,
  evidence_refs_json TEXT NOT NULL,
  decided_by_lease_epoch INTEGER NOT NULL,
  -- 审计用的 attempt 作用域（SPEC §1.4 Assignment.attempt）。DESIGN §4.5 原写
  -- UNIQUE(assignment_id,verdict)，其注释假设「retry 后是新 assignment」；但 SPEC §1.4
  -- 的去重键决定同一切片重跑复用同一 assignment 行，故唯一键必须带 attempt，
  -- 否则第二次 retry 裁决会被 ON CONFLICT DO NOTHING 静默丢弃。偏差已在交付说明中上报。
  attempt INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  UNIQUE(assignment_id, verdict, attempt)
);
CREATE TRIGGER IF NOT EXISTS acceptance_verdict_no_update BEFORE UPDATE ON agent_acceptance_verdicts
BEGIN SELECT RAISE(ABORT,'acceptance_verdict_immutable'); END;
CREATE TRIGGER IF NOT EXISTS acceptance_verdict_no_delete BEFORE DELETE ON agent_acceptance_verdicts
BEGIN SELECT RAISE(ABORT,'acceptance_verdict_audit_immutable'); END;

-- DESIGN §4.6
CREATE TABLE IF NOT EXISTS agent_human_directive_gated (
  directive_id TEXT PRIMARY KEY CHECK(directive_id GLOB 'dir-*'),
  parent_task_id TEXT NOT NULL,
  source_user_directive_id TEXT NOT NULL DEFAULT '',
  raw_text_redacted TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN (
    'received','rule_screened','awaiting_coordinator','relayed','rewritten','rejected','applied')),
  rule_screen_result TEXT NOT NULL CHECK(rule_screen_result IN ('pass','need_confirm','reject')),
  rule_reason_codes_json TEXT NOT NULL DEFAULT '[]',
  coordinator_action TEXT NOT NULL DEFAULT '',
  rewritten_text TEXT NOT NULL DEFAULT '',
  target_worker_id TEXT NOT NULL DEFAULT '',
  broadcast_constraint TEXT NOT NULL DEFAULT '',
  reason_code TEXT NOT NULL DEFAULT '',
  reason_zh TEXT NOT NULL DEFAULT '',
  suggestion_zh TEXT NOT NULL DEFAULT '',
  user_feedback_zh TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- DESIGN §4.7：SPEC §1.3 的 idle/busy 投影。只有 homogeneous 调度事务写这张表，
-- 读路径永不签发身份（对齐现状 attempts 的「No read path issues a replacement identity」）。
CREATE TABLE IF NOT EXISTS agent_worker_instances (
  worker_id TEXT PRIMARY KEY,
  root_run_id TEXT NOT NULL,
  capability_profile_id TEXT NOT NULL DEFAULT 'homogeneous_v1'
    CHECK(capability_profile_id = 'homogeneous_v1'),
  assignment_id TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'idle' CHECK(state IN ('idle','busy','draining','dead')),
  lease_epoch INTEGER NOT NULL DEFAULT 1 CHECK(lease_epoch >= 1),
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_worker_instances_state ON agent_worker_instances(root_run_id, state);

-- SPEC §1.5 WorkerReport 落库（DESIGN §3.3 的汇报闭环需要可回放的原始汇报）。
CREATE TABLE IF NOT EXISTS agent_worker_reports (
  report_id TEXT PRIMARY KEY CHECK(report_id GLOB 'rpt-*'),
  assignment_id TEXT NOT NULL,
  slice_id TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  evidence_refs_json TEXT NOT NULL DEFAULT '[]',
  output_json TEXT NOT NULL,
  consumed_tokens INTEGER NOT NULL DEFAULT 0,
  consumed_requests INTEGER NOT NULL DEFAULT 0,
  terminal_hint TEXT NOT NULL CHECK(terminal_hint IN ('completed','failed','partial')),
  lease_epoch INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  CHECK(consumed_tokens >= 0 AND consumed_requests >= 0)
);
CREATE TRIGGER IF NOT EXISTS worker_report_no_update BEFORE UPDATE ON agent_worker_reports
BEGIN SELECT RAISE(ABORT,'worker_report_immutable'); END;
CREATE TRIGGER IF NOT EXISTS worker_report_no_delete BEFORE DELETE ON agent_worker_reports
BEGIN SELECT RAISE(ABORT,'worker_report_audit_immutable'); END;

-- DESIGN §4.8：契约层 evd- 别名 ↔ 现状 64-hex 证据节点 id。不放宽 SPEC pattern。
CREATE TABLE IF NOT EXISTS agent_evidence_aliases (
  evd_id TEXT PRIMARY KEY CHECK(evd_id GLOB 'evd-*'),
  internal_id TEXT NOT NULL,
  root_run_id TEXT NOT NULL DEFAULT '',
  revision INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  UNIQUE(internal_id)
);

-- 深度分析 §J：付费主管决策经 Rust 核验后的不可变意图审核（不复用 DecisionSummary）。
CREATE TABLE IF NOT EXISTS agent_homogeneous_decision_audit (
  audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
  decision_id TEXT NOT NULL,
  parent_task_id TEXT NOT NULL,
  action TEXT NOT NULL CHECK(action IN ('decompose','assign','accept','reject_retry','stop','relay_directive','reject_directive')),
  stop_reason TEXT NOT NULL DEFAULT '',
  zero_intent_json TEXT NOT NULL,
  intent_snapshot_json TEXT NOT NULL,
  produced_intent INTEGER NOT NULL CHECK(produced_intent IN (0,1)),
  decided_by_lease_epoch INTEGER NOT NULL,
  trigger_reason_code TEXT NOT NULL DEFAULT '' CHECK(trigger_reason_code IN ('','zero_intent_illegal')),
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS homogeneous_decision_audit_no_update BEFORE UPDATE ON agent_homogeneous_decision_audit
BEGIN SELECT RAISE(ABORT,'homogeneous_decision_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_decision_audit_no_delete BEFORE DELETE ON agent_homogeneous_decision_audit
BEGIN SELECT RAISE(ABORT,'homogeneous_decision_audit_immutable'); END;
