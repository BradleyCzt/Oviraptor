# Asset 暴露面独立开发规范：Grok 执行合同

日期：2026-10-08。状态：开发规范，新增能力尚未实施；本文不是交付验收报告。
源码根：`/Users/swyiic/Desktop/Rust/Oviraptor`。本任务不接 Agent、不改 NEST，不阻塞 Master 的运行时开发。

## 0. 给 Grok 的直接执行指令

先阅读本文及文末列出的当前源码，核对工作树和既有交付，再按 G0 → G1 → G2 → G3 → G4 开发。
你的职责是 Asset 暴露面的采集、证据、信息分类、事件聚合、覆盖报表和专属页面；不是资产归属系统、扫描执行器或通用 Agent 系统。
本文是新的独立实现合同。此前 Qoder 文档中的兼容性、真实性和数据保护要求继续有效；涉及 E6/模型分析的部分在本任务中仅保留文档合同，不接生产模型。
同一文件只有一个实现负责人。用户应将 exposure 白名单交给 Grok，Qoder 不再同时修改该白名单；发现其他进程修改时保存指纹、列冲突路径，协调对应文件的修改时段，不终止对方、不覆盖对方结果。不要根据时间戳猜作者。
未经核对不能宣布当前已有功能，也不能把本文列出的规划字段当数据库现存字段。
禁止 reset/restore/clean/stash、批量格式化、自动 commit/push、替换安装 App、清理真实用户库或删除 asset。
保留 Native JSON；不恢复 Strix，不修改 Master 的格式、启动协议或历史环境。
没有凭据、共享合同或真实来源时，交付明确的依赖与未配置状态；继续独立工作，不编造外部成功证据。

## 1. 产品决策：本期为什么不接 Agent

接 Agent 会新增模型配置、费用、输入脱敏、结构化输出校验、重放、取消、故障与授权等闭环；仅增加一次模型调用不能算集成完成。
本期用确定性采集 + 可解释规则 + 人工复核获得可用结果，工作量按来源和验收闭环切分，不给整个任务虚构人天或完成百分比。

| 本期实施 | 本期不实施 |
| --- | --- |
| 集团/单位视角的只读聚合，复用现有目录 | 新建第二套集团/省市目录或修改归属门禁 |
| 自有站点、证书/历史索引的现有采集收敛 | 自主决定扫描目标或动态扩大授权 |
| 一个真实代码平台适配器：GitHub | 全网平台万能爬虫、代码执行或完整历史克隆 |
| 文档/情报的来源标注导入、明确公开 URL 的有界读取 | 无可用接口却声称完成文库、网盘、暗网检索 |
| 分类、招标排除、证据版本、事件去重、复核/整改 | 模型推断直接确认泄漏、自动操作凭据 |
| 真实覆盖和缺口报表 | 创建 Root/worker/账单、复制 Tool Broker 或 Agent runtime |

一期最小可用结果：一个项目的一省/一个单位可以看到资产底账、待归属/待授权量、实际排查渠道、可追溯线索、被排除的公开材料、复核与整改记录；同一文件的多处传播不会重复制造集团风险数量。
如果缺少单位的权威归属边，集团总账与“单位未确定”仍可使用，不能用推测省份伪造省级验收。

## 2. 本次只读核对的真实基线

生产入口：`commands/exposure.rs::start_exposure_scan` → `execute_exposure_run` → `exposure/orchestrator.rs::run_plan`。
已注册开始/恢复 IPC；当前来源注册仅 `passive_web`、`certificate_transparency`、`web_archive`。
自有站点从首页、robots、sitemap、security.txt 等种子跟随观察到的资源；CT 使用 crt.sh，历史索引使用 Wayback。
`rules.rs` 按内容/路径生成凭据、Source Map、文档及外链候选，没有调用模型 SDK、Coordinator 或 Agent 派发。
已有 exposure 专属配置、工作项、观察、线索版本、复核、整改、缓存及聚类持久化；应扩展这些对象，不能另起整套采集引擎。
`clusters.rs::registered_domain` 当前仍取最后两段，不能作为正确的 PSL 归属边界。
`contract.rs` 的部分固定限制与现行设置/WalkBudget 需在 G0 对齐，只能由实际执行计划生成页面限制说明。

组织目录、项目绑定和后台归属组已经出现新的实现/交付记录（O0/O1/O2）。这是并行演进模块：G0 必须以当前代码和合同验证为准，不能照抄此前“完全未实现”的设计，也不能把交付文档自报测试当本任务实测。
当前可见 `org_entities`、`org_entity_identifiers`、`project_org_bindings`、`ownership_review_groups` 等；`candidate_entity_id` 是候选，不是已确认的资产单位归属。
以上为源码可达性核对，本次没有运行采集、测试或写用户库，不证明第三方来源当前可用。

## 3. 文件所有权：严格白名单

Grok 可以修改：

- `src-tauri/src/commands/exposure.rs`（仅 exposure 模块装配/命令实现）。
- `src-tauri/src/commands/exposure/**`（含模块私有 serde 合同、schema、适配器及测试）。
- `src/features/assets/exposure/**`。
- `src/components/ExposureSurface.vue`、`src/exposure-surface.css`、`src/exposure-tabs.css`、`src/exposure-runs.css`。
- `tools/test_asset_exposure*.cjs` 和新的 `tools/test_exposure_*.cjs`，仅暴露面测试。
- `docs/ASSET_EXPOSURE_GROK_*` 专项交付/依赖文档。

Grok 不得修改：

- `agent_runtime/**`、`commands/agent_*`、`commands/agent_native/**`、`commands/multi_agent/**`、`native_*`、控制/授权/预算/沙箱/恢复/日志环境代码。
- `lib.rs`、`commands.rs`、`db.rs`、`db_schema.rs`、共享 `models.rs`/types、`Cargo.toml`/锁文件、`package.json`/锁文件、全局配置和安装脚本。
- `assets.rs`、`asset_export.rs`、项目管理、`AssetWorkspace`、App.vue。
- `asset_ownership.rs`、`ownership_directory.rs`、`ownership_review.rs`、共享 PSL、`AssetOwnershipGate.vue`、`OwnershipProjectSetup.vue`、`OwnershipReviewGroups.vue`、`ownershipDirectoryApi.ts` 和归属测试。
- Master Plan、progress、REM、总交接，以及本规范正文；需变更规范时写专项偏差申请，不能自行把禁止项改成允许项。

新增共享 IPC 注册、依赖、秘密存储、公共只读合同交给主控串行装配。提交 `ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md`，逐项写请求字段、调用方、版本、错误、最小改动路径和测试；不要直接修改共享文件或故意让全项目处于无法编译的状态。
新 Rust DTO 优先放模块内部并显式 serde，不为便利修改共享 models；新前端类型放 exposure 专属 contracts。
新模块可在现有 exposure 内注册；待注册的新 IPC 显示 `integration_pending`，不得伪称页面已可用。

## 4. 独立架构和共享依赖

```text
项目/目录/资产的只读投影
    → 冻结暴露面计划与查询策略
    → 现有工作项/请求与字节额度
    → 来源适配器（网络、索引、导入）
    → 不可变观察及脱敏证据
    → 可解释规则与正常公开排除
    → 信息线索 → 泄漏事件与传播位置
    → 人工复核 → 整改 → 新轮复测 → 覆盖报表
```

整个链条不创建 NEST Root，不访问 Agent 账本，不发送 Agent mailbox，不读取扫描的模型或浏览器身份，不增加 Docker/Redis/Python/Tor 服务。
继续复用 exposure 的 PassiveClient、RequestMeter、工作项认领、取消、恢复和证据 writer；平台读取是来源适配器，不能建立通用命令执行器。

共享依赖分为三个只读接口，名称是设计概念，不能冒称已有 API：

| 接口 | 所需内容 | 不允许的隐式行为 |
| --- | --- | --- |
| DirectoryProjection | entityId、parent/group、类型、名称别名、经营地域/服务地域、目录/绑定版本 | 自动新建实体、改父子关系、复制项目授权 |
| AssetUnitProjection | projectId/assetId、当前项目成员资格、归属状态、实体关系及证据、授权状态/有效期、版本 | 用候选组确认资产归属、从省名自动归类专业公司 |
| DomainBoundaryPort | ICANN/PRIVATE PSL 边界、算法/数据版本 | 最后两段回退后仍标正确注册域、靠域名自动赋权 |

先核对现有共享只读能力。缺失时记录依赖：可继续精确 host 聚合，但必须标 `exact_host`；未知单位保留 `unassigned`。不得复制共享 PSL 或组织目录形成第二套来源真相。
组织文本查询词只来自项目绑定范围内的已确认名称/别名/域名；弱 company 标签只给待复核建议，不加入已确认集团成员总数。

## 5. 集团、省、市和资产的聚合口径

集团总账和单位排查并行：先保留全量候选账，再按已确认的单位关系分配；不要先扫描全部候选后才处理归属，也不要为等全部归属复核而阻止已有授权资产的工作。

- 管理单位 `ownerEntityId`、经营地域 `jurisdiction`、服务地域 `serviceRegions` 分列。
- 浙江省公司与杭州专业子公司不是同一实体；江苏省公司与苏州专业子公司不能按地名合并。
- 城市分公司按目录里的权威父子关系进入省公司，不靠“杭州/南京”关键词推断。
- 共享平台使用共享实体或显式多单位关系；同内容、证书、IP、CDN 或云租户不是归属证明。
- 省/市视图是集团同一账本的筛选视图，不复制资产。专业子公司、共享资产、单位未确定各有独立分组。
- 不因切换项目复用授权。复用目录可以减少录入；每项目的扫描准入仍读取原项目的独立确认和授权。
- O2 导航桶用于导航，`candidate_entity_id` 用于建议；只有权威确认边才能用于“该省已确认资产数”。
- 暴露面可以复核一个事件涉及哪些单位（独立证据关系），不能借此修改 assets 的归属。

统计至少区分：候选关联数、确认集团资产数、确认单位资产数、单位未确定数、待授权数、本轮计划数、已实际排查数、未覆盖数。
集团去重按稳定 asset_id/明确逻辑实体关系；不同协议、端口、系统和租户不盲目合并。共享资产在集团只计一次，在涉及单位分别显示关系，不能简单相加得到集团总数。

## 6. 排查对象：信息类别与来源分列

| 信息类别 | 首期规则与人工依据 | 排除/限制 |
| --- | --- | --- |
| 内部业务资料 | 明确内部标记、内部流程、未公开报表/合同/报价、项目特定文本组合 | 普通公司名、宣传、公开说明不算泄漏 |
| 技术与网络资料 | 系统名 + 内部地址/拓扑/配置等组合 | 单个公共 IP、通用教程不足 |
| 源代码/开发交付 | 第一方源码、业务配置、CI/调试记录、Source Map 内容证据 | vendor、公开开源、空图/错误页分开 |
| 凭据 | 类型规则 + 上下文；值只保存必要脱敏片段与不可逆指纹 | 占位/样例单列；不登录或测试有效性 |
| 员工/协作数据 | 非公开通讯录、工号与联系信息等组合 | 公开领导介绍、招聘/客服联系方式排除 |
| 客户/业务数据 | 非公开名单、业务记录、数据库/表格导出线索 | 匿名统计、正常客服页面排除 |
| 文件/存储误暴露 | 公开可访问的有依据备份、日志、内部附件或目录线索 | 一般公开图片/下载不算泄漏 |
| 售卖/被盗情报 | 公开 feed/用户报告的声明、样本与来源等级 | 卖家宣称不等于已验证事件 |

来源渠道另列：`first_party_web`、`certificate_index`、`archive_index`、`code_platform`、`document_link`、`document_import`、`public_intel_import`。
分类必须输出 ruleId/version、命中的脱敏证据、反证/缺口、可达性、归属依据。规则置信度是分值，不是统计概率，不宣称“62% 真实泄漏”。
凭据、员工/客户规则采用组合证据；中文短别名、常见姓氏、手机号正则单独命中只能是待分类线索。

## 7. 正常公开与招标排除：先于风险生成

明确的招标、采购、中标公告及正常附件/转载默认硬排除，不纳入泄漏风险数、泄漏事件或整改任务。
目录名或标题含“招标”不能自动证明整份文件是正常公告；必须记录来源类型、规范来源或可核验转载关系。无法确定进入 `classification_pending`，不能直接确认风险。
广告、新闻、公开领导信息、正式开源资料、公开产品文档分别记录正常公开理由。
排除判定使用 `excluded_public_procurement`、`excluded_public_material` 等稳定码及 policyVersion、observationRefs，不物理删除资产、观察或旧复核事件。
不能仅按 host 删除全部线索；单个招标公告的正常公开判断不能扩展到同平台所有文件。已明确属于正常公告的记录不得自动解除排除。
内容混合、来源冲突或疑似非公开附件进入人工分类；本期不实现自动例外策略，不借模型推测恢复风险。
排除只能减少风险计数，不应从覆盖分母偷偷抹除该次已执行查询；“排除材料数”和“待分类数”分别显示。

## 8. 来源适配器合同与查询计划

适配器统一描述：sourceKey、adapterVersion、能力、所需凭据、支持的内容类型、分页方式、配额、可用状态、证据等级。
每个查询冻结 queryId/hash、projectId、entity/identifier 版本、来源配置版本、scope/policy/ruleVersion、请求/字节/页数限额和时间窗口。
能力要具体：`repository_metadata`、`default_branch_code_search`、`explicit_document_url`、`file_import`；不允许一个开关写“GitHub 全覆盖”。
计划从已确认根域名、邮箱域名、实体全称/经确认别名、明确系统标识派生；禁止穷举员工姓名或任意扩展短词。弱候选进入查询建议，需用户明确选择后才能成为新查询计划。
实体共用关键词合并一次查询，保留 query→entity 多对多关系；结果归属仍需证据，不能因命中词就自动落到所有省公司。

通用结果至少含：queryId、providerObjectId、canonicalUrl、observedAt、indexedAt、fetchedAt、paginationCursor、providerTruncated、contentHash、contentType、bytes、redactedExcerpt、collectionMode、observationRefs、errors。
摘要命中与下载正文分开记录。没有正文就不能声明“已分析文件内容”；没有新请求就不能声明“当前仍可访问”。

来源状态必须区分：

| 状态/原因 | 解释 |
| --- | --- |
| `configured` / `unconfigured` | 来源/凭据是否就绪 |
| `unsupported` / `integration_pending` | 未实现能力/尚未装配共享接口 |
| `running` / `completed` / `partial` | 当前有界计划的执行情况 |
| `failed` / `rate_limited` / `auth_failed` | 来源真实错误 |
| `blocked_authorization` / `blocked_quota` | 授权或额度阻止后续读取 |
| `cancelled` / `interrupted` | 用户取消或进程中断 |
| `completed_no_matches` | 查询确实执行、当前有界结果未命中；不代表没有泄漏 |

这些细粒度状态通过 exposure 自有合同/附加状态存储表达，不能擅自修改共享 run enums。未知来源拒绝或明确 unsupported，不静默当成功。

## 9. GitHub：第一条新增真实网络来源

先实现公开仓库元数据搜索，再实现受控代码搜索和少量命中文件读取。使用 GitHub 官方 REST API，不能用页面 DOM 抓取冒充稳定搜索接口。
生产适配器与 localhost fixture 使用同一解析、分页、限额和持久化路径，fixture 端点不得混入生产配置。
代码搜索的认证要求、可见范围、默认分支/文件大小/结果数量限制以当前官方合同和实际响应为准；默认设计凭据为可配置必需能力，缺失时显示 unconfigured/auth_failed，不尝试绕过或偷用其他账号身份。
仓库元数据可以使用官方允许的匿名能力；是否有凭据按能力分别显示，不能因元数据搜索成功就标代码搜索也已完成。
固定外联服务配置限定 api.github.com 的批准 API 路径；读取命中正文时使用明确对象 id/ref，校验返回仓库/路径/blob 对应关系。API 返回 URL 不是通用出站授权。
只接公开资源；令牌能看见私有仓库也不意味着本任务获准读取。禁止 clone 全仓库、遍历全部历史、执行仓库脚本、安装其依赖或下载无界 release。
分页消费实际 cursor/Link 与 incomplete_results；记录已读页/对象和未读尾部。403/429/422/503、缺凭据、额度耗尽、空结果分开；重试有次数/等待总限额，不能持续轮换凭据。
每次 HTTP 尝试都扣 exposure 请求额度，字节按实际定义计入；本期没有模型 token/USD 账单。恢复不重置原额度，不把重新发送说成零支出。
token 只能经主控装配的秘密引用端口提供，配置/日志/证据/CSV 不保存明文。没有现有可复用端口时提出共享集成请求；不要改全局环境或将 GitHub token 放进 URL。
重定向跨 provider/host 时不转发 Authorization，未经来源配置授权的新域记录为缺口。
GitHub 正式开源与其他公开材料排除依据应能人工维护；名称像集团不证明仓库归属，仓库是转载也不证明第一方。

官方来源（2026-10-08 查阅）：[GitHub REST Search](https://docs.github.com/en/rest/search/search?apiVersion=2022-11-28)。搜索有结果窗口和不完整结果标记，不能将读完接口窗口宣称全平台检索完毕。不要把当日固定速率数字写死为长期服务合同。

## 10. 文档、网盘和情报：先做可核验入口

G3 提供两条明确能力：

1. **文件导入**：首期 UTF-8 文本、结构化 JSON/CSV 与已提取的文档文本；保存来源声明、文件 hash、导入批次、操作者、时间和格式。标 `document_import`，不是应用从外网观察到的正文。PDF/DOCX/OCR 只有完成真实解析器/配额/测试后才能在能力列表开放，不伪装解析成功。
2. **指定公开文档 URL**：用户明确提供 URL，经独立 external-source 配置准入，在公开地址上有界读取 HTML/text；记录真正 HTTP 观察。未支持的 PDF/压缩包只保存元数据/能力缺口，不假装已分析正文。

用户粘贴的来源 URL 是声明，不等于网络验证。下载许可、外联目的地和信息归属是三件不同的事。
用户上传/导入只写 exposure 观察与线索，不自动创建 assets、org_entities 或 asset_ownership_decisions。
通用文库/网盘的关键词搜索、登录、提取码解锁、目录枚举均不在首期。未来接平台时必须指定一个真实来源、明确接口、能力/凭据和样本验证，不能交付空的“支持全部网盘”列表。
售卖/暗网方向首期仅支持用户提供的脱敏报告或合法可用公开情报 feed 的明确 adapter；不部署 Tor、不购买数据、不登录地下论坛、不下载整库。缺来源时显示 unsupported/unconfigured。
情报标注 `claim_only`、`sample_supported`、`human_verified`；文章说集团被泄漏只能是 claim，人工确认需要可核验样本/单位关联与理由。

## 11. 两类出站边界，均不依赖 NEST

自有站点继续使用原项目 ExposureScope 和精确 origin/method/port 授权；目录/单位推断不增加 target grant。
第三方代码/索引/指定文档采用独立来源外联配置（ExternalSourceScope，设计名）：批准 providerHost/path/method、可读资源类型、credentialRef、项目用途、有效期与额度。不是将 GitHub 或文库登记为集团自有资产。
复用现有公开索引 HTTP 传输的防护与计量语义；需要扩展时只改 exposure 专属模块，不复制 NEST Broker。
所有外联在发送前核项目 active、计划版本、当前来源/目标授权、取消标记、剩余额度；DNS/重定向每跳检查，拒绝 userinfo、危险 scheme、未批准 host、私网/loopback/metadata 地址及未经允许的端口。
私网测试仅在隔离 fixture 的明确开关下进行，不能把测试模式存成生产默认。分离 target grant 与 provider 外联规则，GitHub 凭据永不发送给目标站点。
网络响应晚到可以保存原已发生的脱敏观察与支出；撤权后不发新请求、不基于旧授权重派、不把收到的旧响应自动确认为当前有效泄漏。
配置/授权变化导致旧计划不能继续：显示明确冲突与阶段，旧历史保留；只读历史不授予新读取能力。

## 12. 观察、线索、事件、传播位置的独立合同

继续使用现有 Observation/Finding/Version/Review/Remediation 对象；新增事件关系采用 exposure 自有附加表，不改写原观察。

| 对象 | 必须表达 | 不应混入 |
| --- | --- | --- |
| Observation | 本轮本来源实际看见什么/何时/如何获得，hash/规则/脱敏版本 | 未观察的正文、单位或泄漏结论 |
| Finding | 基于观察的具体候选信息、理由/缺口/反证 | 自动授权或无依据 confirmed |
| Incident（新增设计） | 同一泄漏/信息事件的人工可复核主体，原始版本与理由 | “证书域名数”或每条 URL 都当泄漏 |
| Location（新增设计） | 事件在哪个仓库/文件/分享/镜像/历史索引出现 | 单纯外链就当内容可达 |
| EntityLink（新增设计） | 事件涉及哪个已存在实体、关联等级与证据 | 修改全局资产归属或自动创建实体 |
| Coverage | 单位/信息方向/来源/查询及本轮实际执行范围 | 空结果推导安全 |

新增 DTO 的必要键：schemaVersion、projectId、runId/queryId、observationRefs、findingIds、stableKey、revision、rule/policy/configVersion、directory/bindingVersion、collectionMode、evidenceGrade。
Incident review 至少区分 `candidate`、`reviewing`、`verified`、`dismissed`、`excluded_public`；与旧 finding.status 的 new/reviewing/confirmed/dismissed 建立明确映射，不私改历史枚举。
事件涉及单位可为 `confirmed_link`、`candidate_link`、`unknown`、`conflict`。只有人工证据充分或权威确认边可以进入 confirmed_link。
每个复核/合并/拆分/整改动作带 projectId、expectedRevision、operationId、actor、reason、原引用；同 operationId 同 payload 重放返回原结果，异 payload 拒绝；CAS 与写入在同一事务。
不要把本期规则输出塞入 existing exposure_analysis_cache 的“模型已裁决”记录；规则缓存与未来模型缓存分开标实现来源。

## 13. 去重、版本与跨单位统计

- 同内容可生成合并建议；相同标题/文件名/域名或脱敏后的相同片段不能自动合并事件。
- 精确内容 hash 必须来自同一版本的规范化完整内容；截断响应标 partial，不用于声称全文相同。脱敏前指纹是否可留须按既有数据保护策略处理，不保存原凭据。
- GitHub 的 providerObjectId/repoId/commit/blob/path 可作版本身份；跨站事件需要证据链。同一原文件的转载先建立 same_document 候选关系，人工或严格证据确认后合并。
- URL 规范化保留有意义的路径/查询差异；fragment/default-port 等按版本化规则处理。下载参数、版本号或租户参数不能无条件删除。
- 新内容、新规则、新分类、单位冲突均产生版本/待复核标记；既有人工 verdict 与原证据保留，不静默改历史。
- 证书/domain/content 技术聚类继续可用，但不会自动继承其他成员的风险结论；成员必须有自己的观察或明确关联证据。
- 事件多位置时集团计一个稳定 incidentId，分别展示传播位置；跨省事件在各省列“涉及”，不能相加为集团唯一事件总数。
- 不跨项目自动复用敏感正文/授权。允许匹配 hash 提示可能相关，也必须重新验证项目准入、可见性和当前读取许可。

PSL 的算法/数据版本要写入聚合依据；优先通过共享只读边界接口复用已有解析。官方依据：[Public Suffix List](https://publicsuffix.org/list/)。PSL 仅提供域名边界，不提供企业归属证明。

## 14. 覆盖矩阵与性能

矩阵轴：单位 × 信息类别 × 来源能力 × 本轮/时间窗口，点击可见原查询、分页进度、观察和缺口。
至少显示：未配置、未实施、待授权、计划中、已执行无命中、存在候选、存在已核实事件、部分执行、失败、取消/中断。
100% 仅表示选定配置下计划执行完，不表示全互联网已排查或资产安全；证书发现不算客户信息排查，导入文档不算文库平台已检索。
每个分母要说明：资产、唯一 origin、单位、查询、页还是对象。展示 `planned/executed/uncovered` 与失败原因，不能将失败从分母排除后变绿。
声明已支持的方向，至少有真实 adapter 或清楚的导入能力；未支持方向仍可展示，但标未实施，不生成空的绿色卡片。

几万资产不能在前端拉全量 URL：后端稳定 keyset 分页、筛选条件绑定 cursor、明确总数/截断；UI 默认单位/事件/来源聚合，URL 放在组内详情。
每项目/来源窗口使用分块计划与有界队列；同一 query 去重，实体关系保留。写事务短，网络/OCR/文件解析不在 SQLite 写锁中。
配置 concurrency 必须真的限制工作项/请求并发；当前若实际串行，就如实显示并修正合同，不用线程存在证明配置生效。不要移植 Agent 并行调度器。
4000/20000/100000 临时资产档记录 RSS 峰值、库与证据大小、建计划时间、第一页/后续页延迟、取消响应及恢复重读量；相同硬件/SQLite 同步模式，不能降规模或关同步换数字。
初始产品验收目标：本机热态第一页 ≤2 秒、后续页 ≤1 秒；冷态、RSS、全量建计划仅先报告实际值，G0 冻结设备与阈值后才能称通过。取消后下一次发送前生效，UI 正常轮询/事件展示目标 ≤2 秒；已在飞请求另行标注。

## 15. UI、实时状态、证据与导出

暴露面页面建议布局：范围/单位摘要 → 覆盖矩阵 → 去重事件队列 → 传播位置/观察 → 复核/整改/复测；保留现有资产明细入口，不复制归属设置页面。
显示本轮 stage、当前来源、已完成/总查询与对象数、实际请求/字节、最新持久化时间、缺口/错误；未知总量不显示假百分比。
实时展示来自 exposure 持久状态与现有页面轮询/事件能力；不改全局操作日志模块。轮询须单飞、绑定 projectId/runId，项目切换/卸载取消；晚到旧响应不能覆盖新项目、草稿或复核结果。
候选页显示“为什么关联本集团/单位”“实际读取还是索引/导入”“证据不足在哪里”；不给用户展示 Agent 实现细节或模型开关。
暴露面 CSV/证据包只改 exposure/export.rs：文件名 `<项目名>_暴露面_<本地导出时间>.csv`，合法文件名与显式时区，必要时唯一后缀；可另导事件/位置/覆盖，必须声明 rowKind。全局资产 CSV 与调查字段不在本任务范围。
导出保留来源、信息类别、事件/位置 id、单位关联等级、复核/整改、版本、采集/索引时间、排除原因与缺口。公式注入防护、脱敏、路径/符号链接校验沿既有规范；别把未配置渠道导成零风险。
复测须有新 run/query/observation 及可比资源/内容依据；404/认证失败/来源故障/未排查不能自动关闭整改。人工关闭带原始理由，旧泄漏事实仍可读。

## 16. 存储、兼容与数据保护

新增表只在 exposure/schema.sql 与 exposure 内的 schema 入口：建议 `exposure_queries`、`exposure_source_configs`、`exposure_incidents`、`exposure_incident_locations`、`exposure_incident_entities`、`exposure_incident_events`、`exposure_coverage_cells`、`exposure_import_batches`。名称/DDL 需先提交合同映射，能复用旧表就不再建。
关联约束必须验证 project/run/observation 相同项目；数据库外键不是跨 scope 校验的替代品。
应用自己的附加迁移需要 schemaVersion、短事务、显式错误和损坏拒绝；`CREATE IF NOT EXISTS` 不能发现旧表缺列，`.ok()` 不能把失败藏成空数据。共享 schema 需要主控装配，不能迁就全表快照测试偷偷回填或删表。
新增配置/来源使用新版本，仅新轮计划消费；旧冻结计划/配置保持原字节/hash/来源语义，不能读取时增加来源、默认字段或重新签名。新 reader 支持明确旧版本，未知未来版本拒绝。
现有 scope_hash/options_hash/rule_version/redaction_version 和新版目录/查询版本都可追溯。缓存 key 带项目、输入、策略和授权版本；撤权/变更不触发新出站，旧缓存可读性按项目权限核验。
证据只保存必要脱敏内容；日志/导出不含 API token、cookie、原密码、完整个人敏感资料。受控读取的响应也不允许直接执行其脚本或文档宏。
不直接迁移/清理 `/Users/swyiic/oviraptor/oviraptor.sqlite3` 或真实 CAS。所有开发/迁移/规模测试使用独立临时库；需要用户数据升级时由主控先盘点和备份，再做可恢复的精确迁移。

## 17. 分阶段交付与退出条件

| 阶段 | 工作范围 | 必须能实际验证的终点 |
| --- | --- | --- |
| G0 基线/边界 | 盘点当前文件、注册命令、真实来源、目录投影、配置并发与错误、限制说明 | 一份实测能力矩阵和共享依赖表；缺口不写已完成，无越界源码改动 |
| G1 一省/一单位现有来源闭环 | 只读单位投影/未知兜底、分类/招标排除、已有来源证据与风险口径 | 实际授权站点→观察→线索→复核→整改→新观察复测；单位未知/专业公司不误分省，旧历史可读 |
| G2 GitHub | 公开元数据/代码命中、受控读取、分页/额度/错误、未配置状态 | 同一路径 localhost 正负向；有凭据时另列一次真实来源验证，没凭据不伪报外部完成 |
| G3 文档/情报入口 | 文本/JSON/CSV 导入、指定公开 HTML/text URL、等级/出处 | 实际导入与 HTTP 读取分别可追溯；未支持 PDF/文库/暗网明确缺口，公开材料可排除 |
| G4 事件/覆盖/规模 | 事件与位置、跨单位统计、实时 UI、分页、导出与恢复规模 | 多位置去重、多省涉及不重复集团总数、几万条后端聚合可用、缺口不变绿、导出可复核 |

每阶段先完成可编译的模块改动与自有测试，再交付主控集成清单；共享合同未就绪时不要在页面暴露必然失败的按钮。
阶段通过后继续下一阶段，不为常规可逆实施反复求确认；需要真实凭据/新平台能力时保留依赖，完成其余独立项。G4 后停止本专项，不自行启动模型/Agent 或 Master 改造。
G0 要复核此前 P1：设置保存 CAS 的同事务并发、历史窗口 truncated、schema 错误传播、旧无 project/revision 复核调用。不能仅复制旧待办断言它们仍有缺陷。
不以代码行数、接口数量、绿色数量或设计章节数量折算产品完成度。

## 18. 必须保留的测试矩阵

| 编号 | 必须验证的行为 |
| --- | --- |
| T01–T03 目录 | 省公司/专业子公司/城市权威父子关系分开；弱标签与候选组不当确认；目录/项目绑定变更后旧计划不能静默换范围 |
| T04–T06 授权 | 未授权资产不发请求；跨项目引用拒绝且零写；单位绑定/外链发现不创建授权 |
| T07–T09 正常公开 | 招标公告与转载排除且可审计；公开领导/开源/vendor 排除；混合/来源冲突保持待分类 |
| T10–T12 来源 | 真实请求解析/分页与计划一致；缺凭据/401/403/429/422/503 分别展示；空结果与失败/未配置不混同 |
| T13–T15 范围 | API 响应恶意 URL/跨 host 重定向不泄 token；DNS 私网/metadata/端口拒绝；外部平台不冒充集团资产 |
| T16–T18 额度 | 两个实际并发连接/worker 争抢最后额度只能合法消费；失败/重试计实际尝试；恢复不重置已用请求/字节 |
| T19–T21 观察 | snippet/正文/索引/导入分列；截断不称全文相同；原观察、规则/脱敏版本与晚到响应保留 |
| T22–T24 事件 | 相同文件多位置合并有依据；同标题不同内容不合并；跨省涉及只计集团唯一事件一次 |
| T25–T27 复核 | expectedRevision/CAS 真实并发只有一方成功；同 operationId 异 payload 拒绝；内容/规则变化保留旧 verdict 并提示复核 |
| T28–T30 复测 | 新观察关联到原整改；来源失败/未看见不自动关闭；跨项目/撤权/旧版本拒绝 |
| T31–T33 UI | 项目切换/晚到响应/轮询取消；组内分页不拉全量；待配置/失败/缺口保持真实状态和分母 |
| T34–T36 导出 | 项目+导出时间命名；公式/凭据/路径防护；来源、单位、排除、版本与风险总数可对账 |
| T37–T39 崩溃 | claim 前/网络后保存前/保存后 finish 前实际 SIGKILL；已收/未知响应不冒充零支出重放；同项目启动/恢复不产生双活任务 |
| T40–T42 规模/兼容 | 4000/20000/100000 三档实测；旧 schema/冻结计划原字节与历史读取；未知版本/损坏表不补账、不假空结果 |

这些是验收场景，不要求机械生成 42 个镜像实现的单测。复用有意义的现有案例，缺失场景补实际入口/两个连接/网络或故障证据，不能只检查源码含某个字符串。
导入数据、localhost 模型无关 fixture 和第三方实测各自标证据等级；fixture 通过不能标成真实 GitHub 已验收。
任何失败记录原日志、定位、修复与复验；不得 ignore/删失败测试、提高原预算或放松授权换绿。

## 19. 构建、协调与提交报告

先跑自有前端用例：`npm run test:asset-exposure`；新增脚本直接 `node --test --test-concurrency=1 <白名单脚本>`，由主控之后修改共享 package script。
Rust 测试先列出实际模块测试名再选择 filter，不凭空构造命令；Cargo 与主控约定时段，共用 checkout 不同时争抢构建/切换编译参数，不终止他人 Cargo。
主控集成后验证：前端 build、应用 build、主库 test 编译、暴露面定向回归和共享接口回归；只有新共享变化涉及的 Master 回归由主控调度。本期不要求为文档变更重跑整库。
测试之前冻结本次输入 SHA 与文件集合，保存二进制/命令/退出码/日志；测试中其他模块改变时如实说明快照，不引用旧二进制冒充最终源码。

每阶段交付 `docs/ASSET_EXPOSURE_GROK_G<n>_DELIVERY_2026-10-08.md`，固定内容：

1. 真正新增的用户行为、实际生产调用链和仍缺的步骤。
2. 修改/新增文件与 SHA、白名单检查、共享依赖及最小装配请求。
3. 来源能力矩阵：实现/注册/可达/凭据/实际外部验证，不能将这些列折成“已接入”。
4. 所有测试命令/结果/日志、独立用例数与重复复验数、失败与未运行项。
5. 临时库规模、性能及取消/恢复数据；真实库未修改的范围声明。
6. 一段明确四类证据：真实功能、未接入骨架、只读诊断、测试证据。
7. 下一阶段任务、未完成来源/能力与依赖，不自动提交推送。

## 20. 可选 Agent：只预留文档接口，本期不实现

未来可提供 `ExposureAnalysisPort`，只接收已脱敏的不可变观察、单位候选与版本；返回候选信息分类、关联建议、缺口及 observationRefs。
输出不含目标/HTTP 工具调用、授权、自动 confirmed、资产归属写入或直接整改关闭。它是语义辅助，不能替代适配器真实取证。
Rust 必须校验所有引用存在/同项目、版本未变、类别允许、没有越权字段；失败保持原规则结果和待人工复核，不能自动提高置信度。
模型开关、实现、预算、调用、秘密配置及 runtime adapter 均不在本任务；只写 `docs/ASSET_EXPOSURE_GROK_FUTURE_ANALYSIS_CONTRACT_2026-10-08.md` 即可，不新增 not_ready 按钮或虚假的模型功能。
Master 稳定后由主控决定是否接统一运行时或其他已审查服务；Grok 不能自行创建一套 Exposure Agent、模型账本或 NEST 旁路。

## 21. 阅读顺序与当前代码锚点

1. 本规范；此前产品设计 `docs/NEST_GROUP_ASSET_EXPOSURE_PRODUCT_DESIGN_2026-10-08.md`（范围细节延续，旧现状需重验）。
2. `commands/exposure.rs` 的生产开始/恢复、`exposure/orchestrator.rs`、`state.rs`、`settings.rs`、`scope.rs`、`work_items.rs`。
3. `adapters/http.rs` 与三个现有适配器、`rules.rs`、`redaction.rs`、`fingerprint.rs`。
4. `observations.rs`、`remediation.rs`、`recovery.rs`、`clusters.rs`、`cluster_repository.rs`、`export.rs`、`schema.sql`。
5. `src/features/assets/exposure/**` 和现有七组前端用例。
6. 只读核对 O0/O1/O2 交付与当前 ownership 代码，不在这些文件实施修改。

与旧文档冲突时：用户直接指令优先；本规范控制本专项实施边界；已经冻结的历史数据/授权合同不可被新规范追溯改写。疑义记录为具体字段/文件/行为差异，不能用“为了完整”扩大任务到 Master。
