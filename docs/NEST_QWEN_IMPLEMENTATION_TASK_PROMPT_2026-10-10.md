# 给 Qwen 的实现任务话术（直接复制下方代码块）

> 分工：**Grok 已写完设计定稿**；你（Qwen）**只补代码与测试**，不要写设计、不要改设计结论。
> 复制时从「任务：」开始，到最后一个句号结束。

```
任务：按仓库内已定稿的设计与规格，实现 Nest「同质 Worker + 强主管」调度的 Phase 1（或我另行指定的 phase），干到设计文档勾选的 Phase 1 完成定义满足为止，不要中途停下来问要不要继续，不要产出新的设计文档。

必读（冲突裁定：DESIGN > SPEC > ARCHITECTURE）：
1. docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md（设计定稿，实现主依据）
2. docs/NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md（封闭枚举 / JSON Schema / 七动作契约 / 黄金集）
3. docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md（背景）
辅助对照（只读）：docs/NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md
忽略：docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md（错误分工产物，不作依据）

你的任务是补代码与测试，不是写设计。

Phase 1 范围（以 DESIGN §0.1 勾选为准，摘要）：
- 新建 agent_runtime/multi_agent/homogeneous/ 模块（types/decision/allowlist/gate/apply/machine/decompose/directive_gate/store）
- 任务级 scheduling_model，默认 legacy_role_pipeline；仅显式 homogeneous_workers 走新路径
- 新表与 agent_assignments 增列（见 DESIGN §4）；read_only_analysis 配额并行；dedupe 含 slice_id
- 七动作严格 snake_case 解析与落库；确定性降级
- 引导闸门：可 rewrite/reject 并四字段回用户；驳回不得 incomplete
- 测试覆盖黄金集关键用例 100%，全量尽量 ≥95%
- 接线：scheduler/assignment 分支、coordinator_frame 新 kind、rust_policy、human_hook、web_executor_adapter（均按 DESIGN §5，legacy 行为保持）

硬禁止：
- 禁止发明动作/角色/工具/状态/reason_code；只能用 SPEC §1.1 封闭集合
- 禁止改 exposure / Asset Exposure 业务代码（DESIGN 附录 D 黑名单）
- 禁止 commit、禁止 push、禁止碰用户真实数据库（测试只用临时库）
- 禁止把 DecisionSummary（camelCase）改成 snake_case 复用；新决策结构独立
- 禁止打开 target_touching/review 的同质并行（Phase 1）
- 不确定写「待核实」并读代码确认，禁止编造「已实现」假证据

工作方式：
- 按 DESIGN §11 的 PR-A→PR-G 顺序推进（可在同一工作区连续做，不必真的 commit）
- 每完成一块就跑相关 cargo test；失败先修再往下
- 保持 legacy 路径默认可用、测不红

完成定义（DESIGN §13）：
- DESIGN §0.1 清单全部勾完
- 关键 GOLD（02,03,05,06,07,09,11,12,13,14,15,17,18,19,20,23,24,25）100%
- diff 不含 exposure 业务路径
- 然后停止

做完后在对话里只用一句话汇报：改动路径列表 + 测试命令与通过结果。然后交给窃蛋龙改造审核。不要自行宣布验收通过。
```
