# 业务模块归属与第二轮验证记录 — 2026-10-15

本记录说明云端模块迁移的实际实现、已执行验证和剩余阻塞，不是安装包发布声明。验收代码为 `525f1283adae79a13bcf0e07cf50772841911fb0`；Phase 1–9 的云端回归已完成真实整体验证。新 `final-native-accepted` 无过滤全量：3,602 个唯一 case 全部恰执行一次：3,595 pass、7 个原 ignored、0 fail、0 filtered；outer/command/script 均 exit 0；Rust 1555.55s / Native command 1555.78s / collector 1555.91s。最终严格索引为 `integration/verification-2026-10-15-round2.json`。本地移交包仍未取得和整合，完整用户请求仍受此阻塞；Linux/default-library 与 synthetic-IPC 验收不代替 Mac/Windows、真实桌面或安装包验收。

## 来源与工作边界

| 来源 | 身份与处理 |
| --- | --- |
| 仓库 / 分支 | `swyiic/Oviraptor` / `integration/business-modules-20261014`；未合并 main。 |
| 保留的 WIP | `wip/project-agent-skills-20261014`，基线 `d61454cdf73c52d190d3d4a26d6733356a6495af`；不以 main 的配置默认值替换。 |
| 第一轮 | 验收代码 `496b98f7dee6fcd29bc275e488b539c5705c6180`，文档提交 `8c2810506e8e5f126204e2b55e4f1dbc9538f5aa`。其完整回归仅属历史，见[第一轮集成报告](BUSINESS_MODULE_INTEGRATION_2026-10-14.md)。 |
| 第二轮代码 | Phase 1–9 的验收代码为 `525f1283adae79a13bcf0e07cf50772841911fb0`。最终文档提交及远端回读由随附发布收据记录，不在本文自引用未来提交。 |
| 用户本地原基准 | `8fddee2321b313a8a6372d0895dd4c1f533ba57c`，保留用于未来三方语义整合。 |
| 本地移交包 | Library `libfile_4cfcbe4d240c8191b996a45dda2b97b8`，`Oviraptor-local-source-8fddee2.tar.gz`，预期 10,844,046 字节，SHA-256 `fc6448afee2399a988679ff64db8d95a2c5b7439e78ecd0bed05d94535fa24af`；仍未取得字节或整合。 |

本轮未访问或修改用户 Mac、用户本地源码、用户数据库或真实模型/扫描目标。没有再次尝试下载本地移交包，没有修改代理或域名白名单。现有交付接口的 workspace_path=null，仅返回 signed URL，未直接物化到本云端工作区；下载域名放行授权尚待确认，未绕过此限制。包的 checksum、restore.py、清单和 excluded-files.json 均未实际检查，不能声称已经恢复或合并。

未来取得包后，应先校验并检查恢复脚本，再恢复到独立新目录，结合原基准、清单和当前 owner map 做语义整合。过滤 bundle 根 `0d0f995a2b7312c28589c1d952ae33ffcbe045db` 不是原共同祖先，不能盲合并无关历史或把旧 binary patch 直接覆盖到迁移后的目录。

## 当前模块归属

| Owner | 前端 / Native 入口 | 责任与依赖 |
| --- | --- | --- |
| Shell / application | `src/shell`、`App.vue`；`commands::application_overview/application_settings`、`application/model.rs` | 默认 Chat 导航、可信内置模块、应用设置、概览、主题、通知和明确的跨 owner 组合。 |
| 项目空间 | `features/workspaces`；`commands::workspace_projects`、`workspaces/model.rs` | 项目身份、归档/删除及关联记录保护。跨业务指标属于项目投影，不授予扫描执行权限。 |
| Assets | `features/assets`；`commands::assets/asset_export/ownership/exposure`、`assets/model.rs` | 资产/目标、采集与日志、归属目录/分组/决策/授权、Exposure 采集/证据/复核/恢复和导出。 |
| 外部资产来源 | `features/hackerone`、配置页显式 FOFA 测试；`commands::hackerone`、`asset_fofa`、`hackerone/model.rs` | HackerOne 缓存/同步/项目导入，FOFA 请求和账户测试。没有为验证使用真实凭据或 live FOFA/HackerOne HTTP。 |
| Scanning | `features/sentinel`；`scanning.rs`、`scanning/model.rs`、原 Native runtime/pipeline | Web/Code/Greybox/CI、工作台、原 Agent、调查、证据、预算/费用/租约/权限、恢复和重放；稳定保留 sentinel 内部名。 |
| Chat | `features/conversations`；`conversations` | 独立项目对话、recipient/content consent、目录/Skill grant、工具活动与历史；不接管扫描调查消息或初始化其工作台。 |
| Skills | `features/skills`；`skills_catalog`、`skill_packages` | 已存指令/知识/来源/候选复核和显式应用；声明式包来源、完整性、导入、资源与撤销。扫描 producer 和规则仓库不归普通指令目录。 |
| Configuration | `features/configuration`；`commands::workspace_config_profiles/model_profiles`、`configuration/model.rs` | 保留原 profile、未知字段、配置值和默认值；配置页可以显式组合模型、FOFA、Worker 和扫描规则包。 |
| Environment / Runtime | `features/environment/runtime`；`commands::environment`、`worker_resources`、Worker 服务 | 准备/安装/日志/显式恢复与节点/身份/连通性分开；远程扫描操作前端归 Scanning。沙箱继续是内部能力。 |
| Shared / 基础设施 | 基础 UI、事件协调、原语 CSS/通用词条；`model_configuration`、DB/日志等窄机制接口 | 不收纳具体资产、扫描、知识流程或业务 DTO 兜底桶，不提供通用执行入口。已有 `agent_runtime` 整棵树不是无业务的 common；仅模型 gateway 等明确机制可被复用。 |

`commands.rs` 现在只有 12 个 owner 索引；实际 Scanning 为 `crate::scanning`。其内部仍使用连贯业务作用域中的原 `include!`，不是简单目录分组，也没有重写 Native Agent。125 个扫描 IPC 和 210 处 namespace 引用迁移；全部 289 个注册终端名与顺序保持。

Scanning 的 8 组生产端口和 12 个原 typed 模块保留具体合同，不开放 root wildcard 或私有 witness/writer。8 组生产端口导出 25 个命名符号：23 个函数、FinancialContinuationAuthorization 结构体和 BUSINESS_LOGIC_READONLY_SYSTEM 常量；另有 2 个 cfg(test) port 各导出 1 个测试函数。跨 owner 适配是四个 owner 的 test_support 中 8 个 cfg(test) 薄适配函数，不是 8 个模块；生产 helper 的 pub(super) 边界保留。Core ModelConnection 只有 llm/api_key/api_base/deployment 四字段；Scan 的六字段环境快照、九字段本地资源策略以及 full_power、prompt audit、材料字节预算和 startup idle/hard 时限仍归 Scan。

95 个 Rust struct 与 7 个原私有 default 函数已归 8 个 owner，serde/字段/默认值/声明保持，`models.rs` 只兼容重导出。前端 root API 仍保留 224 个方法；`src/types.ts` 仍为 type-only 兼容导出，原 128 个声明加 4 个辅助契约共 132 个。`sentinelApi` 保留明确的 catalog/learning/rule-pack/reconciliation 聚合，不能宣称全部 facade 已消除。详见[前端指南](FRONTEND_MODULE_OWNERSHIP_2026-10-14.md)和[Rust 指南](RUST_BUSINESS_MODULE_BOUNDARIES_2026-10-14.md)。

### 合法的跨业务边界与保留能力

- Shell dashboard、项目列表和 ProjectImpact 可以读取跨业务指标；项目删除须检查已知和其他关联记录，不能为减少依赖而删保护。
- StartupStatus 是 Shell 的展示契约，底层有原恢复写入：startup_status 将未完成的采集 runs 记为 interrupted，db::initialize 保留 schema、迁移和 Native 状态修复/恢复。默认打开 Chat 不表示整个启动路径只读或零写入。
- 资产和扫描仍为内置模块；当前项目继续进入独立 Chat，Skills 导入/资源浏览有独立入口，配置继续保留原字段。第一轮的 provider/endpoint consent、Skill 指纹/代次撤销和分页合同未因移位改回旧默认值。
- 42 个 CSS 连续功能段按原顺序归 owner；真正跨业务的主题、卡片/表单布局及响应式组合留在 Shell，probe 状态仍后置。两语言各 21 个词条 key/值/顺序和 locale/tr/storage 行为保持。shared 仅接收确实复用的原语，不接收具体业务。

## 原 WIP 六项问题的当前合同

`original-wip-issues-current.json` 逐项索引实现、精确 case 名和真实执行来源；该源码审查截点为 bb200ca，Phase 9 仅改 HTTP fixture，不改这些业务源码。此索引不冒充新的完整 Native suite。

| 问题 | 实际当前处理 |
| --- | --- |
| agent_enabled=false 的普通 Chat | scope revision 独立校验项目/会话；普通聊天无工具，未配置 provider 可保存用户文本，不虚构助手响应。 |
| provider consent 未绑定 endpoint | recipient 指纹绑定完整 endpoint、模型和 local deployment；旧 boolean 不自动升级；请求/FlightScope 同事务冻结并在后续边界复验。 |
| Skill 撤销与内容指纹 | 持久 generation/tombstone、文本、包来源/entry/实际资源 hash 绑定 consent；同秒恢复、重建 ID 或 disable-enable 不复活旧授权。 |
| 36 个内置 Skill 与选择数 | 实际 d61454c 和当前选择上限均为 128，不存在被核查云端源码中的 24 上限；36 skills / 443 files 保留，列表每页至多 12 项。 |
| 读取与工具输出大小 | 实际 d61454c 为一次至多 16 KiB、完整工具 Value 至多 24 KiB，并非报告中的 12 KiB；当前模型读取为 UTF-8 4 KiB 字节页，完整序列化工具 Value 上限 32 KiB，超限显式拒绝，不截断 JSON。资源 UI 默认 4 KiB、最高 8 KiB。 |
| 缺测试模块及未编译 | conversations/access/tests.rs 与 skill_packages/tests.rs 已注册并用安全合成夹具；当前真实 all-targets/opt1 build 见下表，相关阶段 case 已执行；新无过滤全量已完成，见当前验收表。 |

## 分批提交与实际验证

以下证据相对 `/workspace/validation/module-boundaries-round2`。各批变更按 commit JSON 的 rename-aware 清单报告；Phase 2 的 28 项变更含 3 项 rename，合计 31 个原/新路径，并非少记路径。阶段验证可能在提交前工作树运行，关联提交按实际 body/hash 核对；baseHead 与输入 SHA 以相应 result/快照为准。定向集合重叠，不能相加为独立覆盖数。

| Phase / commit | 具体改动 | 已完成验证 |
| --- | --- | --- |
| 1 `fc14c6ce09c0b61a70ea5762e68a7169a62f40ab`，16 文件 | 新 model_configuration、worker_resources、model_profiles；Environment 真实 module；Chat 从调用者配置快照解析模型，Scan flags/policy 留在 Scan。 | 最终 all-targets exit 0 / 67.09s，library build exit 0 / 130.77s；Native 102 通过、边界 13 通过。`phase1-final-check-fixed-result.json`、`phase1-final-test-build-result.json`、`phase1-final-targeted-result.json`。 |
| 2 `0fefa159633951688ddf3ed8b13a5e27f50d8293`，28 项 rename-aware 变更 / 31 个原新路径 | Skills 17 个原入口归 skills_catalog；Scan producer 保留；95 struct + 7 default 归实际 owner，models.rs 兼容重导出。 | all-targets exit 0 / 57.44s，最终 library build exit 0 / 24.91s；Native 149 通过 / Rust 52.99s，owner 守卫 25 通过，其中 5 条使用当时未提交的前端 WIP，不将全部 25 条归为 phase2 clean commit 上的检查。`phase2-owner-guards-result.json`、`phase2-*result.json`、`phase2-targeted-case-verification.json` 记录实际工作树证据；最终 clean 525f128 的完整 Node 另行验收。 |
| 3 `6cd48ce9927f30878a5ac70ebb8bad19aee86b42`，18 文件 | Ownership profile/目录/决策/授权/projection 从 Scan 共享 scope 归 commands::ownership；Exposure 消费显式读取 API。 | all-targets 和 library build exit 0；Native 284 通过、7 个既有 ignored。`phase3-{check,test-build,targeted}-result.json`、独立 review。 |
| 4 `80566e1df0ad27b7cf49595a2c3a8e66ccd9490a`，18 文件 | Exposure facade 与 collection kernel 同属真实 commands::exposure，保留子模块解析、数据、事务和恢复合同。 | 修正后的 all-targets 与 library build exit 0；Native 286 通过、7 个既有 ignored；精确 selectors 验证。`phase4-check-fixed-result.json`、`phase4-test-build-result.json`、`phase4-targeted-case-verification.json`。 |
| 5 `9085a9c9dc5ff70bdf17697f042d980fe4c4afd0`，7 文件 | FOFA 请求/测试归 assets/fofa.rs，HackerOne 独立 module，jobs/配置测试使用确切 owner。 | all-targets 与 library build exit 0；Native 14 通过。无既存专用 Native HackerOne helper suite；另有正文/SQL 保持审查，不冒称 live 服务测试。`phase5-*result.json`、`phase5-commit.json`。 |
| 6 `7cfcea9b7f22f780508e6e528a9216fbaf4e4386`，116 文件 | commands 变 12 owner 索引，Scanning 独立；125 IPC/210 namespace、8 ports/12 typed modules/cfg(test) 适配；17 selectors 迁移；3 retirement fixture 哈希/理由逐项审查刷新。 | all-targets exit 0 / 67.50s，library build exit 0 / 124.91s；精确 552 项中 545 通过、7 个既有 ignored、0 失败 / Rust 50.48s，17 selectors 全部实际通过，Node owner 守卫 50 通过。`phase6-*result.json`、`phase6-selector-verification.json`。 |
| 7 `fb6061889bad1c6a0a6ac4a823359fed881fd509`，132 文件 | owner API/DTO 直接接线；Environment/远程 Scan/Skills producer/rule packs 分开；224 API、128 原类型 + 4 auxiliary 保持；42 CSS 段/词条资源与真实 harness 路径适配。 | 35 focused Node 通过；原前端构建 exit 0 / 17.12s，284 个构建输入复核；99 审查输入稳定。随后完整 Node 在本提交真实失败，见下一节。`phase7-commit.json`、`phase7-focused-result.json`、`frontend-final-contract-review.json`。 |
| 8 测试适配 `bb200cab36b0f2a9dcd54c063e9e199451372fd0`，2 文件各 1 行 | AgentIdentityMaterial 的 synthetic API 注入引用已有局部 api；导航静态断言检查 runtimeApi.listWorkerNodes，保留资产 workspace guard 的顺序验证。没有生产源码变更。 | Native opt1 缓存构建 exit 0 / 0.34s；完整 Node 修复重跑真实 exit 0 / 215.77s：1,547 total = 1,535 pass + 12 skip，0 fail；全部 2,574 项输入无变化。 |
| 9 HTTP fixture `525f1283adae79a13bcf0e07cf50772841911fb0`，1 文件 1 行 | `commands/exposure/fixture.rs` 对本就只响应一次后关闭的 socket 显式发送 Connection: close；不改生产 client、timeout、重试、预算或业务断言。 | all-targets exit 0 / 15.32s；opt1/default-library 构建 exit 0 / 88.56s；完整 Node 1,535 pass / 12 skip / 0 fail，exit 0 / 126.26s；两项原 HTTP 用例 40 轮、80 次执行全部通过；Exposure 227 + 三项门禁精确 230 项中 226 pass / 4 既有 ignored / 0 fail，outer 0 / 22.85s。`phase9-commit.json`、`final-fixture-*result.json`、`final-node-fixture-result.json`。 |

阶段 test profile 为 opt-level 0；最终 opt-level 1 验收单独记录，不能混用 binary 或性能数字。`phase6-native-case-migration.json` 保留第一轮 3,600 项逻辑测试身份，新增两项 ModelConfiguration；当前最终 binary 已实际列举 3,602 个唯一项，并在新无过滤全量中逐项执行；原 3,600 + 2 的映射和执行 ledger 一起核对，不由案例映射单独推断通过。

呈现证据为 `presentation-css-equivalence.json`、`presentation-vite-css-equivalence.json`、`presentation-i18n-equivalence.json`：42 段拼接逐字节还原原 81,149 字节；展开 1,067 + 14 个 selector/context 项与 Vite 后 3,455 + 28 项声明全序保持；21 key × 2 locale、六个 storage 场景一致。它们支持机械迁移，不代替当前完整 UI 验收。

## 已保留的失败尝试及修复

| 尝试 / 证据 | 实际结果 | 原因与处理 |
| --- | --- | --- |
| `phase1-targeted-setup-failure.json` | exit 127，0 个 case 执行 | binary 列举前未 source 私有运行库环境，缺 libwebkit2gtk-4.1.so.0；加载环境后重做列表/选择/执行。 |
| `phase1-final-check-result.json` | exit 101，E0063 | 共享资源 policy 初始化漏 memory_guard_tier；补齐原字段后重做 check/build/102 项。 |
| `phase2-check-result.json` | exit 101，E0432 | DTO 模块接线误写 assets_models；修正为实际 asset_models 后重做 check/build/149 项。 |
| `phase4-check-result.json`、`phase4-check-abort.json` | 已确证 E0583；外层实际 -15 / 157.79s | 显式 #[path] 使 Exposure 子模块误向 commands/ 解析；终止已知无效编译，恢复默认 module 解析后真实重编。不能把主动中止写成 exit 101 或通过。 |
| `phase6-check-result.json` | exit 101，E0433/E0603 | 遗留 commands namespace 与跨 owner 测试对 pub(super) helper 的访问不再成立；迁移生产引用、增加 cfg(test) adapters，保留生产私有性后重编。 |
| `phase6-native-case-migration-before-skills-map.json` | 仅识别 3,598 个原名逻辑项 | 两项测试实际迁到 skills_catalog，映射尚未登记；补精确 namespace 映射后保留 3,600 + 2，未删除用例或更改断言。 |
| `final-node-result.json`、`final-node.log` | **exit 1 / 118.59s；1,547 total = 1,047 pass + 488 fail + 12 skip** | 487 项为 harness 局部 sentinelApi 未绑定，另 1 项为 module navigation 静态断言仍检查旧 API alias。最小两行测试适配已提交为 bb200ca；完整重跑真实 1,535 pass / 0 fail / 12 skip，exit 0 / 215.77s；逐项标题与跳过多重集保留，不覆盖这次失败。 |
| `final-native-full-result.json`、`final-native-full-full-result.json`、`final-native-full-abort.json` | **旧 bb200ca 全量不完整：主动 SIGTERM，outer 143；Native 563.00s / wrapper 563.13s；1,849 个 case rows，2 FAILED、7 ignored，summary=null、accepted=false。** | 全量中 G2/Q2 出现失败。Q2 同 binary 精确诊断在第 9 轮实际 exit 101 / Native 0.19s（outer 0.423s），断言为 transport_failed 对 redirect_hop_limit。独立 A/B 实证后 root 结束已知失败 run 以修复重跑；缺完整 footer，G2 原 full 的具体失败 body 未取得，不能宣称与 Q2 同一精确断言。中止前后源码/路径集/二进制 SHA 均不变。 |
| `http-fixture-protocol-ab/run-80-result.json`、`proposal-and-validation-design.json` | 原 fixture 160 trials 中 24 次 OS104 ConnectionReset；Connection: close 对照 160/160 通过。 | 独立诊断复用逐字节原 fixture/client constructor，仅响应头不同；24 次错误在 5–35ms 出现，不能用来提高 timeout。它支持一响应后关闭却未宣告 Connection close 的协议竞态归因，不代替原应用用例或完整验收。Phase 9 一行 fixture 修复后真实重编、80 次原用例及整个 Exposure 集执行见上表。 |

三项 retirement fixture 仅更新 reason 和内容 hash，literal 数量 4/4/1、pathMatch=false、50 项路径/类别、固定 scopes 和空 active baselines 不变。实际 Native literal/active/importer 守卫在 Phase 6 定向集中通过。审查后重复运行严格预演生成器因输入已被应用而拒绝，不是 Native 测试失败；proposal 元数据命名说明和原哈希分别保留，没有盲刷新其他记录。

## 当前整体验收状态

| 项目 | 当前实际记录 | 证据与边界 |
| --- | --- | --- |
| 生产前端构建 | `final-npm-build-result.json`：fb60618，exit 0 / 18.35s，输入无中途变化。 | Phase 8/9 仅改测试工具/HTTP fixture，生产前端不变；当前 525f128 的浏览器 281/288 输入重新逐 SHA 核对一致。 |
| 当前 all-targets check | `final-fixture-cargo-check-result.json`：525f128，exit 0 / 15.32s。 | 固定输入，无新增编译失败；此前阶段和旧最终 check 记录保留。 |
| 当前完整 Node | `final-node-fixture-result.json`：525f128，1,547 total = 1,535 pass + 12 skip，0 fail；exit 0 / 126.26s。 | 2,574 个输入无变化。旧首次 1,047/488/12 与 bb200ca 修复后 1,535/0/12、215.77s 仍独立保留；`node-case-migration.json` 保留 96 个入口、原 1,524 + 新 23 个 occurrence 及 12 项 skip。 |
| 当前 Native opt1 build | `final-fixture-native-build-result.json`：525f128，exit 0 / 88.56s；binary SHA-256 474e15f2d28932a485b020fcdda03204c9d5b3b8cdf1c62485b9e0efd0d0fd1c。 | default-feature library，opt_level=1、debug_assertions/overflow_checks=true；native/audit/tracked 1,800/2,218/2,569 路径及 SHA 均无变化。旧 382.07s 和缓存 0.34s 构建仍保留，不混用 binary。 |
| 原 HTTP 两项重复执行 | `final-fixture-repeat-result.json`：40 轮、每轮 exact 两项 / 两线程，共 80 case executions，全部通过，outer 0。 | 原 G2/Q2 断言和 timeout 未改；重复执行是诊断/回归，不是测试内部隐藏重试，也不代替完整 suite。 |
| 当前 Exposure / retirement | `final-fixture-exposure-result.json`、`final-fixture-exposure-verification.json`：227 Exposure + 3 gates 精确选择 230；226 pass、4 既有 ignored、0 fail；outer 0 / 22.85s。 | case set、数量和唯一性均实际核对；四项 ignored 不算通过。 |
| 新完整 Native 3,602 项 | `final-native-accepted-full-result.json`：3,602 个唯一 case 全部恰执行一次：3,595 pass、7 个原 ignored、0 fail、0 filtered；outer/command/script 均 exit 0；Rust 1555.55s / Native command 1555.78s / collector 1555.91s。 | source525f128 / default-feature library / opt_level=1 / 8线程 / debug_assertions和overflow_checks=true；native/audit/tracked 1,800/2,218/2,569 个路径及全部 SHA 在运行前后精确一致，binary SHA-256 474e15f2d28932a485b020fcdda03204c9d5b3b8cdf1c62485b9e0efd0d0fd1c 不变。严格索引重新解析 raw footer、逐 case ledger、原 ignored 集、3600+2 双射及完整前后输入证据。 |
| 当前 Chromium | fb60618 的真实 Chromium / 实际 Vue / synthetic IPC：18/18，outer 0；runtime error / unknown IPC / external fetch 均 0。 | `browser/current-input-verification.json` 于 525f128 再核 281 个 source / 288 个 browser+QA inputs，路径集与 SHA 全部相同；没有假称在 525f128 重新执行浏览器，也不代表真实 Tauri/用户库/模型后端。 |
| 当前迁移与发布审计 | 新无过滤全量中 retirement literal / active baseline / retired importer 三项实际全部通过；严格索引再次复核原 retirement 审定的 3 项 fixture context，路径/类别/数量/固定 scopes/空 active baseline 未放宽。 | `retirementReviewBoundary` 绑定 allowlist SHA-256 23ae6cdbbb03b10774e089e0c1c4694e659fa44c7747d5f10b7682f88aa20ee7；216 个迁移源文件 hash 和 owner/API/DTO/IPC map 维持原静态证明。验收代码为 525f128；最终 docs-only 提交及远端回读由提交后发布收据记录，本文不预报提交或推送成功。 |

日志中的阶段通过、静态同一性、实际编译、定向执行、完整 suite 和安装包验收是不同证据。当前云端回归的已执行范围已通过严格验收；新 `final-native-accepted` 的完整结果独立于第一轮 496b98f，不迁用旧通过数。本地应急源码未整合、付费续跑和平台/安装包限制仍按下节保留。

## 剩余限制与后续整合

1. 本地移交包未取得，因此用户允许的本地应急源码尚未参加三方语义整合；本轮没有新下载重试或内容检查。云端成功回归也不能消除该阻塞。
2. 既有付费暂停后成功 Resume/Retry 仍缺显式财务续跑合同。两项第一轮测试已改为验证真实付费后安全拒绝及独立 fresh Root 轮次合同，不表示成功续跑已经实现。
3. legacy None 重入保留原 lease 三个时间字段的既有心跳写入；其它状态/身份/费用/请求不增加，不能概括为物理全表零写。
4. Linux 不执行两项 Mac exclusive-publication 成功用例或 Darwin 专属分支；Linux 的 unsupported 无副作用负例不代替 Mac 正例。Mac/ARM64、Windows、实际安装包、真实 Tauri IPC/用户数据及真实模型/扫描目标尚未验收。
5. 七个既有 ignored 项包括 live 外部读取、规模测试和提取项目 acceptance，未算作通过。可选 import-tools 等额外 feature 必须单独报告，不能由 default-feature 结果外推。
6. 第一轮完整失败 run 的 Rust FAILED summary 和 3,501/77/7 保留；外层退出码未恢复，仍为 null。旧 77 项最终覆盖区分 73 原名通过、2 行为合同替代、2 Mac 正例未在 Linux 执行，不将它们统称“原 77 项均恢复通过”。

第二轮 current migration map 已记录文件/函数/声明/module/IPC/API/CSS section 的原新路径与 commit，静态核对涉及 216 个源文件，并保留第一轮 17 组件、2 个 Skills 路径和 128 类型的历史定位。它辅助未来从原本地基准合并语义，不是把过滤 bundle 当共同祖先或自动重放旧补丁的许可。
