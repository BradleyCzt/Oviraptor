# Project 级应用模块化审计与分批迁移矩阵

> 后续进展：同日第二轮已实现单一主应用内的可信插件注册、业务工作空间抽取及聊天关联入口，见 [内置插件重构说明](BUILTIN_PLUGIN_HOST_2026-10-13.md)。本审计的基线与第一批验证记录保留，不应把第一批“尚未完成”误读为后续最新状态。

日期：2026-10-13（UTC）  
基线：`94cdb8a47b2ec4164c6e6c7b8297835fa579fcf4`  
审计分支：`refactor/project-scoped-applications-20261013`

## 1. 结论与证据边界

可以在保留现有功能、数据和执行安全的前提下，将产品组织为共享 Project 下的 **Chat、Assets、Scanning** 三个独立应用。现有代码已经有部分 feature API 和领域目录，适合渐进迁移；目前还不是三个独立应用。

最重要的五项事实：

1. `assets.asset_key` 全局唯一，项目关系、人工判定和软删除属于 `project_assets`。不需要另造一份“项目资产库”，也不能用新 `project_id` 列替换现有多对多归属。
2. 现有“调查对话”属于 Scanning 的运行控制与证据投影。`agent_messages.run_id → agent_runs.id → sentinel_scans.id` 是真实执行链；它不是通用聊天存储。新增 Chat 必须有独立 session/message 生命周期。
3. 无 LLM 不等于离线，也不等于无授权：Assets 本地管理、原生采集/分层/探测、规则归属、公开暴露面、HackerOne 同步均没有业务上的 LLM 前置条件，但采集 API、目标网络、凭据、范围和速率约束仍然存在。
4. 当前 Web **创建草稿**不要求模型；Web **确认启动**以及 Code/Greybox/CI **启动**均要求模型。不能仅移除界面检查，就宣称已有扫描执行支持无模型模式。
5. 沙箱应作为后台共享执行服务边界，而不是第四个面向用户的应用。隐藏入口不等于隐藏缺失能力，也不等于取消工具审批、冻结计划、scope、lease、预算、收据或平台限制。

本报告是源码审计和迁移设计，不是运行验收：读取了导航、主要组件/IPC、数据库及初始化、代表性执行/安全入口、现有测试入口；没有运行目标网络扫描、安装工具、读取用户数据库、构建安装包或执行全量测试。文中的“当前”是上述基线的源码行为，不承诺用户机器已具备模型、凭据、容器或平台沙箱。同期实施结果应另列实际通过/失败/未运行的检查，不回填为本审计已通过。

仓库及祖先目录未发现适用的 `AGENTS.md`，也未发现仓库 `.agents/skills` 指令；已核对 `README.md` 与 `docs/architecture.md`。README 的部分历史描述与执行代码不完全一致，以下以实际入口为准。

## 2. 当前应用全景：能力、直接依赖和迁移归属

下表中的“无 LLM”指该操作不需要模型调用，不意味着无需外部 API、网络或授权。

| 用户可见区域 | 当前入口与主要直接依赖 | 数据/实际执行依赖 | 无 LLM 能力与当前门禁 | 目标归属 |
|---|---|---|---|---|
| 项目/工作空间、概览、归档/删除 | `App.vue`、`ProjectDialog.vue`、`features/workspaces/api.ts` → `workspace_projects.rs` | Project 聚合读取 Assets、Scanning、knowledge 等领域；SQLite | CRUD/概览可用；删除先统计全部关联数据，有数据应归档；归档项目不能启动资产/扫描任务 | Shared Project；聚合读接口不要成为跨域写 owner |
| 资产表、查询、过滤、排序、列配置、CSV 导出、人工判定、软删除/恢复、入口已读 | `AssetWorkspace.vue`、`features/assets/api.ts` → `assets.rs` | `assets` + `project_assets`、资产事件、导出文件；打开入口调用系统 opener | 可用；选择必须携带项目归属，跨项目总览不是隐式操作项目 | Assets |
| 采集种子、任务创建、完整流程、复测 | `QueryPanel.vue`、App 快速任务 → `jobs.rs`、`jobs/native_workers.rs` | Project、显式 config profile、FOFA、Rust 原生 refinement/content/probe、任务目录 | 不用模型；采集需 FOFA Key 和可用种子；复测需已有项目资产；探测需目标网络 | Assets Operations |
| 仪表盘、资产任务、变化、隔离区、日志、启动中断提醒 | `App.vue`、`useAssetLogPanel.ts` → `workspace_projects.rs`、`assets.rs`、`asset_logs.rs` | `runs`、`asset_events`、`logs`、内容规则与探测状态；提交事件提示 | 可用；中断“继续”语义不同：复测可恢复，其他管线创建新任务 | Assets；通用通知保留于 shell |
| 归属档案、组织目录、主体建议、复核分组、确认/撤销、授权/撤权 | `AssetOwnershipGate.vue`、`OwnershipProjectSetup.vue`、`OwnershipReviewGroups.vue`、`ownershipDirectoryApi.ts` → `asset_ownership.rs`、`ownership_directory/review/operations/authorization*.rs` | 项目资产、组织目录、归属 policy version、分析/操作持久记录 | 当前明确为 `rules_only` / `model_not_ready`；并非调用模型后才可用；版本 CAS、预览/确认、项目成员、批量操作和授权范围不可省略 | Assets.Ownership；可被其他应用只读查询 |
| 公开暴露面：概览、轮次、线索、来源、待归并、聚类、复核、事件、覆盖、设置、导出/复测 | `ExposureSurface.vue`、`features/assets/exposure/*` → `commands/exposure/*` | Assets ownership/grant、独立 exposure settings/schema、冻结计划与 work items、HTTP/public indexes、证据/版本/归并 | 不依赖 LLM；要求 active project、实际授权 origin、来源许可、请求/字节限制；不同来源有各自凭据与平台限制 | Assets.Exposure，保持自己的领域模块 |
| HackerOne 项目/Policy/Scope/排除项、收藏、变化、同步、向项目导入 Scope | `HackerOneBoard.vue`、`features/hackerone/api.ts` → `hackerone.rs` | 全局 HackerOne 缓存与 notes/events；同步用 profile 中 API identifier/token；导入到 `targets` | 缓存读/收藏不需 LLM 或在线 API；同步需官方 API 凭据。当前 App 错把凭据就绪用于整个入口可见性/导航阻断 | Assets 的外部来源/连接器入口，保留完整看板；不强行把全局节目缓存变成每项目副本 |
| Web 新调查、资产转扫描、草稿/确认、重扫/暂停/继续/取消/删除 | `AgentWorkbench.vue`、`AssetWorkspace.vue`、`SentinelBoard.vue` → `scan_lifecycle.rs`、`scan_control.rs`、`scan_startup.rs` | Project、targets、scan/attempt/context、技能快照、模型、浏览器/HTTP工具、身份、runtime | 草稿可无模型；确认/执行需 `model_runtime_env`；目标和熔断过滤、旧 attempt 静默退出确认、审批和预算保持后端 authority | Scanning.Web |
| Code 源码审计 | Workbench → `code_analysis.rs`、`native_source_*`、`native_pipeline/*` | 可读源码目录、source snapshot/scope/runtime contract、模型、选定分析器、只读容器/受控进程 | 当前启动需模型。底层确定性工具可复用不代表已有独立无模型扫描产品；缺失工具须显示 coverage gap | Scanning.Code |
| Greybox 灰盒 | Workbench → 同一 `start_workbench_scan_impl`、source/Web 分支编排 | 至少一个 URL，源码可选；身份/认证上下文；source 与 Web 独立 dispatch/收据 | 当前启动需模型；单任务最多 200 URLs，身份与 source/Web 范围各自验证；一个分支失败不能伪造另一个分支结果 | Scanning.Greybox |
| CI/CD | `WorkbenchCicdSettings.vue` → `code_analysis.rs`、`native_source_ci_policy.rs`、`native_pipeline/ci.rs`、`source_ci.rs` | 源码目录、repository/branch/commit/build metadata、diff/full scope、阈值策略、分析证据 | 当前启动需模型；不能因为阈值计算是确定性的就绕过扫描/覆盖证据直接发布通过结论 | Scanning.CI |
| 调查对话、任务中心、成本、指令草案/确认、角色线程 | `AgentDialog.vue`、`useAgentDialog*`、`SentinelTaskCenter.vue` → `agent_dialog_*`、`agent_directive_*`、runtime status/history | scan/attempt/run/mailbox、能力租约、发送/确认收据、恢复记录 | 历史读取无模型；新指令不是通用聊天发送，必须绑定当前接收者、attempt、fencing token、确认及有效能力 | 留在 Scanning；Chat 可用显式引用打开它 |
| 调查结果、API/端点/指纹/JS、漏洞、源码结果、图谱/假设、人工验证/Repeater、熔断与恢复 | `SentinelBoard.vue`、`features/sentinel/results/*`、`components/results/*`、graph/repeater/fuse/recovery/reconciliation | Findings、checks、opportunities、appsec/investigation 数据、执行证据、目标请求 broker | 已保存结果/人工管理无模型；请求重放是目标网络操作；恢复有专门预算/身份/原收据规则，不能当“Chat tool”直接绕过 | Scanning.Results/Review/Recovery |
| Trace Hub、进程/SDK/runner 日志、历史导入预览 | `AgentTraceHub.vue`、`useTaskTrace`、`useNative*Log`、`HistoricalJsonImport.vue` | run/attempt 与明确历史导入源、脱敏日志和 artifact ledger | 读取可无模型；历史材料不授予当前执行能力；不能把模型 prompt audit 当最终完整请求证据 | Scanning.Audit，Shared artifact service 提供材料接口 |
| 技能目录、选择、编辑、导入/导出、知识/学习候选/规则包 | `WorkbenchSkillsCatalog.vue`、`useWorkbenchSkills.ts` → `scan_lifecycle.rs`、`knowledge_learning*.rs`、`rule_packs.rs` | 全局 `agent_skills`，project/scan provenance 的 knowledge，rule pack 本地文件/远端仓库 | 技能 CRUD/普通导入导出、确定性知识转换/聚合可无模型；生成候选、知识辅助 refine、source ingest 分析调用模型；rule sync 需网络/工具 | Shared Skills & Knowledge catalog；按应用过滤使用，不删除原功能 |
| 配置中心：多方案、模型、FOFA、HackerOne、代理、采集、探测、内容规则、Native 分析器、运行策略 | `ConfigDialog.vue`、`workspace_config_profiles.rs`、`db_settings.rs`、`runtime_environment.rs` | 全局 config profiles/settings；资产任务显式选 profile，扫描使用 default-first profile | 读取/编辑可无模型；模型测试确实访问 provider；保留当前完整字段及旧 inert fields，不能使用缩减表单覆盖现有 blob | Shared Configuration |
| 本机 Worker、远程节点、健康/环境检查、远程任务控制、项目增量同步 | `WorkerSettingsPanel.vue`、`features/runtime/api.ts` → `worker.rs`、snapshot import/export | `app_settings`、`worker_nodes`、Tailscale IP、Bearer token、namespace mappings、native project bundle | 节点配置/健康/历史同步不需 LLM；远端恢复扫描仍经过扫描 runtime；当前 API 不是通用 Chat 执行器 | Shared Worker service；按消费者暴露能力 |
| 运行环境、依赖准备、安装日志、工具批准/撤销、准备状态恢复 | App settings/runtime、`ToolApprovalPanel.vue` → `environment.rs`、`tool_administrator.rs`、`tool_supply/*` | 平台、管理员批准包、固定 runtime/digest、环境准备 OS/DB lease、日志 | 检查与管理不需 LLM；安装和 active scans 互斥；已批准 inventory 不等于 runtime 支持，更不等于 worker 获得执行权 | Shared Runtime/Execution service；管理入口仍在设置 |
| 主题、语言、图标、托盘、发布说明、全局通知 | App、`AppSettingsDialog.vue`、`ReleaseNotesDialog.vue`、`i18n.ts`、`lib.rs` | localStorage、app settings、应用生命周期；tray 跳转资产/任务 | 可用；切换应用不可触发环境安装、模型请求或重设用户配置 | Shared shell |

### 2.1 当前前端边界

- `src/api.ts:1–17` 是 stable facade，聚合 workspace、asset、ownership、HackerOne、Sentinel、runtime API；feature 文件存在，但这不是依赖隔离保证。
- `App.vue` 当前同时持有 module/view、selectedProject、资产任务/事件、扫描日志、HackerOne events、配置/安装状态和 worker 自动同步。`ModuleKey` 仍为 `hackerone | asset | sentinel`。
- `SentinelBoard.vue` 是大体量扫描组合根，承担创建、结果、对话、任务、历史与恢复。不要为了出现“Chat”导航，把它整块移动/复制成 Chat 应用。
- `ExposureSurface.vue` 已使用独立 exposure API/composables，但资产 picker 和部分证据读取仍经过 facade。可以渐进收窄依赖，无需重写整个暴露面。
- 当前 `selectedProjectId?: number` 的缺省值在多个列表里表示“所有项目”。新 mutation contract 应要求明确 ProjectId；旧聚合读取的缺省语义应保留并明确标为只读总览。

## 3. 数据 owner 与保留矩阵

数据库仍可保留一个 SQLite 文件；“模块拥有数据”是写入口和不变量的边界，不要求立即分库。`commands.rs` 大量使用 `include!`，被 include 的文件共享命名空间，不是独立 Rust module。先抽窄接口，再迁移代码目录。

| 数据族 | 当前 owner/关系 | 必须保留 | 建议迁移方式 |
|---|---|---|---|
| `projects` | 共享根实体 | 原 ID、归档状态、时间、名称；项目间隔离 | Project service 查询/生命周期，不改 ID，不自动复制项目 |
| `config_profiles`, `app_settings`, `worker_nodes` | 全局共享基础设施 | 现有 profile ID/default、所有配置、当前和 inert 字段、节点身份 | 薄 service/view；新 app 读取同一配置，禁止创建“Chat 默认配置”覆盖原默认 |
| `targets`, `runs`, `asset_events`, `logs`, `saved_views` | Assets；`runs` 是整数 ID 的资产任务 | 原项目、profile、快照/输出目录、事件及恢复语义 | 保持表与 owner；不与 `agent_runs` 合并 |
| `assets`, `project_assets`, `content_rules` | 全局 endpoint record + 项目关系；内容规则当前全局 | 一资产跨项目复用；判定/删除属于 link；全局端点更新的现有影响 | 引入 Asset repository/operation port，沿用联结模型 |
| `asset_ownership_*`, `project_org_bindings`, `ownership_*` | 归属领域；组织目录本身有共享实体 | 分开归属与授权、版本/操作记录、目录复用、回滚语义 | 归属 service 作为 Assets 子域；下游只消费授权事实，不写结论表 |
| `exposure_*` | 项目/轮次/发现/证据/版本/设置 owner | frozen plan、work items、scope receipts、pending discoveries、clusters、incidents、review、remediation、coverage | 保留 `commands/exposure` 独立 schema 初始化及其事务协议；不并入 Chat 表 |
| `hackerone_*` | 全局外部源缓存；导入 seeds 时才选择 Project | programs/scopes/exclusions/bookmarks/events | 连接器保持全局缓存；对项目使用 typed import operation |
| `sentinel_scans/scan_attempts/targets/contexts/checkpoints/findings/opportunities/validations/fuse_zone` | Scanning；scan 可留历史 project_name，现 schema `project_id` 可空 | scan/attempt 状态、历史、task path、mode ceiling、targets、熔断 | 增量扫描 facade；不要批量改 NULL/FK 或迁移所有 SQL |
| `browser_auth_sessions` | Project + draft/scan-bound 身份 | scope binding、有效性/生命周期、私有存储 | Shared identity service 可以被扫描消费；不因项目相同就给 Chat 自动读取认证材料 |
| `appsec_*`, `investigation_*`, `knowledge_facts/strategies/outcomes` | 扫描结果、验证、证据关系和学习事实 | 来源与目标关联、人工结论、mutation approval | Scanning 专有写入口；Chat 查询只拿最小可引用摘要 |
| `agent_runs/messages/events/snapshots`, assignments、leases、budget、directives、reviews、evidence、protection | 原运行和 mailbox owner，run 必须引用 scan | 原始收据/预算/租约/fencing、派发和终态 writer、删除封存、恢复规则 | 完整留在 runtime；禁止“通用 Chat 化”改变其 FK/字段语义 |
| `source_*`, `native_scan_*`, `native_branch_*`, `native_web_*`, `analyzer_*` | Scanning 的 source/Web attempt 执行与证据 | 原冻结输入、source contract、分支持有者、容器清理收据、CI coverage | 由受控执行 service 消费，不暴露裸 SQL/进程执行给应用 |
| `agent_skills` | 全局 prompt instruction catalog，无 Project FK | ID、builtin/enabled、内容；内置技能禁止覆盖/删除 | 兼容现有记录，添加类型视图/适用 app 元数据应有默认映射；不将文本解释为任意命令 |
| `agent_knowledge_entries`, `agent_learning_candidates`, `security_rule_packs`, role-config drafts | knowledge 的 Project/scan provenance、全局规则/草案 | 来源 hash、候选 review/apply、扫描类型、规则版本和同步变更 | Catalog 查询与明确动作分开；保留每类当前模型/工具需求 |
| `artifact_objects`, `import_*` 及 worker namespace mappings | 材料导入/归档与来源隔离 | 本地/远程 ID 映射、bundle 来源、版本、诊断、历史只读语义 | Shared artifacts service，不把导入结果升级为当前 run authority |
| 新 `chat_sessions/messages`（建议） | 独立 Project-owned Chat | 不能使用 `agent_messages`；不要求 scan_id；不发送伪造 scan/run | 只做新增 schema 与新 IPC；项目归属读写校验、分页、幂等、取消/并发状态自有 owner |

Schema 证据：`db_schema.rs` 的 `projects`（5）、`config_profiles`（14）、`worker_nodes`（29）、`sentinel_scans`（173）、`targets/runs/assets/project_assets`（663/674/693/721）、ownership（735–998）、exposure base（999–1049）、`agent_runs`（1092）、`agent_messages`（1199）、source/native（2009–2339）；`db_neutral.rs:5–45`；`db_initialize_columns.rs` 的 rule-pack schema；`commands/exposure/schema.sql`；`db/worker_namespaces.rs`。行号为审计基线定位，后续改动应以符号名核对。

### 3.1 新 Chat 数据契约的最低要求

- Session 的 Project 必须存在；写入时明确处理归档状态，不能仅相信前端选中项目。
- 列表/读取/发送均校验 `projectId + sessionId`，同项目 ID 不等于允许读取其他 session 的消息。跨 Project session ID 必须失败并零写入。
- Message 有稳定 ID、role、content、时间/排序；模型 turn 有独立状态与 request identity。重复发送、关闭窗口、取消、切换项目、晚到回复不能产生重复消息或污染新视图。
- 默认 Chat model request 只包含该 session 和用户显式选入的内容，空工具列表；不得静默携带所有项目资产、扫描历史、浏览器身份、凭据或隐藏 prompt 审计全文。
- 运行时 mailbox 历史留在 Scanning；Chat 通过显式 reference/link 指向 scan/asset，而非搬迁原消息。任何 cross-app operation 走原应用命令和授权逻辑。
- 若首批仅实现本地会话/消息管理，产品必须写明未实现模型回复；不能把保存文本称为完整 AI Chat。若首批接入模型，必须同时完成 below 的发送/取消/错误/使用量测试，不能以导航完工代替。

## 4. 真正的依赖与门禁

### 4.1 能力就绪应按动作，而非整页或整个应用

建议建立只读 capability 描述：`operationId`、`application`、`projectRequired`、`requiresModel`、`requiresNetwork`、`provider/tool/platform`、`availability`、稳定 `reasonCode`。它是 UI 解释，不是授权凭证；执行端每次仍校验真实状态。

必须区分：

- `available`：已核实必要本地条件；不保证未来网络请求成功
- `unconfigured`：缺模型、FOFA/HackerOne/API reference 等明确配置
- `unsupported`：当前平台/adapter 没有安全实现
- `blocked`：Project archived、scope/approval/lease/budget/fuse 状态不允许
- `unknown`：未检查或读取失败；不得显示为可用，也不得伪称无功能

具体源码门禁：

| 操作 | 入口证据 | 当前行为/迁移约束 |
|---|---|---|
| Assets collect / reprobe | `jobs.rs::start_job`, `execute_job`, `execute_reprobe_job`; `jobs/native_fofa.rs::collect_fofa` | active Project + profile；collect 空 key 明确失败；reprobe 从项目已有资产读取；都不调用 `model_runtime_env` |
| Ownership | `ownership_directory.rs` 和 `ownership_review.rs` 中 readiness；`asset_ownership.rs` 的 `OWNERSHIP_PROFILE_VERSION_CONFLICT` / `OWNERSHIP_DECISION_PRECONDITION`; `ownership_authorization.rs` | 当前规则能力可用；origin/host/subdomain 规则范围有明确限制，不能把“确认属于某组织”等同于所有测试已授权 |
| Exposure start/resume | `exposure.rs::start_exposure_scan/resume_exposure_scan`；`scope.rs::ExposureScope::load`；`orchestrator.rs`、`recovery.rs` | active Project、active project_asset、exposure eligibility、origin/方法范围；start 冻结 settings，resume 用原计划并重新验 scope，不读取今日默认值冒充原计划 |
| Exposure GitHub | `adapters/github.rs::CredentialLookup::from_ref/read_keychain` | 仓库元数据可匿名；代码搜索/受控 blob 需凭据引用。实际代码已实现 macOS Keychain lookup，非 macOS 当前返回无值；文件顶部“integration_pending”旧注释不能当现状结论 |
| HackerOne | `App.vue::hackerOneEnabled/switchModule/refresh`; `hackerone.rs::list_hackerone_programs/get_hackerone_detail/sync_hackerone` | UI 的整个入口被凭据阻断；后端 cached reads 无此需求。可分离“查看缓存”和“同步需配置”，但不得删除同步凭据验证 |
| Web draft vs execute | `scan_lifecycle.rs::create_sentinel_asset_scan_in/create_sentinel_url_scan_with_mode_in`; `scan_control.rs::start_web_scan` | 草稿事务先验 Project、目标、mode、身份、tool selection；启动显式 model config、quiescence、runtime/worker、执行计划与工具绑定 |
| Code/Greybox/CI | `code_analysis.rs::start_workbench_scan_impl` | 类型、目录、URL 数、预算、CI阈值、active Project、skills、model、source/Web contract；不得只删 model check 而留下依赖模型的 coordinator/terminal path |
| Shared model | `runtime_environment.rs::model_runtime_env`; `agent_model_client.rs`; `agent_runtime/model/*` | explicit modelProfiles 列表为 authority；空列表不回退旧缓存；local 使用明确 endpoint 和 local key/default local，不读取 cloud key/base 环境变量。Cloud 可用当前配置或现有环境 fallback；保持当前行为 |
| Chat model adapter（建议） | 可复用 storage-agnostic `ModelRequest/ModelGateway/CancelToken`；`agent_model_client.rs::agent_scan_cancel_token` 是 scan 专用 | 复用 gateway/admission，不复用 scan cancellation/query。Chat 需要自己的 turn owner、取消和用量记录；无工具 Chat 不应进入 target executor |
| Knowledge actions | `knowledge_learning_candidates.rs::generate_agent_learning_candidate/apply_agent_learning_candidate`; `knowledge_learning_skills.rs::refine_agent_instruction_with_knowledge/ingest_agent_knowledge_source`; `knowledge_learning_traces.rs` | 候选生成/refine/source 分析需模型；apply 已接受候选及若干 trace 转换为确定性操作。不要一律标为“技能全部需要模型”或“知识全部离线” |
| Tools / sandbox | `tool_supply/execution_readiness.rs::require_current_on/require_actual_supported_parser`; `native_pipeline/analyzer.rs/container.rs`; `agent_runtime/controlled_executor.rs` | 原 approval、frozen plan、manifest/digest/platform/resource policy 必须匹配；host fixed parser 可明确 unsupported；容器需要实际安全条件。禁止因“已安装/已批准”直接当可执行 |
| Worker | `worker.rs::handle_worker_request/restart_worker_service/sync_worker_node` 与 namespace 模块 | 本机监听检测到的 Tailscale IP，Bearer 认证；远程列表/环境/控制/导入不是通用执行入口；同步应保持序列化，避免大 bundle 争抢 SQLite |

### 4.2 不把基础能力从业务授权里抽空

后台服务建议拆为只读 `RuntimeCapabilities`、`ModelGateway`、`WorkerDirectory`、`ArtifactStore`、`ControlledExecution` 接口。它们可以被不同 app 使用，但调用必须包含真实 owner/scope；不能造一个接收任意 shell、URL、路径和“approved=true”的统一万能执行 API。

现有沙箱/执行保护分散在 tool_supply、native_pipeline 和 agent_runtime 中，暂时不要宣称已经统一成单一 SandboxManager。先加 adapter/facade，保持原路径，逐步证明每个消费者使用同一安全接口。管理与诊断继续放在设置中，错误要展示明确“缺工具/平台不支持/未批准/无法核实”，不能藏成静默失败。

## 5. 目标模块边界与跨应用操作

### 5.1 Shared Project + 三应用

- Shell 只管理当前 Project、应用切换、全局导航/通知、主题和共享设置入口。
- Chat 拥有会话、消息、模型 turn 和显式上下文引用；可以独立打开/使用，不要求先建扫描任务。
- Assets 拥有采集、整理、ownership、exposure、外部来源。没有模型配置时其确定性功能仍应可用。
- Scanning 拥有 Web/Code/Greybox/CI、运行对话、结果、审计、恢复；没有资产数据也可以直接建立 URL/source 任务。
- Shared 配置/技能/模型/工具/身份/worker 不拥有业务终态，不替某 app 批量改它的数据。

采用单向依赖：`application → domain port → infrastructure`。允许 shell 组合应用，避免 application component 互相 import。迁移期 `src/api.ts` facade 可继续兼容老组件，但新代码应直接依赖所属 app API 或明确 shared API。

### 5.2 Skills 与 executable operations 必须分型

现有 `agent_skills` 的核心是 `instructions` 文本。建议保留这一事实并区分：

1. Instruction skill：提示/方法/审核知识，带适用 app/scan type 和 version；选择它不产生执行权限
2. Knowledge artifact：来源、证据、摘要、候选审核状态；可以被引用，不直接执行
3. Executable operation：固定 `operationId`、输入输出 schema、owner、requirements、side-effect class、审批/预算/scope contract、执行 adapter；不得仅凭技能正文生成并执行任意命令

第一批可以只引入 typed descriptor/registry 和现有技能的兼容映射，不必立刻修改所有技能 SQL。后续若增加类型字段，做 additive、版本化迁移，并保持旧记录为 instruction，不把旧 JSON 中未知字段当新权限。

跨应用的最小显式操作示例：

- `assets.createScanDraft(projectId, assetIds, mode)`：回到现有资产→扫描草稿入口；验证 project membership / fuse；用户后续确认运行
- `hackerone.importScopes(projectId, handle)`：调用现有 Scope→targets 操作；不假设外部平台“in scope”就取得额外目标网络授权
- `scanning.readSummary(projectId, scanId)`：返回可引用结果与 provenance；不返回认证材料
- `chat.openReference(projectId, kind, id)`：导航/只读引用，不开始扫描、修改资产或写 runtime mailbox

## 6. 推荐的第一批实施范围（可单独审查与回退）

第一批目标是建立真实边界和独立 Chat 最小闭环，保留所有旧入口和执行路径；不是一次重写扫描后端。

### Batch 1A：共享导航/上下文与 capability 合同

1. 增加明确 `ApplicationKey = chat | assets | scanning` 和 Project context；兼容原 localStorage 的 asset/sentinel/hackerone/view 值。HackerOne 旧入口映射到 Assets 来源页，不丢缓存/书签/同步按钮。
2. 在 shell 上挂三个应用入口，旧 Assets/Scanning 内容原样包裹，原调查对话仍归 Scanning。各 app 的内部 tab、选中 scan/session、草稿状态单独保存。
3. capability registry 按动作表明模型、API、工具和平台要求；无模型不得阻断 Assets、历史读取、设置/技能 CRUD。HackerOne 缓存可查看，同步仍显示配置要求。
4. 不改执行状态、扫描 schema、配置默认值或原保密字段；不执行自动数据修复。

### Batch 1B：独立 Chat 的小范围垂直实现

1. 新增 Project-scoped session/message schema 和 dedicated commands/API/component；只做新表和新注册，不松动 `agent_runs` / `agent_messages` 的 scan FK。
2. 会话创建/列出/选中、消息历史、无模型时可明确保存/保留输入；分页和错误状态要真实。
3. 若本批包含模型回复，优先“纯文本、无工具”的一次请求/响应，复用共享模型配置和 gateway；完成本 app 独立请求 ownership、重复请求 fencing、取消、project/session switch、失败后重试和 usage 记录。不得通过制造 dummy scan 取得模型能力。
4. 不在第一批提供 Chat 自动启动扫描/任意 shell/浏览器目标访问，不把已有运行指令确认流程改为普通发送。以后接 operation registry 时，再逐个接上原后端授权门禁。
5. 新 Chat 项目关联应纳入 project impact/delete guard（现动态扫描已提供基础保护，但仍需明确 UI 项目统计与测试）。非空项目继续归档；先不提供绕过归档策略的 Chat cascade-delete 入口。

### 第一批明确不做

- 不批量改所有 `project_id`、表 FK 或 SQL；不分库；不复制 assets
- 不将 `agent_messages` 重命名/复用为通用 Chat
- 不删模型门禁宣称现有扫描已是 deterministic-only；这需要单独的产品执行模式与闭环测试
- 不拆动 native financial/terminal/lease/receipt owner，不扩大 pub visibility 代替设计接口
- 不删旧配置、不把当前安全阻断变成“降级执行”、不自动安装依赖/工具

### 第一批验收与回退标准

可验收结果：项目切换下三个入口均可独立打开；已有 Assets/Scanning 功能可到达；Chat 有独立持久化且不创建任何 scan/run/mailbox 行；当前配置读写 round-trip 保留；指定动作的 readiness 显示准确。

安全回退：恢复旧 shell/入口即可；新增 Chat 表保留不破坏现有数据，不做破坏性 down migration。若发送产生未知结果，保留 turn 状态为待核实而不是自动再次发送。首批未完成模型闭环则在 UI 和交付说明中标明，不作为“完整 Chat 已完成”。

## 7. 后续迁移矩阵

| 批次 | 工作 | 完成证据 | 主要风险 |
|---|---|---|---|
| 2 | 从 shell 抽 Assets app container，将配置/Project/Worker service 与 app 业务 state 分离；收窄 feature API | 无模型 Assets 操作集 + 原导出/变更/复测/隔离/ownership/exposure 回归；仅所属 app 激活刷新 | 全局统计仍需聚合；切项目晚到数据；遗漏日志/托盘/中断恢复 |
| 3 | Scanning 拆任务/结果/运行对话/创建/审计的模块界面，业务 runtime 不变 | Web/Code/Greybox/CI、Single/Multi、身份、follow-up、暂停/恢复/删除的既有行为测试；IPC shapes 不变 | 组件拆分误丢状态/props，既有源码 pin 测试需逐项审查而非统一删除 |
| 4 | Typed skill/knowledge catalog + operation registry；Chat 显式引用 Assets/Scanning | 指令无执行权；跨 app 每个 operation 输入、owner、审批、返回 provenance 和 negative tests | Prompt 注入被误作权限，跨 Project 索引/详情泄露，权限从文本“升级” |
| 5 | 后台执行服务统一 facade；新增真正无模型 deterministic scan 模式（若仍需要） | 模式明确、无模型请求、工具缺失/平台 unsupported/取消/结果持久化/CI coverage 皆可观察；原 owner 和安全负例不回归 | 把可运行工具误当完整 pipeline；模型相关退出/费用语义被误删；平台不对称 |
| 6 | Rust include 边界转真 module，按领域迁移模型/存储；审查 config typed views | 编译可见性最小化、迁移幂等/旧库/reentry、无循环依赖、全量相关检查及目标平台包验证 | 大规模 SQL 改写、初始化顺序差异、旧表/配置被误清理 |

## 8. 必需的测试与风险清单

### 新增行为测试

1. 两个 Project、同一个 asset 全局 record、两条 project_assets link：人工判定/软删除只影响选定项目；全局 endpoint 字段更新仍遵循原语义
2. Chat 无 scan：创建会话/消息后 `sentinel_scans`、`agent_runs`、`agent_messages` 数量不变；重启后能加载
3. Chat project/session mismatch、归档/不存在 Project、空/超长输入、无模型、错误 profile、请求取消和重复请求：失败零越权/零意外写入
4. A Project 请求在途→切 B→回复到达；session 切换、Close/Back、重复点击、较新编辑与旧 receipt：历史归原 session，当前屏幕不被旧结果覆盖
5. 共享配置：FOFA/HackerOne/proxy/model profiles/local key/native analyzer/worker 及原 inert 字段 round-trip，explicit empty modelProfiles 不复活旧模型；local 不发送 cloud credentials
6. 无模型功能矩阵：Assets 查询/导入/判断/导出、规则归属、历史结果、skills CRUD 可用；FOFA/HackerOne 缺凭据仅阻断对应在线动作；Web draft 可创建，实际启动明确缺模型
7. 技能正文/外部知识不能授予 shell、网络或追加能力；Chat 无工具请求中不会暗带浏览器身份/所有项目数据
8. 原项目非空删除保护覆盖新增 Chat 数据；归档后历史可读，新的业务执行仍被拒绝
9. 旧导航 localStorage 值、HackerOne 入口、托盘跳转、原各页和配置 tabs 都能到达；模块切换不自动启动后台任务
10. Event/poll 消费只刷新当前 app 对应范围；保留资产/进程/安装日志各自的 committed cursor 与 redaction 协议，不按 UI 收到消息就当写入完成

### 现有检查入口（待实际执行，不是本报告的通过结果）

- 基础：`npm run build`；`cargo check --manifest-path src-tauri/Cargo.toml`
- UI：`npm run test:ui`、`test:agent-workbench`、`test:agent-dialog`、`test:project-deletion`、`test:scan-deletion`、`test:execution-history`；新增 app/Chat 行为测试应加入明确脚本
- Assets/Exposure：`test:asset-exposure`、`test:exposure`，Rust `ownership_*`、`commands::exposure::*`、`asset_logs` / DB tests
- 配置/项目：`db_settings_tests`、`db_settings_startup_tests`、`commands/tests_settings_preservation.rs`、`tests_model_config.rs`、`tests_project_deletion.rs`
- Scanning/runtime：Web draft/startup/mode-binding/toolchain、workbench admission/auth/retry/startup、source scope/runtime/CI/coverage、dialog/action fencing、scan quiescence/deletion/retired-import、预算/lease/收据/工具供给相关 Rust tests
- Gateway：`agent_runtime/model/admission_tests.rs` 及 gateway/transport 的 fake-provider tests；新 Chat 应用测试不能仅凭编译和文本 source pin 宣称发送成功
- 平台：macOS/Windows/Linux 的现有 cfg 与实际 sandbox/工具能力分别报告；某平台 `0 tests` 不代表通过；安装包/sourceBinding/运行验收遵循 README 的独立流程

重点风险及处理：

- **业务同名误合并**：`runs` 是资产 job；`agent_runs` 是扫描执行 run；`agent_messages` 是执行 mailbox。保持各自 owner
- **过度去模型化**：把可离线查询和底层确定性工具与完整 agent pipeline 区分；能力标记不能绕过后端
- **配置损失**：不能用只含模型的对象调用全量 profile save；`settings_for_update` 只补回被 normalize 排除的 inert 字段，调用者仍须传完整当前设置
- **跨项目旧响应**：所有异步读取/操作的视图身份包括 Project 和实体 ID；不能只比较“最新一次全局刷新”
- **隐性模型费用/隐私**：打开页面不发模型请求；send 时明确 provider/profile，最小上下文；错误/不完整用量不可当“未收费”
- **能力声明冒充执行证明**：metadata、平台检查、签名、冻结计划均不能代替实际执行 gate 和 receipts
- **审计材料误变实时权限**：远端导入、历史日志、knowledge、Chat 引用均只携带事实，不授予当前执行能力
- **迁移范围失控**：一次仅收窄一个 owner/接口并保留兼容 facade；未经完整行为证据不做 blanket SQL/状态/终态重构

## 9. 关键源文件索引

- 产品组成：`src/App.vue`、`src/api.ts`、`src/types.ts`、`src/components/*`、`src/features/*`
- IPC 注册/组合根：`src-tauri/src/lib.rs::invoke_handler`、`src-tauri/src/commands.rs`
- Project/config：`commands/workspace_projects.rs`、`workspace_config_profiles.rs`、`db_settings.rs`
- Assets/native jobs：`commands/assets.rs`、`jobs.rs`、`jobs/native_workers.rs`、`native_fofa.rs`、`native_probe.rs`、`native_refinement.rs`
- Ownership/Exposure：`commands/asset_ownership.rs`、`ownership_*.rs`、`commands/exposure.rs`、`commands/exposure/{scope,settings,orchestrator,recovery,state}.rs`、`schema.sql`
- 扫描创建/执行：`commands/{scan_lifecycle,scan_control,scan_startup,code_analysis,workbench_startup,workbench_auth,workbench_retry}.rs`
- 调查对话与 mailbox：`commands/agent_dialog_*`、`agent_directive_*`、`src/features/sentinel/composables/useAgentDialogDirectives.ts`、`db_schema.rs`
- 模型：`commands/runtime_environment.rs::model_runtime_env`、`agent_model_client.rs`、`agent_runtime/model/{gateway,admission,profile,transport,openai_compatible}.rs`
- 技能/知识：`db_neutral.rs`、`commands/scan_lifecycle.rs::list/save/delete_agent_instruction`、`commands/knowledge_learning*.rs`、`commands/rule_packs.rs`
- 隐藏执行服务与供应链：`agent_runtime/controlled_executor.rs`、`tool_supply/*`、`native_pipeline/{analyzer,container,process}.rs`、`commands/tool_administrator.rs`
- Worker/import：`worker.rs`、`db/worker_namespaces.rs`、`artifact_import/*`、`snapshot_export.rs`

本审计的代码改动范围仅为此文档；任何同期实现请用各自的 diff、测试结果与未完成项说明，不将本报告的建议当作已经落地。

## 10. 同期第一批实施与验证登记（不与基线审计混同）

以下是 2026-10-13 06:27 UTC 的最终冻结源码验证状态：新增 Conversation Rust tests 22/22 通过，前端 focused tests 43/43 通过，冻结源码 UI 972/972、Assets 72/72 通过，production build 退出 0；input_parser 筛选测试终态为 42 passed / 1 failed。所有本批已启动的检查均已结束。下方保留 5 个原基线同样复现的失败、Strict Clippy 阻断和未验证范围，不据此宣称所有设计目标或全量测试完成。

| 项目 | 本批实际范围 | 当前证据/状态 | 尚未完成的目标 |
|---|---|---|---|
| Shell 边界 | `src/shell/moduleCatalog.ts`、`navigation.ts`、`WorkspaceNavigation.vue`；新增 Chat 入口，旧 asset/sentinel ID 保持兼容 | 实现完成；focused 6 route + 9 actual-App tests 通过，完整冻结 UI suite 972/972 通过 | 实际保留 HackerOne 第四个兼容入口，且仍有 `requiresHackerOne:true`；目标三应用收敛与缓存无凭据访问不是本批已完成项 |
| 独立 Chat 数据 | `conversations/schema.sql` 的 `conversations`、`conversation_messages`、`conversation_links`、`conversation_replies` | 实现完成；Project FK，独立 persistence/scope/幂等/零 scan-run-mailbox 写入包含于 22 个已通过 Conversation tests | 不是对旧调查对话搬迁；多会话协同 agent orchestration 尚未实现 |
| Chat UI/IPC | `features/conversations/*`，独立 create/read/update/send/cancel/link/delete 合同 | 实现完成；28 conversation frontend tests + 15 navigation/actual-App tests 通过；未做真实桌面浏览器交互验收 | 不等于完整聊天 agent 工具执行、全局上下文检索或多 agent 编排 |
| Chat 文本回复 | `conversations/reply.rs` 复用既有 resolver/gateway，显式发送触发受限单次零工具请求；独立锁、幂等/CAS、持久 dispatch fence 和取消状态 | 实现与边界核对完成；22 个新增 Rust tests 通过，包含受控 loopback provider 验证；未做 live provider 验收 | 仅独立文字回复服务；没有通用 agent tool/filesystem/scan 执行，linked evidence 不自动发送模型；不替代各 app 的执行 owner |
| 隐藏执行机制 | `execution_service/{host_process,process_output,offline_parser}.rs`，native_pipeline 保留兼容路径 | 实现完成，原 scope/budget/lease/receipt owner 保留；execution_service 22 passed / 1 failed（原基线同失败）；其他相关过滤项见下表 | 未完成全服务 owner 分离、通用调度/事件总线、Chat/Assets execution adapter、跨平台 sandbox 支持 |
| Skills / capabilities | 本审计给出分型和逐动作能力矩阵 | 设计建议 | Typed skill registry/policy 迁移仍待实施；不宣称本批把技能变成可执行操作 |
| 独立部署 | 仍为同一 Vue/Tauri/Rust 应用内边界 | 本批明确范围 | 没有独立 binary、独立进程应用、独立 headless CLI 或完全独立 Rust crate |

### 10.1 本批 Chat 的确切能力边界

- 同一既有 Project 可以拥有多个持久 conversation。用户/助手消息属于 `conversation_messages`，不会写入扫描 `agent_messages`；不为了发送普通消息创建 dummy scan/run
- 搜索、历史读取、创建和 `requestReply=false` 的保存消息不需要模型。仅显式 `send(requestReply=true)` 可以触发一次模型请求；无模型时保存真实用户文字并返回不可回复状态，不生成模拟助手回复
- Availability 只表示配置和本地策略符合发送条件，**不表示 provider 可达、凭据有效或模型已完成测试**。既有 resolver 仍为 profile/凭据的唯一配置来源，错误/DTO 不携带 secret
- `ModelRequest` 的工具列表为空。当前没有通用 agent 工具、文件系统、浏览器或扫描执行；scan/asset links 只作同项目导航引用，既不授予证据/工具 authority，也不把 linked evidence 自动加入模型上下文。目标删除后引用显示不可用，不重新造一个目标
- 模型 URL 要求 HTTPS；只有 local profile 可用 literal loopback IP 的 HTTP（如 `127.0.0.1` / `[::1]`），拒绝 URL userinfo/password/fragment，保持正常 TLS 校验
- Chat 显式启用新增的严格 transport 选项：拒绝 redirects、最多 1 MiB wire response；既有 transport 消费者默认 `reject_redirects=false`、`max_response_bytes=None`，不改变旧运行行为
- 用户文字上限 16 KiB；取最近最多 64 条完整历史消息，在 64 KiB context ceiling 内再受模型自身 budget 约束并保留封装余量；响应文字上限 64 KiB；输出 token 上限 2048；单次回复有 90 秒单调时钟截止和取消检查（包含本地模型路径）
- 仅有 **process-local 两个 reply workers** 的准入限制；还需经过既有 gateway admission。这不是跨进程并发限制、共享财务账本或跨应用费用预算
- 稳定 per-conversation OS lock 在发布 queued 前取得，持续到 transport 与最终持久化完成；SQLite dispatch connection 使用 `synchronous=FULL`。Operation ID 支持幂等收据，revision/CAS 防止过期写入；同会话 active reply 阻止修改/link/archive/delete
- 取消先记 `cancel_requested`，只有 transport owner 结束后才完成状态。孤儿恢复必须取得同一 OS lock；不会做启动时 blanket reset，不自动 resume/retry。网络结果或费用未知时明确保留 unknown，不能用“重试”把不确定调用悄悄再次发出

以上由 `conversations/{mod,store,reply,model}.rs`、`schema.sql` 与对应测试源码交叉核对；实际运行结果按下面的独立日志登记，不把假 provider 测试等同于 live provider 或产品端到端验收。

### 10.2 改动前基线

基线验证由协调者在改动前执行；本审计已核对保留日志的结果尾部：

- `npm run build`：基线通过
- `npm run test:ui`：934 passed，0 failed/cancelled/skipped
- 现有基线 Rust binary 的筛选项：`project_deletion` 7 passed；`asset_draft` 5 passed；`scan_quiescence` 11 passed
- Asset/exposure 套件：运行中断/不完整，不标为通过
- 这些基线结果不能替代新工作树编译、测试或目标平台验收；改动后的独立结果见下节

### 10.3 改动后已完成检查与剩余验证

以下已核对日志尾部。Rust 行为测试是按 filter 运行的子集，过滤项可能重叠；不能把行数简单相加后称为全量 Rust suite。

| 检查 | 实际结果 | 证据文件/边界 |
|---|---|---|
| Conversation Rust 新测试 | 22 passed，0 failed | `conversations-final.log`；持久化、项目边界、幂等/CAS、取消/恢复、零工具和受控 loopback transport |
| Frontend focused | 43 passed，0 failed | `frontend-focused.log`；28 conversation + 6 routes + 9 actual-App checks；不是 GUI 浏览器验收 |
| Production frontend build | passed，冻结复跑退出 0 | `frontend-build.log`、`frozen-build.log`；编译不代表安装/GUI运行验收 |
| `execution_service` | 22 passed，1 failed | `final-rust-execution_service.log`；`native_process_group_sweep_waits_out_a_zombie_only_group` 在本 Linux 环境失败 |
| `native_pipeline::tests` | 62 passed，0 failed | `final-rust-native_pipeline__tests.log` |
| `native_helper` | 15 passed，1 failed | `final-rust-native_helper.log`；`resolve_helper_node_prefers_modern_over_usr_local_v16` 的 Node fixture 失败 |
| `execution_readiness` | 4 passed，0 failed | `final-rust-execution_readiness.log` |
| `tool_supply` | 51 passed，2 failed | `final-rust-tool_supply.log`；两个 `approved_package_owned_writer_actual_*` 遇到 `unsupported_tool_package_exclusive_publish_platform` |
| `project_deletion` / `asset_draft` / `scan_quiescence` | 分别 7 / 5 / 11 passed，0 failed | `final-rust-project_deletion.log`、`final-rust-asset_draft.log`、`final-rust-scan_quiescence.log` |
| `auth_session::` / `agent_runtime::model` | 分别 13 / 31 passed，0 failed | `final-rust-auth_session__.log`、`final-rust-agent_runtime__model.log` |
| `input_parser` | 42 passed，1 failed（364.64s） | `final-rust-input_parser.log`；`input_parser_root_feedback_admits_only_input_parser_with_distinct_trigger_name` 同基线 line 218 assertion；先前 `homogeneous_parser` filter 匹配 0 项，不算测试通过 |
| Strict Clippy | blocked / failed | `clippy-final.log`；27 个现有 diagnostics（23 lib、24 lib-test 有重叠）；协调者已正规化行号，message + source path multiset 与改动前保留结果完全一致，0 added / 0 removed |
| 冻结源码 UI 聚合 | 972 passed，0 failed | `frozen-ui.log`；包含新增导航/Chat checks，不另加 focused 数量冒充独立覆盖 |
| 冻结源码 Assets/Exposure 聚合 | 72 passed，0 failed | `frozen-assets.log`；本次终态覆盖前述不完整基线运行，但不回写基线为已通过 |

五个 Rust 失败均已在 **改动前原二进制** 逐项复现：`compare-baseline-native_process_group_sweep_waits_out_a_zombie_only_group.log`、`compare-baseline-resolve_helper_node_prefers_modern_over_usr_local_v16.log`、`compare-baseline-approved_package_owned_writer_actual_.log`（两项）、`compare-baseline-input_parser_root_feedback.log`。最后一项是原有 admission fixture 的 line 218 assertion：实际 `[]`，预期 `[business-logic-output, authorization-output, external-surface-output]`，不是环境缺失原因，不能归为新重构失败。其他四项的具体限制已逐行列出。这支持“此次改动没有新增这五个失败”，不等于这些测试通过，也不证明相关能力在所有平台可用。未将失败改成 ignored、未放松安全门禁来取得绿色结果。五个失败跨互不相同的过滤项，但其他通过项可能重叠，不汇总成虚假的唯一全量测试数。

**未完成/未运行的验收**：全量 Rust suite、macOS/真实容器沙箱平台验收、真实桌面 UI/browser 交互、live model provider、真实目标扫描、安装包/安装后运行验收。首次独立 Chat 和执行机制抽取完成，不代表独立应用二进制、完整 Rust owner 封装、技能策略迁移或通用 tool-agent orchestration 已完成。

验证日志位于工作区的 `oviraptor-modularization-verification/` 目录；不将其原始日志、临时路径或源码树状态包装成用户机器的安装/发布验收。仓库未跟踪 `.github/workflows`，没有可据此声称通过的远端 GitHub Actions CI。本报告在上述终态后冻结；文档写入范围仅本文件，不扩大实现范围。

## 11. 第一批之后的有序待办与迁移验收

第一批明确保留原 Scanning workbench、调查对话和完整 settings；HackerOne 第四入口与当前凭据门禁也是兼容行为，不是本批修复结果。后续按以下顺序推进，每项独立审查，不因路由存在就宣布领域分离完成。

| 顺序 | 下一步交付 | 迁移验收/停止条件 |
|---|---|---|
| 1 | 补齐本批新代码集成回归和 UI 实际交互检查 | 新 Chat 建立/选中/发送/取消/重试/切项目闭环，旧扫描/资产/配置仍可达；未跑平台明确列出。验证失败先修复本批，不继续扩大迁移 |
| 2 | Shared Project context 与默认工作空间 onboarding | 明确“首次空库”“已有项目但未选中”“归档/已删除项目引用”“导入旧偏好”四种情况；初次用户可以理解并选择/创建工作空间，不静默把历史数据重新归属，也不把所有项目总览当 mutation scope |
| 3 | Assets/Scanning 应用容器与共享设置/Worker 的最小接口 | App shell 不再直接持有这两个应用的业务刷新/选择状态；所有旧功能和日志/托盘/中断路径具备等价入口；HackerOne 是否并入 Assets 及缓存无凭据可读单独验收，不暗中移除来源看板 |
| 4 | 技能目录与策略接口分离 | 保存旧技能 ID/内容/内置不可变与配置；instruction、knowledge、executable operation 类型不可混用；每动作模型/API/tool requirements 可解释，选择 instruction 不授予执行能力 |
| 5 | Rust 领域封装与 store/command port 收窄 | 先选 Project/Conversation 等小边界，再按 Assets/Scanning 分批把 include 移为真 module；编译最小可见性、无循环、旧 IPC 兼容；schema 重开/升级/异常中断与配置保护测试覆盖每批实际变化 |
| 6 | Chat 的显式跨应用 operation 与通用 tool-agent orchestration | 先只读引用，再按 operation 逐个加写动作；拥有 Project/实体 scope、明确审批/预算、持久 receipt、取消/重复/恢复负例；无任意 shell/URL/权限布尔捷径。多个会话的 agent 协同是后续新能力，不能复用其他 scan 的 lease |
| 7 | 执行 service 完整 owner 边界和可选独立部署 | 每消费者有自己的 admission/context/任务事件/状态 owner；保留原 scan 安全不变量；实际目标平台 sandbox 验收后才开放其能力。独立 binary/CLI/crate 若需要，另有构建和部署验收，不由目录抽取自动获得 |

这些条目是剩余工作，不是授权之外的自动执行承诺；当前批次的发布结论仅限其实际实现和实际验证范围。
