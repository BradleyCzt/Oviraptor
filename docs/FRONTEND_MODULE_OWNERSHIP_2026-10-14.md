# 前端业务模块归属与迁移边界

本版描述第二轮前端 owner、API/DTO 和呈现迁移，行为仍来自云端 WIP `d61454cdf73c52d190d3d4a26d6733356a6495af`，不以 main 默认值替换。原生 owner 迁移为 `7cfcea9`，前端 Phase 7 为 `fb60618`，当前验证源码冻结为 `525f1283adae79a13bcf0e07cf50772841911fb0`；完整验收由第二轮报告记录。

## 业务归属

| Owner | 实现与契约 | 责任 |
| --- | --- | --- |
| Chat | `features/conversations/` | 独立项目对话、历史、权限、工具活动和当前项目上下文；不接管扫描调查时间线。 |
| Assets | `features/assets/` | 资产表、目标、采集 jobs、归属、人工判定和操作日志；显式确认/恢复保留原合同。 |
| Assets.Exposure | `features/assets/exposure/` | 暴露面页面、设置、证据、轮次、复核与恢复；消费资产领域的明确读取/选择契约。 |
| HackerOne | `features/hackerone/` | 保留外部来源的独立内置入口和缓存；向项目导入仍显式选择项目。 |
| Scanning | `features/sentinel/` | 稳定保留 sentinel 内部名称；工作台、任务、调查消息、证据、结果、费用/授权收据、恢复和扫描日志。 |
| Skills | `features/skills/` | 指令编辑、包导入/资源浏览、知识来源与目录、候选复核及显式应用；由 Chat 和 Scanning 分别消费。 |
| 项目空间 | `features/workspaces/` | 项目身份、list/save/impact/archive/delete 和项目对话框。项目聚合及删除保护不等于扫描执行权限。 |
| Configuration | `features/configuration/` | 完整配置界面和 profile/provider 契约，保留既有值、未知字段和默认值；显式组合模型、FOFA、扫描规则包等设置。 |
| Environment | `features/environment/` | 环境检查/安装、安装日志、准备状态、工具管理和显式恢复。 |
| Runtime | `features/runtime/` | Worker 本机设置、节点 CRUD、身份、连通性和远端环境查询；沙箱仍是内部基础设施能力。 |
| Shell | `shell/`、`App.vue`、可信 builtin 注册 | 导航、应用设置、概览、StartupStatus 展示、主题、全局通知与跨 owner 组合。默认启动 Chat，资产和扫描保持内置模块。 |
| Shared 机制 | `shared/components/`、共享事件协调、基础样式和通用词条 | ModalShell、InlineConfirm 等无业务状态的原语；不导入业务 DTO、业务 API 或平台执行能力。 |

第一轮原全局 components 的 17 个组件已归实际 owner，Skills 目录组件/composable 不再反向依赖扫描工作台。Scanning 调用 Skills 的目录契约，自己的调查产出和任务选择适配仍归 Scanning。

## API 和类型边界

实际 Vue/composable 消费所属 owner API。以下兼容导出继续保留，不将“调用归 owner”写成“全部 facade 已删除”：

- `src/api.ts` 是明确的 spread-only 兼容组合，原 224 个公开方法仍存在，无缺失、新增、运行时正文变化或 spread 同名覆盖。224 是前端方法数量，不是原生 IPC 注册数量。
- `features/sentinel/api.ts` 保留 reconciliation、Skills catalog、扫描 learning 和 rule-pack 的显式聚合，以兼容已有扫描调用。
- `src/types.ts` 是 type-only re-export。原 128 个类型声明保持；新增显式辅助契约 InstallLogRow、InstallLogPage、SecurityRulePackInput、AgentSkillInput，当前共 132 个。它们不产生运行时代码或新的执行权限。

原 128 项历史映射继续保留在[第一轮迁移表](integration/module-moves-2026-10-14.json)；新增 owner 和辅助类型使用第二轮 current map/验收记录，不覆盖旧来源。当前 DashboardStats、StaleProject、StartupStatus 归 Shell；环境安装日志归 Environment；知识条目/候选归 Skills knowledgeTypes；扫描规则包归 Scanning rulePacksTypes。不要将业务类型搬入 shared 来隐藏依赖。

| 接口边界 | 当前规则 |
| --- | --- |
| 项目 / Shell / 配置 | workspaceApi 只保留项目五项操作；shellApi 管应用概览、设置和 StartupStatus 调用；configurationApi 管 profile、模型/FOFA 测试等设置入口。 |
| Environment / Runtime | 环境准备 API 和安装日志 DTO 属 Environment。Runtime 保留 Worker 节点/身份/连通性，远端环境查询可使用 EnvironmentReport。 |
| 远程扫描 | `sentinel/remoteWorkerApi.ts` 管任务列表、控制和历史同步，保留 nodeId + scanId、action 及原 IPC 参数。 |
| Skills 生命周期 | instructionCatalogApi 与 knowledgeCatalogApi 合计 17 个原入口，管理已保存指令、知识、来源、复核和显式应用；scanId 作为已存记录过滤条件不等于实时扫描生产。 |
| Scan producer / rule packs | `sentinel/learningApi.ts` 的两个入口产生/聚合扫描知识；`sentinel/rulePacksApi.ts` 的四个入口管理扫描规则仓库及同步，不能因 UI 名称含 Skill 就归指令目录。 |

设置页和 WorkerSettingsPanel 可以显式组合多个 owner。特别是 Worker 设置中的远程任务操作属于 Scanning，节点身份属于 Runtime，环境报告属于 Environment；产品组合入口不等于这些业务已成为一个公共机制层。配置页可以调用外部来源测试，但不会因此将 Native FOFA 实现从 Assets owner 转为通用模型基础设施。

## 呈现归属与兼容行为

`styles.css` 保留为有序组合入口。原 890 条顶层规则按 42 个连续功能段迁移，业务段归 Assets、Scanning、Configuration、Workspaces、HackerOne、Environment。跨业务的项目/profile 卡片、查询/配置表单、桌面/紧凑主题和混合响应式布局留在 `shell/styles/`，有原 range 与保留理由；shared 只含基础样式原语。迁移不将整站规则按 owner 重新分组。

`probe-status.css` 仍在原位置作为后加载兼容入口，具体状态规则归 Assets。42 段按入口顺序拼接逐字节还原原 81,149 字节；展开后 selector/声明/at-rule context 次序一致，Vite 处理后的声明次序也一致。仍有历史 root CSS 文件供实际 owner 导入，不把这批迁移宣称为所有样式路径已完全独立。

根 `i18n.ts` 保留 locale/tr API、本地存储迁移和组合机制；业务词条归 Shell、Assets、Workspaces、Configuration，common 只有通用动作词。两种语言各 21 个公开 key、值和枚举顺序不变。合并优先序明确为 shell → assets → workspaces → configuration → common；当前 key 互不重复，未来有意重复时后者覆盖前者。

`useCommittedRefresh` 仍是 Assets 与 Scanning 复用的事件协调机制，不承接业务状态。共享弹窗、确认、事件清理和基础控件可以复用；业务权限、恢复动作和 API 调用留在 owner。

## 合法的跨业务与启动边界

Shell 允许展示跨项目/资产的 dashboard 和跨 owner 状态。项目 impact 和删除前检查需要读取资产、扫描、知识、认证会话及其他关联记录；这是项目生命周期保护，不能为消除跨域依赖而删除。

StartupStatus 属 Shell 的展示契约，底层 Native 启动路径有既有恢复写入：`startup_status` 会将未结束的采集 runs 标记为 interrupted，`db::initialize` 仍执行 schema、迁移和 Native 状态修复/恢复。前端调用名称或默认 Chat 路由不表示整个启动过程只读、零写入，也不改变这些 owner 的原授权合同。

## 验证与当前限制

前端准备批次为 132 个变更文件。独立审查确认 224 个兼容 API、128 个原类型及四个辅助契约的 owner；99 个原审查输入在后续复核保持不变。`frontend-final-contract-review.json` 记录真实方法/声明比对；没有以类型更名或目录数量代替行为验证。

已记录的生产构建 `npm run build` exit 0 / 17.12s，构建输入无中途变化，相关前端源保持。CSS/i18n 同一性证明、六种 storage 场景和项目删除 18/18 回归分别保留；Phase 7 focused owner/接口测试为 35/35。harness 加载真实 SFC 和 owner API，仅用 synthetic native invoke 隔离 IPC，不复制业务实现或削弱断言。

最终生产前端构建另有 exit 0 / 18.35s。Phase 8 的两行测试适配与 Phase 9 的 HTTP fixture 一行修复不改生产前端。完整 Node 首次真实 1,047 pass / 488 fail / 12 skip（exit 1 / 118.59s）；487 项为 synthetic API 注入局部变量漏绑定，另 1 项为导航静态检查的旧 alias。bb200ca 两行修复后真实全量 exit 0 / 215.77s；当前 clean 525f128 再跑为 1,535 pass / 12 skip / 0 fail、exit 0 / 126.26s，2,574 输入无变化，原 occurrence/skip 身份保留。

Chromium 在 fb60618 的实际 Vue + synthetic IPC 执行 18/18，runtime errors / unknown IPC / external fetch 均 0。525f128 重新核对 281 个 source 和 288 个 browser/QA inputs 的路径集及全部 SHA，完全相同；不是在新 HEAD 再执行浏览器。当前迁移 map 的 216 个源文件 hash 也全部相同。

Phase 2 的 25 条 Node owner 守卫中有 5 条使用当时未提交的前端 WIP；该阶段实际工作树 baseHead/输入 SHA 及 body/hash 关联由 result/快照记录，不将其全部称为 phase2 clean commit 验证。当前 clean 525f128 的完整 Node 是另一次独立执行。新完整 Native `final-native-accepted` 已实际完成：3,602 个唯一 case 全部恰执行一次：3,595 pass、7 个原 ignored、0 fail、0 filtered；outer/command/script 均 exit 0；Rust 1555.55s / Native command 1555.78s / collector 1555.91s。native/audit/tracked 1,800/2,218/2,569 个路径及全部 SHA 在运行前后精确一致，binary SHA-256 474e15f2d28932a485b020fcdda03204c9d5b3b8cdf1c62485b9e0efd0d0fd1c 不变；它与 synthetic IPC 浏览器、Node 和前端构建分别取证，仍不代替真实 Tauri/用户数据或 Mac/Windows 安装包。

模块守卫检查真实 Vue/TypeScript import/export/dynamic import、shared 依赖、Skills/Scanning 方向、owner 类型不反向引用旧 facade，以及 App 同步闭包不引入懒加载业务页面。源码证明、定向 DOM/IPC 和构建不等价于真实 Tauri 桌面或网络扫描验收。

第二轮最终构建、完整 Node、Chromium、完整 Native suite 与冻结 source/hash 的当前结果见[第二轮报告](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md)及结构化验收索引，新全量及最终严格索引是当前云端验收证据，阶段验证仍不代替完整执行。[第一轮集成报告](BUSINESS_MODULE_INTEGRATION_2026-10-14.md) 保留 `496b98f` 的完整验收和 `8c28105` 文档历史，不能将旧通过数移用到当前版本。本地源码移交包仍未取得和整合，接口 workspace_path=null、仅 signed URL，域名放行未确认，本轮无新下载重试；真实模型/扫描目标、用户数据及 Mac/Windows 安装包不由现有合成回归覆盖。
