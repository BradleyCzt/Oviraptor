# 归属门禁 O0 / O1 核验收口（2026-10-08）

依据：`NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md`（sha256 6a7de5ac…，与设计时一致，未改动）§1、§4、§5、§11 第 1–2 步、§12。基线 HEAD 14a7875，未 commit/push/stash/reset/checkout；未写真实用户库，未改任何真实资产的归属/授权。O2–O4 未开始。O1 细节见 `NEST_OWNERSHIP_GATE_O1_DELIVERY_2026-10-08.md`。

## 1. O0 当前页面正确性（§11-1）

| 设计要求 | 实现位置 | 证据 |
| --- | --- | --- |
| 项目响应代次：projectId + generation，过期响应丢弃 | `AssetOwnershipGate.vue` projectGeneration/profileGeneration/queueGeneration、`isCurrent` | gate 测试“late response… never fills the next project page” |
| 草稿保护：筛选/翻页/评估/批量结论不重填档案；按项目保留草稿（切视图卸载后仍在） | `loadProfile` 只在进入项目时调用；模块级 `ownershipProfileDrafts` | gate 测试 queue reloads / remount / per project |
| 档案保存 CAS：expectedVersion，冲突整批拒绝并保留草稿 | `save_ownership_profile_for_connection`（IMMEDIATE 事务，`OWNERSHIP_PROFILE_VERSION_CONFLICT`）；前端“放弃草稿/保留草稿改基于最新” | Rust `profile_save_requires_the_version_the_page_loaded`；gate CAS 测试 |
| 批量结论：项目一致、未删除成员资格、确认授权要求仍在复核范围、逐行结论版本一致，任一不满足整批不写 | `apply_asset_ownership_decisions`（`OWNERSHIP_DECISION_PRECONDITION_FAILED`）；前端带 expectedProjectId + 每行 expectedDecisions，切项目清空选择 | Rust `decisions_*` 3 项；gate bulk/切换/拒绝后重载测试 |
| 旧“确认归属并授权”影响说明 | 按钮 title、常驻说明、确认对话框逐项列出“归属=确认、授权=允许、暴露面准入=是”，以及学习域名规则会匹配子域 | gate 测试“honest about … the granting button” |
| 项目标题不预填法人名称 | `read_ownership_profile` 未保存时 legal_name 为空 | O1 测试 `saved_profile_wins_over_directory_and_title_is_never_a_legal_name` |

O0 未拆分按钮语义（归属与授权仍是同一动作），按 §8/§11 留给 O2/O3。

## 2. O1 项目预填闭环（§11-2）

唯一组织目录（`org_*` / `project_org_bindings`，纯新增表）、3 个 IPC（bootstrap / suggestions 只读 / 目录保存 CAS）、确定规则（省公司按法定名称结构、括号城市仅为所在地线索、CIDR/IP/ASN/域名型 company 不生成法人）、目录线索接入 assess 但结果仍为 attributed + 授权 unknown + 不准入、前端 `OwnershipProjectSetup.vue` 接受建议只进草稿、按项目草稿与代次丢弃。真实库标签抽样（项目 3：中移（苏州）/（杭州）、中移物联网、中国移动通信集团江苏/浙江有限公司、中国移动香港有限公司；项目 15：CIDR 与 cmi.chinamobile.com）均在 fixture 覆盖的规则分支内。

## 3. §12 与 O0/O1 相关条目

| §12 条目 | 状态 |
| --- | --- |
| 已确认目录的集团项目不需逐个输入省市；新别名有出处；保存冲突不覆盖草稿 | O1 满足（`confirmed_directory_is_reused…`、sourceRef 带 project/rows/examples/dataRevision、目录与档案均 CAS 保草稿） |
| 苏州/杭州专业子公司不按地名归省公司；CIDR/ASN 不当法人；管理归属与服务地域分开 | O1 满足（`label_rules_keep_location_clues_and_network_labels_apart`；region_code 仅管理单位地域，jurisdictions 不从目录复制） |
| 项目 3 切到 15 后旧请求不能写入新页面/批量操作；共享目录不共享授权 | O0+O1 满足（前端代次 + 后端 expectedProjectId；`assessment_uses_project_scope_of_directory_without_granting_authorization`） |
| com.cn / 多租户 / 同主机多单位不被根域整组放行 | 部分：目录域名只按精确主机；旧档案“确认根域名”与“学习域名规则”仍按子域匹配（已在对话框说明）。整组放行问题属于 O2 组合同 |
| 10 万行、组快照、批量幂等、旧预览失效、扫描权限分离、取消/崩溃恢复 | O2–O4，未开始 |

## 4. 本轮验证（北京时间）

- `cargo test --lib`（18:27 开始，约 42.5 分钟，二进制含全部 O0/O1 改动，之后源码无变更）：2900 通过 / 21 失败。21 项均为 `commands::agent_tests` 的 Native 多智能体预算/监督测试（multi_agent_executor_budget_unavailable、web_start_paid_history…、root_supervision_request_capacity_exhausted 等）。单独重跑：19 项稳定失败、2 项（protection_recovery_approval_concurrent…、single_finally_exit_overhard…）通过（负载相关）。这些测试文件及 agent_runtime 均不引用 asset_ownership_decisions / org_* / ownership_view，与本轮改动无关，属并行 Native 工作。
- 同一二进制 `ownership` 过滤：29/29 通过（含 O0 5 项、O1 7 项、旧 asset_ownership 4 项、exposure clusters 归属隔离 1 项）。
- `cargo clippy --lib --tests`：O0/O1 文件 0 告警。`ownership_directory.rs` 已重新 rustfmt（仅空白），其后重新编译 `cargo test --lib ownership` 仍为 29/29。
- 前端：`node --test tools/test_asset_ownership_gate.cjs tools/test_ownership_project_setup.cjs` 17/17；`npx vue-tsc --noEmit` 通过；`npm run test:asset-exposure` 59/59（确认 api.ts 改动未破坏 exposure 页面测试）。

## 5. 需要用户决定的边界项

`src-tauri/src/commands/assets.rs` 的 `ownershipView="eligible"` 过滤由“exposure_eligible=1 AND authorization_status='allowed' AND status='confirmed'”改为“exposure_eligible=1”，目的是让门禁“暴露面准入”卡片数字与列表总数一致（与 `exposure_summary`、`save_exposure_finding`、exposure scope 的口径相同）。但该过滤也被暴露面页面的“绑定准入资产”列表使用（HEAD 的 `ExposureSurface.vue` 和 Qoder 的 `useExposureSurface.ts`）：历史上 exposure_eligible=1 但非 confirmed/allowed 的异常行会因此出现在该下拉列表中（后端保存线索本来就只校验 exposure_eligible=1）。这超出了 O0 清单原文，按用户要求需由用户确认：A. 保留（三处口径统一）；B. 回退为旧口径，并把门禁卡片改为同样的 confirmed+allowed+eligible 口径（只改 asset_ownership.rs）。确认前未改动。

## 6. 未完成 / 后续

- 未对真实库做 O1 只读试跑（需要新增只读工具或测试入口，未擅自添加）。
- 目录只支持新增；改名/停用/合并/导入未做。归属与授权按钮未拆分；学习域名规则仍按子域匹配。上述归 O2/O3。
- 两个前端测试文件未加入 package.json 脚本（package.json 为共享文件，未改）。
