# Nest 同质 Worker + 强主管：给 Qwen 的详细设计规格（可照抄）

> 版本：2026-10-10 ｜ 目标读者：国产 Qwen 设计会话 ｜ 性质：目标态详细规格，不是现状
> 总览文档：`docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md`
> 现状基线：`docs/NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md`
> 只写文档，不改代码，不 commit。

---

> **设计定稿（实现依据）**：[`NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md`](NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md)；冲突裁定 DESIGN > SPEC > ARCHITECTURE。
> **Qwen 实现话术**：[`NEST_QWEN_IMPLEMENTATION_TASK_PROMPT_2026-10-10.md`](NEST_QWEN_IMPLEMENTATION_TASK_PROMPT_2026-10-10.md)（勿再用已废弃的 DESIGN_TASK 话术）。

## 0. 开篇硬规则（违反任一条 = 整份设计作废）

1. **禁止发明**：不得发明新角色名、新工具名、新状态名、新动作名、新 reason_code。只能使用本文列出的集合。
2. **禁止跳过主管**：用户/Worker 不得直达对方；一切派发、验收、引导转发必须经主管。
3. **禁止平权互委派**：Worker 之间不得互相通信或委派。
4. **JSON 键必须 snake_case 英文**；中文只出现在 `*_zh` 字段与提示文案中。
5. **主管每次输出必须是单个 JSON 对象**，符合本文 schema；缺字段、多字段、类型错、越权字段 → **整单拒绝**，走确定性降级。
6. **候选只能从 allowlist 选 id**：切片、工具、目标、Worker 均由 Rust 提供候选列表；模型不得发明 URL/工具/目标。
7. **第一期不动 Asset Exposure / 外网暴露面业务**；对接点见 §10。
8. **交付设计时必须按 §11 固定大纲**，缺节视为未完成。

---

## 1. 完整类型定义与 JSON Schema

以下所有类型字段均为**封闭集合**。未列出的字段一律非法。

### 1.1 公共枚举（封闭）

```json
{
  "worker_capability_profile_id": ["homogeneous_v1"],
  "lane": ["read_only_analysis", "target_touching", "review"],
  "assignment_state": ["created", "assigned", "running", "reported", "accepted", "rejected_retry", "stopped", "cancelled"],
  "slice_state": ["pending", "assigned", "in_progress", "reported", "accepted", "rejected", "cancelled"],
  "task_state": ["created", "decomposed", "running", "partial_success", "succeeded", "failed", "cancelled", "stopped"],
  "directive_state": ["received", "rule_screened", "awaiting_coordinator", "relayed", "rewritten", "rejected", "applied"],
  "verdict": ["accept", "retry", "reject"],
  "coordinator_action": ["decompose", "assign", "accept", "reject_retry", "stop", "relay_directive", "reject_directive"],
  "reason_code": [
    "schema_invalid",
    "unknown_action",
    "missing_required_field",
    "extra_field",
    "allowlist_violation",
    "slice_overlap",
    "budget_exceeded",
    "tool_not_authorized",
    "identity_change_forbidden",
    "scope_escape",
    "url_invented",
    "cross_slice_write",
    "evidence_missing",
    "evidence_contradiction",
    "output_schema_invalid",
    "lane_quota_exceeded",
    "worker_not_found",
    "assignment_not_found",
    "directive_privilege_escalation",
    "directive_budget_change",
    "directive_identity_change",
    "directive_cleanup_forbidden",
    "machine_check_failed",
    "model_judgement_fail",
    "duplicate_slice",
    "stale_evidence_revision",
    "lease_invalid",
    "degraded_path_used",
    "user_cancelled",
    "goal_reached",
    "budget_exhausted",
    "retry_limit_exceeded",
    "markdown_wrapped_json",
    "free_text_instead_of_json",
    "invented_role",
    "invented_tool",
    "invented_state"
  ]
}
```

### 1.2 TaskSlice

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| slice_id | string | 是 | `^slc-[a-f0-9]{16,32}$`，全局唯一 |
| parent_task_id | string | 是 | `^tsk-[a-f0-9]{16,32}$` |
| scope_kind | string enum | 是 | `path_prefix` \| `host` \| `asset_id` \| `source_type` |
| scope_value | string | 是 | 非空，长度 1–512，无控制字符 |
| evidence_revision_base | integer | 是 | ≥ 0 |
| lane | string enum | 是 | 见公共枚举 |
| worker_capability_profile_id | string enum | 是 | 仅 `homogeneous_v1` |
| state | string enum | 是 | 见 slice_state |
| max_retries | integer | 是 | 0–3 |
| retry_count | integer | 是 | 0–max_retries |

合法例：

```json
{
  "slice_id": "slc-a1b2c3d4e5f67890",
  "parent_task_id": "tsk-1122334455667788",
  "scope_kind": "path_prefix",
  "scope_value": "/api/v1/users",
  "evidence_revision_base": 3,
  "lane": "read_only_analysis",
  "worker_capability_profile_id": "homogeneous_v1",
  "state": "pending",
  "max_retries": 2,
  "retry_count": 0
}
```

非法例：
1. `scope_kind: "regex"` → 不在枚举 → `schema_invalid`
2. 缺 `slice_id` → `missing_required_field`
3. 多字段 `"priority": 1` → `extra_field`
4. `worker_capability_profile_id: "expert_auth"` → `invented_role` / allowlist 违规

JSON Schema：

```json
{
  "$id": "https://oviraptor.local/schema/task_slice.json",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "slice_id", "parent_task_id", "scope_kind", "scope_value",
    "evidence_revision_base", "lane", "worker_capability_profile_id",
    "state", "max_retries", "retry_count"
  ],
  "properties": {
    "slice_id": { "type": "string", "pattern": "^slc-[a-f0-9]{16,32}$" },
    "parent_task_id": { "type": "string", "pattern": "^tsk-[a-f0-9]{16,32}$" },
    "scope_kind": { "enum": ["path_prefix", "host", "asset_id", "source_type"] },
    "scope_value": { "type": "string", "minLength": 1, "maxLength": 512 },
    "evidence_revision_base": { "type": "integer", "minimum": 0 },
    "lane": { "enum": ["read_only_analysis", "target_touching", "review"] },
    "worker_capability_profile_id": { "enum": ["homogeneous_v1"] },
    "state": { "enum": ["pending", "assigned", "in_progress", "reported", "accepted", "rejected", "cancelled"] },
    "max_retries": { "type": "integer", "minimum": 0, "maximum": 3 },
    "retry_count": { "type": "integer", "minimum": 0, "maximum": 3 }
  }
}
```

### 1.3 WorkerInstance

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| worker_id | string | 是 | `^wrk-[a-f0-9]{16,32}$` |
| capability_profile_id | string enum | 是 | 仅 `homogeneous_v1` |
| assignment_id | string\|null | 是 | 空闲时 null |
| state | string enum | 是 | `idle` \| `busy` \| `draining` \| `dead` |
| lease_epoch | integer | 是 | ≥ 1 |

合法例：

```json
{
  "worker_id": "wrk-0f1e2d3c4b5a6978",
  "capability_profile_id": "homogeneous_v1",
  "assignment_id": "asg-9988776655443322",
  "state": "busy",
  "lease_epoch": 4
}
```

非法例：
1. `capability_profile_id: "InputParser"` → `invented_role`
2. `state: "thinking"` → `invented_state`

### 1.4 Assignment

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| assignment_id | string | 是 | `^asg-[a-f0-9]{16,32}$` |
| slice_id | string | 是 | 必须存在于已分解切片 |
| worker_id | string | 是 | 必须存在且 idle→busy |
| parent_task_id | string | 是 | |
| lane | string enum | 是 | 与切片一致 |
| state | string enum | 是 | assignment_state |
| reserved_tokens | integer | 是 | ≥ 0，≤ 主管剩余 |
| reserved_requests | integer | 是 | ≥ 0，≤ 主管剩余 |
| capability_lease | string[] | 是 | 每个元素必须在工具 allowlist |
| evidence_revision_base | integer | 是 | ≥ 0 |
| attempt | integer | 是 | ≥ 1 |

**去重键（目标态，必须含 slice_id）**：
`dedupe_key = "{capability_profile_id}:{trigger_code}:{target_key}:{evidence_revision}:{slice_id}"`

合法例：

```json
{
  "assignment_id": "asg-9988776655443322",
  "slice_id": "slc-a1b2c3d4e5f67890",
  "worker_id": "wrk-0f1e2d3c4b5a6978",
  "parent_task_id": "tsk-1122334455667788",
  "lane": "read_only_analysis",
  "state": "assigned",
  "reserved_tokens": 8000,
  "reserved_requests": 4,
  "capability_lease": ["http.get", "snapshot.read", "evidence.read"],
  "evidence_revision_base": 3,
  "attempt": 1
}
```

非法例：
1. `capability_lease` 含 `"budget.extend"` → `tool_not_authorized`
2. `reserved_tokens: -1` → `schema_invalid`
3. 缺 `slice_id` → `missing_required_field`（现状去重缺切片是已知缺陷，目标态禁止）

### 1.5 WorkerReport

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| report_id | string | 是 | `^rpt-[a-f0-9]{16,32}$` |
| assignment_id | string | 是 | |
| slice_id | string | 是 | 必须与 assignment 一致 |
| worker_id | string | 是 | |
| evidence_refs | string[] | 是 | 每项 `^evd-[a-f0-9]{8,32}$`，可空数组但若有结论则非空 |
| output | object | 是 | 符合 Worker 输出 schema，`additionalProperties:false` |
| consumed_tokens | integer | 是 | ≥ 0，≤ reserved_tokens |
| consumed_requests | integer | 是 | ≥ 0，≤ reserved_requests |
| terminal_hint | string enum | 是 | `completed` \| `failed` \| `partial` |

合法例：

```json
{
  "report_id": "rpt-aabbccddeeff0011",
  "assignment_id": "asg-9988776655443322",
  "slice_id": "slc-a1b2c3d4e5f67890",
  "worker_id": "wrk-0f1e2d3c4b5a6978",
  "evidence_refs": ["evd-11112222", "evd-33334444"],
  "output": {
    "summary_zh": "在 /api/v1/users 发现 2 个只读端点",
    "findings": [],
    "coverage_notes": ["GET /api/v1/users", "GET /api/v1/users/{id}"]
  },
  "consumed_tokens": 3200,
  "consumed_requests": 2,
  "terminal_hint": "completed"
}
```

非法例：
1. `slice_id` 与 assignment 不一致 → `cross_slice_write`
2. `consumed_tokens` > reserved → `budget_exceeded`
3. `output` 含 `"new_tool": "shell.exec"` → `extra_field` / `invented_tool`

### 1.6 AcceptanceVerdict

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| verdict_id | string | 是 | `^vrd-[a-f0-9]{16,32}$` |
| assignment_id | string | 是 | |
| slice_id | string | 是 | |
| verdict | string enum | 是 | accept \| retry \| reject |
| machine_checks | object | 是 | 见下 |
| model_judgement | object\|null | 条件 | 机器可检全过才可非 null |
| reason_codes | string[] | 是 | 每项必须在 reason_code 枚举 |
| evidence_refs | string[] | 是 | |
| decided_by_lease_epoch | integer | 是 | ≥ 1 |

`machine_checks` 固定键：

```json
{
  "scope_in_bounds": true,
  "budget_ok": true,
  "tools_authorized": true,
  "output_schema_ok": true,
  "evidence_refs_exist": true,
  "evidence_revision_fresh": true,
  "no_cross_slice_write": true
}
```

任一为 false → `verdict` 只能是 `reject` 或 `retry`，且 `model_judgement` 必须为 null。

合法例（accept）：

```json
{
  "verdict_id": "vrd-deadbeefcafebabe",
  "assignment_id": "asg-9988776655443322",
  "slice_id": "slc-a1b2c3d4e5f67890",
  "verdict": "accept",
  "machine_checks": {
    "scope_in_bounds": true,
    "budget_ok": true,
    "tools_authorized": true,
    "output_schema_ok": true,
    "evidence_refs_exist": true,
    "evidence_revision_fresh": true,
    "no_cross_slice_write": true
  },
  "model_judgement": {
    "adequate": true,
    "question_zh": "该切片是否覆盖了声明的 path_prefix 且结论与证据一致？",
    "answer_zh": "是，两个端点均有证据支撑。"
  },
  "reason_codes": ["goal_reached"],
  "evidence_refs": ["evd-11112222", "evd-33334444"],
  "decided_by_lease_epoch": 4
}
```

非法例：
1. `machine_checks.budget_ok: false` 且 `verdict: "accept"` → 整单拒绝
2. `reason_codes: ["看起来不行"]` → 不在枚举 → `schema_invalid`

### 1.7 HumanDirective

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| directive_id | string | 是 | `^dir-[a-f0-9]{16,32}$` |
| raw_text_redacted | string | 是 | 已脱敏，1–2000 字符 |
| state | string enum | 是 | directive_state |
| rule_screen_result | string enum | 是 | `pass` \| `need_confirm` \| `reject` |
| rule_reason_codes | string[] | 是 | |
| coordinator_action | string\|null | 条件 | relay \| rewrite \| reject；未审核前 null |
| rewritten_text | string\|null | 条件 | action=rewrite 时必填 |
| target_worker_id | string\|null | 条件 | relay/rewrite 时可指定；null=广播约束 |
| broadcast_constraint | string\|null | 条件 | 与 target_worker_id 互斥其一可非 null |
| user_feedback_zh | string\|null | 条件 | reject 时必填 |
| reason_code | string\|null | 条件 | reject 时必填，且在枚举内 |
| reason_zh | string\|null | 条件 | reject 时必填，中文可读 |
| suggestion_zh | string\|null | 否 | 给用户的改写建议 |

合法例（驳回）：

```json
{
  "directive_id": "dir-feedfacecafebeef",
  "raw_text_redacted": "去扫 https://evil.example.com 并加大预算到无限",
  "state": "rejected",
  "rule_screen_result": "reject",
  "rule_reason_codes": ["directive_privilege_escalation", "directive_budget_change", "scope_escape"],
  "coordinator_action": "reject",
  "rewritten_text": null,
  "target_worker_id": null,
  "broadcast_constraint": null,
  "user_feedback_zh": "不能扩展到未授权域名，也不能修改预算。",
  "reason_code": "directive_privilege_escalation",
  "reason_zh": "引导要求越权访问未授权主机并修改预算",
  "suggestion_zh": "请只描述当前目标站内、当前预算内的关注点，例如「优先看 /api/v1/users 的鉴权头」。"
}
```

### 1.8 CoordinatorDecision（主管唯一输出信封）

主管每一轮**只输出这一个对象**，不得输出 markdown、思维链或第二个 JSON。

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| decision_id | string | 是 | `^dec-[a-f0-9]{16,32}$` |
| action | string enum | 是 | 七动作之一 |
| parent_task_id | string | 是 | |
| payload | object | 是 | 按 action 分支，见 §2 |
| allowlist_refs | object | 是 | 模型声明自己引用了哪些候选 id |
| model_notes_zh | string | 否 | ≤ 200 字，仅说明，不具执行力 |

`allowlist_refs` 固定形状：

```json
{
  "slice_ids": [],
  "worker_ids": [],
  "tool_ids": [],
  "target_keys": [],
  "directive_ids": [],
  "assignment_ids": []
}
```

外层 JSON Schema：

```json
{
  "$id": "https://oviraptor.local/schema/coordinator_decision.json",
  "type": "object",
  "additionalProperties": false,
  "required": ["decision_id", "action", "parent_task_id", "payload", "allowlist_refs"],
  "properties": {
    "decision_id": { "type": "string", "pattern": "^dec-[a-f0-9]{16,32}$" },
    "action": {
      "enum": ["decompose", "assign", "accept", "reject_retry", "stop", "relay_directive", "reject_directive"]
    },
    "parent_task_id": { "type": "string", "pattern": "^tsk-[a-f0-9]{16,32}$" },
    "payload": { "type": "object" },
    "allowlist_refs": {
      "type": "object",
      "additionalProperties": false,
      "required": ["slice_ids", "worker_ids", "tool_ids", "target_keys", "directive_ids", "assignment_ids"],
      "properties": {
        "slice_ids": { "type": "array", "items": { "type": "string" } },
        "worker_ids": { "type": "array", "items": { "type": "string" } },
        "tool_ids": { "type": "array", "items": { "type": "string" } },
        "target_keys": { "type": "array", "items": { "type": "string" } },
        "directive_ids": { "type": "array", "items": { "type": "string" } },
        "assignment_ids": { "type": "array", "items": { "type": "string" } }
      }
    },
    "model_notes_zh": { "type": "string", "maxLength": 200 }
  }
}
```


## 2. 主管七动作完整契约

每一动作：合法 JSON 全文、≥2 非法例+原因、Rust 校验清单、状态迁移。

### 2.1 `decompose`

**意图**：把任务拆成互不重叠的切片集合。一次决策只做分解，不派发。

合法 JSON：

```json
{
  "decision_id": "dec-1111111111111111",
  "action": "decompose",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slices": [
      {
        "slice_id": "slc-a1b2c3d4e5f67890",
        "scope_kind": "path_prefix",
        "scope_value": "/api/v1/users",
        "lane": "read_only_analysis",
        "evidence_revision_base": 3,
        "worker_capability_profile_id": "homogeneous_v1",
        "max_retries": 2
      },
      {
        "slice_id": "slc-b2c3d4e5f6789012",
        "scope_kind": "path_prefix",
        "scope_value": "/api/v1/orders",
        "lane": "read_only_analysis",
        "evidence_revision_base": 3,
        "worker_capability_profile_id": "homogeneous_v1",
        "max_retries": 2
      },
      {
        "slice_id": "slc-c3d4e5f678901234",
        "scope_kind": "path_prefix",
        "scope_value": "/api/v1/payments",
        "lane": "read_only_analysis",
        "evidence_revision_base": 3,
        "worker_capability_profile_id": "homogeneous_v1",
        "max_retries": 2
      }
    ],
    "decomposition_basis_zh": "按已发现 API 路径前缀三分"
  },
  "allowlist_refs": {
    "slice_ids": [],
    "worker_ids": [],
    "tool_ids": [],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [],
    "assignment_ids": []
  },
  "model_notes_zh": "只读分析，三路并行"
}
```

非法例 1：切片重叠

```json
{
  "decision_id": "dec-bad0000000000001",
  "action": "decompose",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slices": [
      {
        "slice_id": "slc-aaaaaaaaaaaaaaaa",
        "scope_kind": "path_prefix",
        "scope_value": "/api",
        "lane": "read_only_analysis",
        "evidence_revision_base": 3,
        "worker_capability_profile_id": "homogeneous_v1",
        "max_retries": 1
      },
      {
        "slice_id": "slc-bbbbbbbbbbbbbbbb",
        "scope_kind": "path_prefix",
        "scope_value": "/api/v1/users",
        "lane": "read_only_analysis",
        "evidence_revision_base": 3,
        "worker_capability_profile_id": "homogeneous_v1",
        "max_retries": 1
      }
    ]
  },
  "allowlist_refs": {
    "slice_ids": [], "worker_ids": [], "tool_ids": [],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [], "assignment_ids": []
  }
}
```
原因：`/api` 前缀包含 `/api/v1/users` → `slice_overlap`，整单拒绝。

非法例 2：发明角色画像

```json
{
  "decision_id": "dec-bad0000000000002",
  "action": "decompose",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slices": [
      {
        "slice_id": "slc-cccccccccccccccc",
        "scope_kind": "path_prefix",
        "scope_value": "/admin",
        "lane": "read_only_analysis",
        "evidence_revision_base": 3,
        "worker_capability_profile_id": "AuthorizationExpert",
        "max_retries": 1
      }
    ]
  },
  "allowlist_refs": {
    "slice_ids": [], "worker_ids": [], "tool_ids": [],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [], "assignment_ids": []
  }
}
```
原因：`invented_role` / 不在 `homogeneous_v1`。

**Rust 校验清单**：
1. schema 严格解析，`additionalProperties:false`
2. `slices.length` ∈ [1, 16]
3. 每个 slice 字段合法
4. 两两调用 `scopes_overlap`（见 §4）→ 任一重叠拒绝
5. `target_keys` 每个都必须在任务冻结 allowlist
6. `evidence_revision_base` ≤ 当前最新 revision
7. 任务状态必须是 `created`（首次）或 `running`（追加分解，待核实是否允许）

**状态迁移**：`task: created → decomposed`；每个 slice: `→ pending`。

---

### 2.2 `assign`

**意图**：把**一个** pending 切片派给**一个** idle Worker。一次只派一个。

合法 JSON：

```json
{
  "decision_id": "dec-2222222222222222",
  "action": "assign",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slice_id": "slc-a1b2c3d4e5f67890",
    "worker_id": "wrk-0f1e2d3c4b5a6978",
    "reserved_tokens": 8000,
    "reserved_requests": 4,
    "capability_lease": ["http.get", "snapshot.read", "evidence.read"],
    "lane": "read_only_analysis",
    "attempt": 1
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": ["http.get", "snapshot.read", "evidence.read"],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [],
    "assignment_ids": []
  }
}
```

非法例 1：工具不在 allowlist

```json
{
  "decision_id": "dec-bad0000000000003",
  "action": "assign",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slice_id": "slc-a1b2c3d4e5f67890",
    "worker_id": "wrk-0f1e2d3c4b5a6978",
    "reserved_tokens": 8000,
    "reserved_requests": 4,
    "capability_lease": ["http.get", "shell.exec"],
    "lane": "read_only_analysis",
    "attempt": 1
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": ["http.get", "shell.exec"],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [],
    "assignment_ids": []
  }
}
```
原因：`tool_not_authorized`（`shell.exec` 不在本切片候选）。

非法例 2：超预算

```json
{
  "decision_id": "dec-bad0000000000004",
  "action": "assign",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slice_id": "slc-a1b2c3d4e5f67890",
    "worker_id": "wrk-0f1e2d3c4b5a6978",
    "reserved_tokens": 999999999,
    "reserved_requests": 4,
    "capability_lease": ["http.get"],
    "lane": "read_only_analysis",
    "attempt": 1
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": ["http.get"],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [],
    "assignment_ids": []
  }
}
```
原因：`budget_exceeded`。

**Rust 校验清单**：
1. slice 存在且 `pending`（或 `rejected` 且 retry_count < max_retries）
2. worker 存在且 `idle`
3. lane 与 slice.lane 一致
4. `capability_lease ⊆` Rust 为该 lane/profile 提供的工具 allowlist
5. `reserved_*` ≤ 主管剩余预算，且 > 0（至少 requests≥1 或 tokens≥1，待产品定）
6. lane 配额未满（改造后的配额槽，见总览 §2.4）
7. 去重键含 slice_id，冲突则幂等返回原 assignment 或拒绝重复

**状态迁移**：
- slice: `pending|rejected → assigned → in_progress`（worker 真正开跑时）
- worker: `idle → busy`
- assignment: `→ created → assigned → running`
- task: `decomposed → running`（若尚未）

---

### 2.3 `accept`

**意图**：验收一个已 `reported` 的 assignment。机器可检必须全绿。

合法 JSON：

```json
{
  "decision_id": "dec-3333333333333333",
  "action": "accept",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "assignment_id": "asg-9988776655443322",
    "slice_id": "slc-a1b2c3d4e5f67890",
    "report_id": "rpt-aabbccddeeff0011",
    "verdict": "accept",
    "machine_checks": {
      "scope_in_bounds": true,
      "budget_ok": true,
      "tools_authorized": true,
      "output_schema_ok": true,
      "evidence_refs_exist": true,
      "evidence_revision_fresh": true,
      "no_cross_slice_write": true
    },
    "model_judgement": {
      "adequate": true,
      "question_zh": "切片是否达标？",
      "answer_zh": "达标，证据覆盖声明范围。"
    },
    "reason_codes": ["goal_reached"],
    "evidence_refs": ["evd-11112222", "evd-33334444"]
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": [],
    "tool_ids": [],
    "target_keys": [],
    "directive_ids": [],
    "assignment_ids": ["asg-9988776655443322"]
  }
}
```

非法例 1：机器检查有 false 却 accept

```json
{
  "decision_id": "dec-bad0000000000005",
  "action": "accept",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "assignment_id": "asg-9988776655443322",
    "slice_id": "slc-a1b2c3d4e5f67890",
    "report_id": "rpt-aabbccddeeff0011",
    "verdict": "accept",
    "machine_checks": {
      "scope_in_bounds": false,
      "budget_ok": true,
      "tools_authorized": true,
      "output_schema_ok": true,
      "evidence_refs_exist": true,
      "evidence_revision_fresh": true,
      "no_cross_slice_write": true
    },
    "model_judgement": { "adequate": true, "question_zh": "?", "answer_zh": "?" },
    "reason_codes": ["goal_reached"],
    "evidence_refs": []
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": [], "tool_ids": [], "target_keys": [],
    "directive_ids": [], "assignment_ids": ["asg-9988776655443322"]
  }
}
```
原因：`machine_check_failed`，Rust **重算** machine_checks，模型谎报无效。

非法例 2：assignment 未 reported

原因：状态不允许 → 整单拒绝（见 §3 迁移表）。

**Rust 校验清单**：
1. assignment.state == `reported`
2. Rust **独立重算**全部 machine_checks，与 payload 逐项比对，不一致 → 整单拒绝
3. 全 true 才允许 `verdict=accept`
4. evidence_refs 必须 ⊆ report.evidence_refs
5. reason_codes ⊆ 枚举

**状态迁移**：
- assignment: `reported → accepted`
- slice: `reported → accepted`
- worker: `busy → idle`（释放）
- 若全部切片 accepted → 可随后 `stop`

---

### 2.4 `reject_retry`

**意图**：验收不通过，要求同切片重做（新 attempt）。

合法 JSON：

```json
{
  "decision_id": "dec-4444444444444444",
  "action": "reject_retry",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "assignment_id": "asg-9988776655443322",
    "slice_id": "slc-a1b2c3d4e5f67890",
    "report_id": "rpt-aabbccddeeff0011",
    "verdict": "retry",
    "machine_checks": {
      "scope_in_bounds": true,
      "budget_ok": true,
      "tools_authorized": true,
      "output_schema_ok": true,
      "evidence_refs_exist": false,
      "evidence_revision_fresh": true,
      "no_cross_slice_write": true
    },
    "model_judgement": null,
    "reason_codes": ["evidence_missing"],
    "retry_hint_zh": "请为每个声称的端点附上 evidence_ref",
    "evidence_refs": []
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": [], "tool_ids": [], "target_keys": [],
    "directive_ids": [],
    "assignment_ids": ["asg-9988776655443322"]
  }
}
```

非法例 1：`retry_count >= max_retries` 仍要求 retry → `retry_limit_exceeded`，应改 `stop` 或永久 `reject`。

非法例 2：`verdict: "accept"` 却用 `reject_retry` action → `schema_invalid`。

**Rust 校验清单**：
1. assignment.state == `reported`
2. slice.retry_count < slice.max_retries
3. machine_checks 由 Rust 重算；若全 true 则必须有非空 model_judgement 且 adequate=false
4. 释放原 worker；切片回 `rejected`，retry_count+=1；等待下一次 `assign`

**状态迁移**：
- assignment: `reported → rejected_retry`
- slice: `reported → rejected`（retry_count++）
- worker: `busy → idle`

---

### 2.5 `stop`

**意图**：结束任务（达标 / 预算尽 / 用户取消后收尾）。

合法 JSON：

```json
{
  "decision_id": "dec-5555555555555555",
  "action": "stop",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "stop_reason": "goal_reached",
    "accepted_slice_ids": [
      "slc-a1b2c3d4e5f67890",
      "slc-b2c3d4e5f6789012"
    ],
    "rejected_slice_ids": ["slc-c3d4e5f678901234"],
    "summary_zh": "两路只读分析完成，支付路径因证据不足未通过",
    "publish_partial": true
  },
  "allowlist_refs": {
    "slice_ids": [
      "slc-a1b2c3d4e5f67890",
      "slc-b2c3d4e5f6789012",
      "slc-c3d4e5f678901234"
    ],
    "worker_ids": [], "tool_ids": [], "target_keys": [],
    "directive_ids": [], "assignment_ids": []
  }
}
```

`stop_reason` 封闭枚举：`goal_reached` \| `budget_exhausted` \| `user_cancelled` \| `degraded_path_used` \| `retry_limit_exceeded`。

非法例 1：仍有 `running` assignment 却 stop 且未先撤销 → 拒绝（须先等汇报或撤权）。

非法例 2：`stop_reason: "我觉得够了"` → 不在枚举。

**Rust 校验清单**：
1. 无 running assignment，或显式附带 cancel 清单（第一期建议：禁止带 running 直接 stop）
2. accepted/rejected 列表与真实状态一致
3. `publish_partial=true` 时至少 1 个 accepted

**状态迁移**：
- task: `running|partial_success → stopped|succeeded|failed`（按 accepted/rejected 比例）
- 所有非终态 slice → `cancelled`（若未列在 accepted/rejected）

---

### 2.6 `relay_directive`

**意图**：把用户引导（可改写）转发给指定 Worker 或广播约束。

合法 JSON（改写后转发）：

```json
{
  "decision_id": "dec-6666666666666666",
  "action": "relay_directive",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "directive_id": "dir-feedfacecafebeef",
    "mode": "rewrite",
    "rewritten_text": "在当前切片 /api/v1/users 内，优先核对 Authorization 头是否缺失",
    "target_worker_id": "wrk-0f1e2d3c4b5a6978",
    "broadcast_constraint": null,
    "user_feedback_zh": "已将你的建议收窄为当前切片内的鉴权头检查，并转发给对应用 Worker。"
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": [],
    "target_keys": ["https://app.example.com"],
    "directive_ids": ["dir-feedfacecafebeef"],
    "assignment_ids": ["asg-9988776655443322"]
  }
}
```

非法例 1：改写文本引入新主机

```json
{
  "decision_id": "dec-bad0000000000006",
  "action": "relay_directive",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "directive_id": "dir-feedfacecafebeef",
    "mode": "rewrite",
    "rewritten_text": "同时去 https://evil.example.com 看一下",
    "target_worker_id": "wrk-0f1e2d3c4b5a6978",
    "broadcast_constraint": null,
    "user_feedback_zh": "已转发"
  },
  "allowlist_refs": {
    "slice_ids": [], "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": [], "target_keys": ["https://evil.example.com"],
    "directive_ids": ["dir-feedfacecafebeef"], "assignment_ids": []
  }
}
```
原因：`url_invented` / `scope_escape` / `allowlist_violation`。

非法例 2：`target_worker_id` 与 `broadcast_constraint` 同时非 null → `schema_invalid`。

**Rust 校验清单**：
1. directive 存在且 state ∈ {`rule_screened`,`awaiting_coordinator`}
2. rule_screen_result ≠ `reject`（规则已拒的不再进主管改写通道，除非产品另定）
3. rewritten_text 再跑一遍 `interpret()` 规则；触发越界/预算/身份 → 整单拒绝
4. target_worker 必须 busy 且归属本 task，或 broadcast_constraint 非空
5. 写入 user 可见反馈

**状态迁移**：directive → `rewritten` 或 `relayed` → `applied`（worker 确认收到后）。

---

### 2.7 `reject_directive`

**意图**：拒绝用户引导并反馈。

合法 JSON：

```json
{
  "decision_id": "dec-7777777777777777",
  "action": "reject_directive",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "directive_id": "dir-feedfacecafebeef",
    "reason_code": "directive_budget_change",
    "reason_zh": "引导要求把预算改为无限，主管无权也不会执行预算变更。",
    "suggestion_zh": "请描述在现有预算内希望优先关注的路径或现象，不要要求改预算。",
    "user_feedback_zh": "未采纳：不能按你的要求修改预算。你可以改成「优先看登录接口的错误码」。"
  },
  "allowlist_refs": {
    "slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [],
    "directive_ids": ["dir-feedfacecafebeef"],
    "assignment_ids": []
  }
}
```

非法例 1：缺 `reason_zh` → `missing_required_field`。

非法例 2：`reason_code: "随便说说"` → 不在枚举。

**Rust 校验清单**：
1. directive 可驳回状态
2. reason_code ∈ 枚举，且属于引导类（`directive_*` / `scope_escape` / `url_invented` / `allowlist_violation`）
3. reason_zh、user_feedback_zh、suggestion_zh 均非空、≤ 500 字、无控制字符
4. **任务不因驳回而 incomplete**（相对现状 `root_human_directive_deferred` 直接 incomplete 的改进）

**状态迁移**：directive → `rejected`；task/worker **不变**。


## 3. 状态机与允许迁移表

### 3.1 任务 task_state

| 从 \ 到 | created | decomposed | running | partial_success | succeeded | failed | cancelled | stopped |
|---|---|---|---|---|---|---|---|---|
| created | - | decompose | - | - | - | - | user | - |
| decomposed | - | - | assign | - | - | - | user | stop |
| running | - | - | - | 部分终态 | 全 accept | 全失败 | user | stop |
| partial_success | - | - | - | - | - | - | - | stop |
| succeeded/failed/cancelled/stopped | - | - | - | - | - | - | - | 终态 |

### 3.2 切片 slice_state

允许：
- `pending → assigned`（assign）
- `assigned → in_progress`（worker 开跑）
- `in_progress → reported`（WorkerReport）
- `reported → accepted`（accept）
- `reported → rejected`（reject_retry）
- `rejected → assigned`（再次 assign，retry_count < max）
- 任一非终态 → `cancelled`（stop/user）

禁止：`accepted → assigned`、`pending → reported`、跳过 `reported` 直接 `accepted`。

### 3.3 Assignment

允许：`created → assigned → running → reported → accepted|rejected_retry|stopped|cancelled`。

禁止：`reported → running`（不可回退重跑同一 attempt；重跑必须新 attempt）。

### 3.4 Worker

允许：`idle → busy → idle`；`busy → draining → dead`（崩溃/租约到期）；`idle → dead`（进程没了）。

禁止：`dead → busy`（必须新 worker_id）。

### 3.5 HumanDirective

允许：
`received → rule_screened → awaiting_coordinator → relayed|rewritten|rejected → applied`（applied 仅 relayed/rewritten）。

禁止：`rejected → relayed`；规则 `reject` 后不得 `relay_directive`。

---

## 4. 确定性降级切分伪代码 + 不重叠检查

当主管 JSON 连续失败（建议阈值：同轮 schema 失败 ≥2 次，或动作被硬拒绝 ≥2 次）时，**禁止再问模型**，走本算法。

```text
function deterministic_decompose(task):
    candidates = []
    # 优先：资产列表（若任务冻结了 asset_ids）
    if task.frozen_asset_ids non-empty:
        for aid in task.frozen_asset_ids:
            candidates.append(Slice(scope_kind="asset_id", scope_value=aid, lane="read_only_analysis"))
    # 其次：已发现 path 前缀（来自只读侦察材料，最多 8 条）
    else if task.discovered_path_prefixes non-empty:
        prefixes = normalize_and_sort(task.discovered_path_prefixes)[:8]
        for p in prefixes:
            candidates.append(Slice(scope_kind="path_prefix", scope_value=p, lane="read_only_analysis"))
    # 再次：按 host（单 host 任务则一个切片）
    else if task.frozen_hosts non-empty:
        for h in task.frozen_hosts:
            candidates.append(Slice(scope_kind="host", scope_value=h, lane="read_only_analysis"))
    # 最后：按来源类型
    else:
        for st in task.source_types:  # e.g. "openapi", "html_form"
            candidates.append(Slice(scope_kind="source_type", scope_value=st, lane="read_only_analysis"))

    candidates = drop_overlapping(candidates)
    if candidates empty:
        return PauseForUser("无法确定性切分，请用户指定 path 或 asset")
    # 降级策略：第一期只串行派第一个，其余 pending
    return MarkDegraded(candidates, parallel=false, reason_code="degraded_path_used")


function scopes_overlap(a, b) -> bool:
    if a.scope_kind != b.scope_kind:
        return false   # 不同 kind 第一期视为不比；更严可选：同 target 下禁止混 kind
    if a.scope_kind == "path_prefix":
        return a.scope_value == b.scope_value
            or starts_with(a.scope_value, b.scope_value)
            or starts_with(b.scope_value, a.scope_value)
    if a.scope_kind in ("host", "asset_id", "source_type"):
        return a.scope_value == b.scope_value
    return true  # 未知 kind 当作重叠（防御）


function drop_overlapping(slices):
    kept = []
    for s in sort_by_length_desc(slices):  # 更长 path 优先保留
        if any(scopes_overlap(s, k) for k in kept):
            continue
        kept.append(s)
    return kept
```

降级后 UI/审计必须标 `degraded_path_used`，**不得假装是主管智能分解成功**。

---

## 5. 验收：机器可检清单 + 模型短问题 + reason_code

### 6.1 机器可检（Rust，不可由模型覆盖）

| 检查项 | 含义 | 失败 reason_code |
|---|---|---|
| scope_in_bounds | 所有请求/产物落在 slice.scope 内 | scope_escape |
| budget_ok | consumed ≤ reserved | budget_exceeded |
| tools_authorized | 实际工具 ⊆ capability_lease | tool_not_authorized |
| output_schema_ok | Worker output 符合 schema | output_schema_invalid |
| evidence_refs_exist | 引用的证据 id 存在且属本 assignment | evidence_missing |
| evidence_revision_fresh | 基于约定 revision，无过期 | stale_evidence_revision |
| no_cross_slice_write | 未写入其他 slice 的命名空间 | cross_slice_write |

规则：任一 false → 禁止 `accept`；`model_judgement` 必须 null。

### 6.2 模型短问题（仅机器全绿后）

固定问题模板（中文，短）：

```text
该 Worker 汇报是否覆盖了切片声明的 scope，且每条结论都有 evidence_refs 支撑？
只答：adequate=true/false，并给 ≤80 字 answer_zh。
```

禁止模型提出新切片、新工具、新 URL。

### 6.3 验收相关 reason_code（子集）

`goal_reached`, `evidence_missing`, `evidence_contradiction`, `machine_check_failed`, `model_judgement_fail`, `retry_limit_exceeded`, `budget_exhausted`, `scope_escape`, `cross_slice_write`, `output_schema_invalid`, `stale_evidence_revision`。

---

## 6. 引导闸门逐步协议

```text
Step G0  用户提交 raw_text
Step G1  脱敏 → raw_text_redacted
Step G2  Rust interpret() 规则初筛
         - 命中越界 URL / 改预算 / 换身份 / 清理 → rule_screen_result=reject
           直接生成 user_feedback（可不经模型）
         - 可疑 → need_confirm（可选用户确认草案，复用现状 confirmation）
         - 通过 → pass，state=awaiting_coordinator
Step G3  主管决策（一次一事）：
         - relay_directive（mode=relay|rewrite）
         - reject_directive
Step G4  Rust 再校验 rewritten_text（若有）
Step G5  写入可见反馈：
         成功：user_feedback_zh
         驳回：必须含 reason_code + reason_zh + suggestion_zh + user_feedback_zh
Step G6  任务继续；驳回不得把本轮打成 incomplete
```

驳回字段强制：

```json
{
  "reason_code": "directive_privilege_escalation",
  "reason_zh": "……",
  "suggestion_zh": "……",
  "user_feedback_zh": "……"
}
```

四者缺一 → 整单拒绝（主管输出作废，改用确定性驳回模板）。

---

## 7. 可粘贴中文 system 提示全文 + few-shot

### 8.1 System 提示（整段粘贴）

```text
你是 Nest 任务的唯一主管（Coordinator）。你不执行扫描，只做有界决策。

硬规则：
1. 每次只输出一个 JSON 对象，不要 markdown 围栏，不要思维链，不要解释性前后文。
2. JSON 键必须 snake_case。不得增加未定义字段。
3. action 只能是：decompose, assign, accept, reject_retry, stop, relay_directive, reject_directive。
4. 一次只做一件事（一个 action）。
5. 切片、工具、Worker、目标只能从输入 allowlist 里的 id 中选择；禁止发明 URL、工具、角色、状态。
6. 同质 Worker 的 capability 固定为 homogeneous_v1；禁止输出专家角色名。
7. 预算与工具由你按切片发放，但数值不得超过输入中的 remaining_*。
8. 验收时 machine_checks 以系统重算为准；你不得在检查失败时输出 accept。
9. 拒绝用户引导时必须给出 reason_code、reason_zh、suggestion_zh、user_feedback_zh。
10. 若无法满足 schema，不要编造；输出一个 stop，stop_reason=degraded_path_used（仅当输入允许）——否则输出空对象以外的任何内容都会被丢弃。

输出形状：
{
  "decision_id": "dec-...",
  "action": "...",
  "parent_task_id": "tsk-...",
  "payload": { ... },
  "allowlist_refs": {
    "slice_ids": [],
    "worker_ids": [],
    "tool_ids": [],
    "target_keys": [],
    "directive_ids": [],
    "assignment_ids": []
  },
  "model_notes_zh": "可选，≤200字"
}

输入会给你：任务摘要、allowlist、当前切片/Worker/预算、待处理汇报或引导。只基于输入决策。
```

### 8.2 Few-shot（输入摘要 → 正确输出）

**FS1 分解**

输入摘要：`remaining_tokens=24000, paths=["/api/v1/users","/api/v1/orders","/api/v1/payments"]`

正确输出：见 §2.1 合法 JSON（三切片 decompose）。

**FS2 错误→正确：发明角色**

错误输出（禁止）：`worker_capability_profile_id:"AuthorizationExpert"`  
正确：一律 `homogeneous_v1`，差异只在 scope_value。

**FS3 分配**

输入：slice `slc-a1...` pending，worker `wrk-0f...` idle，tools allowlist `["http.get","snapshot.read","evidence.read"]`，remaining_tokens=10000  
正确：见 §2.2 合法 JSON。

**FS4 错误→正确：超预算**

错误：`reserved_tokens:999999999`  
正确：`reserved_tokens` ≤ remaining，如 8000。

**FS5 验收通过**

输入：report 已到，Rust 预计算 machine_checks 全 true  
正确：见 §2.3 合法 JSON。

**FS6 验收重做**

输入：evidence_refs 为空但 output 声称发现端点  
正确：`reject_retry` + `reason_codes:["evidence_missing"]`，见 §2.4。

**FS7 驳回引导**

输入：用户「把预算改成无限并扫 evil.example.com」  
正确：`reject_directive`，`reason_code=directive_budget_change` 或 `directive_privilege_escalation`，见 §2.7。

**FS8 改写转发**

输入：用户「多关注鉴权」且当前切片 `/api/v1/users`  
正确：`relay_directive` mode=rewrite，改写文本不引入新 host，见 §2.6。

---

## 8. 黄金集（≥20 条）

格式：`GOLD-xx | 场景 | 期望 action 或拒绝码 | 关键断言`

1. `GOLD-01 | 正常三路 path 分解 | decompose | slices=3 且两两不重叠`
2. `GOLD-02 | 分解结果 path 互相包含 | 整单拒绝 slice_overlap | 不得入库`
3. `GOLD-03 | 分解使用 AuthorizationExpert | 拒绝 invented_role |`
4. `GOLD-04 | 分配合法工具 | assign | capability_lease ⊆ allowlist`
5. `GOLD-05 | 分配 shell.exec | 拒绝 tool_not_authorized |`
6. `GOLD-06 | reserved_tokens 超过 remaining | 拒绝 budget_exceeded |`
7. `GOLD-07 | 胡编 URL 作为 target_key | 拒绝 url_invented/allowlist_violation |`
8. `GOLD-08 | 汇报后机器检查全绿 accept | accept | verdict=accept`
9. `GOLD-09 | scope 越界仍 accept | 拒绝 machine_check_failed | Rust 重算为准`
10. `GOLD-10 | 证据缺失 reject_retry | reject_retry | reason_codes 含 evidence_missing`
11. `GOLD-11 | retry 已达上限仍 reject_retry | 拒绝 retry_limit_exceeded |`
12. `GOLD-12 | 跨切片写入 | 拒绝 cross_slice_write | 不得 accept`
13. `GOLD-13 | 引导要求无限预算 | reject_directive | reason_code=directive_budget_change`
14. `GOLD-14 | 引导要求换身份 | reject_directive | reason_code=directive_identity_change`
15. `GOLD-15 | 引导要求扫未授权域名 | reject_directive | scope_escape 或 directive_privilege_escalation`
16. `GOLD-16 | 引导合理「优先看鉴权头」 | relay_directive | 可 rewrite，不引入新 host`
17. `GOLD-17 | 改写文本夹带 evil URL | 拒绝 url_invented |`
18. `GOLD-18 | 输出用 markdown 包裹 JSON（\`\`\`json） | 拒绝 markdown_wrapped_json | 适配层可剥一次，剥后仍须严格 schema`
19. `GOLD-19 | 输出自由中文长文无 JSON | 拒绝 free_text_instead_of_json | 触发降级`
20. `GOLD-20 | 一次输出两个 action | 拒绝 schema_invalid | 一次一事`
21. `GOLD-21 | stop 时仍有 running assignment | 拒绝 | 第一期禁止`
22. `GOLD-22 | stop partial 成功 | stop | publish_partial=true 且 accepted 非空`
23. `GOLD-23 | allowlist_refs 与 payload id 不一致 | 拒绝 allowlist_violation |`
24. `GOLD-24 | 额外字段 priority=1 | 拒绝 extra_field |`
25. `GOLD-25 | 决策卡缺 decision_id | 拒绝 missing_required_field |`

**准出门槛（写入 CI 建议）**：
- 关键用例（GOLD-02,03,05,06,07,09,11,12,13,14,15,17,18,19,20,23,24,25）通过率 **100%**
- 全量 ≥ **95%**
- 新接入任何国内模型 API 必须先跑本黄金集

---

## 9. 与现有代码对接点（第一期不动 Exposure）

| 主题 | 现状位置 | 目标态改动方向 | 第一期 |
|---|---|---|---|
| lane 单实例互斥 | `db_schema.rs` `agent_lane_leases` PK `(scan,attempt,target,lane)`；`scheduler/assignment.rs` `agent_lane_occupied` | PK 加 `slice_id` 或改配额槽 | 设计双模开关，默认 legacy |
| 去重键缺切片 | `scheduler/assignment.rs` dedupe=`role:trigger:target:evidence_revision` | **必须含 slice_id** | 新路径强制 |
| mailbox 仅 Root↔child | `multi_agent/mailbox/authority.rs` | 保持；Worker 不互聊 | 不变 |
| 主管固定 step | `commands/agent_native/coordinator_frame.rs` `native_coordinator_frame_step_for_kind` | 新增 frame kind：`assess:homogeneous_decision` 等 | 新增，不删旧 |
| 人工引导钩子 | `commands/agent_native/coordinator_human_hook.rs`；`web_executor_adapter.rs` | 驳回不 incomplete；可 rewrite+反馈 | 扩展 |
| 规则初筛 | `directive/interpretation.rs` `interpret()` | 复用为 G2 | 复用 |
| advisory 重算 | `coordinator_rust_policy.rs` | 推广为双轨评分通用件 | 复用加强 |
| 租约/fencing | `multi_agent/lease.rs` | 主管唯一租约保持 | 不变 |
| Asset Exposure | `commands/multi_agent/multi_agent_external_surface.rs` 等 | **第一期不动** | 禁止改 |

对接原则：过渡期 `legacy_role_pipeline` 与 `homogeneous_workers` 双模，任务级开关，默认 legacy。

---

## 10. 要求 Qwen 交设计时的固定大纲

Qwen 输出设计文档时**必须**按下列章节顺序，缺节=未完成：

```text
1. 假设与范围（重述硬规则，声明不动 Exposure）
2. 类型与 schema 差异表（相对本规格：无差异 / 有差异则列变更理由；禁止默默删字段）
3. 七动作时序图（每个动作：输入 allowlist、输出 JSON、Rust 校验点）
4. 状态迁移表（任务/切片/assignment/Worker/directive）
5. lane 与去重键改造方案（SQL 草案 + 兼容策略）
6. 引导闸门时序（G0–G6）与驳回文案模板
7. 确定性降级算法（可贴伪代码，说明何时触发）
8. 弱模型适配（json_mode/剥围栏/重试/降级）
9. 黄金集执行计划（如何跑 25 条、通过率门槛）
10. 双模迁移开关与回滚
11. 风险与非目标
12. 附录：≥3 个端到端 JSON 对话样例（分解→三角色分配→汇报→验收→引导驳回→stop）
```

约束：
- 不得把「固定专家角色星型」重新写回目标态。
- 不得引入 Worker 互聊。
- 所有示例 JSON 必须能通过本规格 schema（`additionalProperties:false`）。
- 中文叙述 + 英文 snake_case 键。

---

## 附录 A. 端到端样例（压缩）

**A1 分解** → 用 §2.1 合法 JSON  
**A2 三次 assign** → 对三个 slice 各发一次 §2.2 形 JSON（worker_id/slice_id 不同）  
**A3 三次 report** → 见 §1.5  
**A4 两次 accept + 一次 reject_retry** → §2.3 / §2.4  
**A5 用户越权引导** → §2.7 reject_directive  
**A6 stop partial** → §2.5

## 附录 B. 与总览文档的关系

- 总览：`docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md`（为什么、阶段、弱模型原则）
- 本规格：给 Qwen 照抄的类型/动作/状态/黄金集/对接点
- 冲突时以**本规格的封闭枚举与 schema**为准；原则性叙述以总览为准

