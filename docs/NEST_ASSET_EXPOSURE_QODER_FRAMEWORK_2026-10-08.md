# Qoder 专属任务书：公开暴露面框架与生产闭环

日期：2026-10-08。最新用户纠正优先于此前“主控接管整个 Asset”的表述：主控只负责用户点名的资产导出、Strix 环境/配置清理、操作日志，以及缓存清理和版本交付。Qoder只负责公开暴露面，禁止修改其他功能。

## 1. 排他边界：按文件白名单工作

代码根目录：/Users/swyiic/Desktop/Rust/Oviraptor。工作树已有大量未提交改动，先保存自有文件开工前SHA与差异，不以Git HEAD覆盖现有代码。

可修改：
- src-tauri/src/commands/exposure.rs：仅暴露面命令门面。
- src-tauri/src/commands/exposure/**：暴露面合同、仓储、迁移、采集、观察、聚类、缓存、恢复与自有测试。
- src/features/assets/exposure/**：暴露面专属接口、状态机、类型、页面子组件。
- src/components/ExposureSurface.vue、src/exposure-surface.css。
- tools/test_asset_exposure_*.cjs。
- docs/NEST_ASSET_EXPOSURE_QODER_FRAMEWORK_2026-10-08.md：只追加事实交付记录；新建独立的暴露面交付报告亦可。

上述名单之外全部禁止修改，特别包括：lib.rs、commands.rs、db.rs、db_schema.rs、db_initialize_columns.rs、models*.rs、src/types.ts、src/api.ts、src/features/assets/api.ts、App.vue、AssetWorkspace.vue、AssetOwnershipGate.vue、ConfigDialog.vue、环境检测、asset_export.rs、jobs.rs、asset_logs、日志composable、agent_runtime/**、multi_agent/**、native_*、package*.json、Cargo*.toml/lock、tauri配置、Master/progress/交接。

旧交付允许追加 src/features/assets/api.ts 的规则在本任务书后撤销。需要新IPC时在暴露面白名单里定义实现与独立TS包装，输出待注册命令、参数/返回值和对应测试；主控串行注册及装配。需要全局schema或类型修改也只能提交依赖说明，禁止自行修改。禁止修改共享白名单以扩大权限。

禁止全库格式化、reset/clean/restore、批量覆盖、自动提交/推送、清理用户数据库/缓存/资产、调整其他代理测试进程、结束他人cargo任务。重型Rust门禁由主控排队，Qoder不得为抢锁停掉现有测试。原交付主动中止他人全量测试的做法不得重复。

## 2. 当前状态与证据等级

基线来源：Q0–Q6交付记录、当前命令/页面源码。历史交付声称Rust62/0、前端24/0；这些是交付时测试证据，不是当前真实平台或公网验收。

主控本轮已注册九个IPC：page_exposure_findings、page_exposure_runs、list_exposure_source_results_for_project、read_exposure_finding、exposure_scope_summary、exposure_known_limits、resume_exposure_scan、page_exposure_pending_discoveries、read_exposure_evidence。新增test:asset-exposure脚本，当前前端暴露面24项加日志9项通过，Rust cargo check通过。此集成由主控负责，不意味着所有暴露面功能完成。

已经存在：页长/游标合同、项目复验、原子启动与冻结计划、工作项租约、逐项终态、CT/归档解析、观察与finding分离、只读证据包、CSV脱敏、后端簇与缓存合同。

需要优先核实和补齐：
- exposure_scan_settings() 仍返回默认配置，缺用户可编辑来源、额度与出口设置闭环。
- cluster_repository::save 尚无生产采集调用，簇由测试写入；页面缺簇视图。
- 规模证据只到20,000计划物化及4,000终态，未达100,000目标；真实磁盘证据/重启恢复不足。
- 新IPC未做安装App端到端验收。
- Q7模型裁决与整改复测尚未实现；Native ExternalSurface角色/共享预算尚未就绪。

每次交付逐条标注：生产可达功能 / 未接入骨架 / 只读诊断 / 测试证据。localhost不等于第三方，临时SQLite不等于真实用户库；不得用“Q6完成”掩盖未接入链。

## 3. 框架层次与数据流

```text
暴露面页面与专属API
 → 输入合同（项目、来源、配置版本、额度、范围）
 → 暴露面授权/出口校验 + 冻结计划
 → 原子run建立/复用 + 持久工作项
 → claim/version/lease
 → CT、归档、被动Web适配器（事务外网络）
 → 请求/字节用量记录 + immutable observation
 → finding归并 / pending discovery
 → 内容簇、域名簇与带来源的缓存
 → 逐工作项/逐来源终态及未覆盖缺口
 → 分页展示、证据、人工复核、CSV、恢复与复测关联
```

页面不得直接绕过门面访问采集器；网络请求不能发生在长写事务内。出口逐跳校验，取消/撤权/期限/归属变更必须在下一外部效应前重验。CT/归档发现的新域只进入待审核队列，不能自动变成已授权资产或扫描目标。

边界合同：只管理暴露面run/work item/observation/finding/cluster/cache等专有对象；不得写Native Root、模型账单、Broker grant、Sentinel任务、资产归属决定。需要发送扫描时只提出已有主控入口的集成依赖，不自行制造授权。

## 4. 分阶段实施与验收

### E1：安装入口与范围合同复核（先消除接口假空白）

- 保留已注册IPC和旧接口错误提示；未知命令、权限拒绝、跨项目、损坏游标必须展示真实错误，不能显示“没有线索”。
- 同一project/run/filter下汇总和列表使用相同谓词；总数、截断、hasMore、来源状态可解释。
- 覆盖A→B→A切换、在途迟到、刷新失败保留旧快照且标stale、卸载/隐藏停止轮询。
- 验收：真实SFC+IPC模拟正反测试；主控另做安装App临时数据验收，不污染用户数据。

### E2：采集配置的实际闭环

- 增加暴露面专属配置合同与持久化：启用来源、每轮请求/字节/工作项额度、并发、超时、CT/归档页长等；明确单位、有限上界及无效值错误。
- 配置关联project和版本；启动把完整值冻结到run，恢复验证原版本和已花额度。修改当前配置不能偷改旧run或令已花额度归零。
- 不默认放开私网/localhost出口，不把“用户选来源”当成跨scope授权。测试localhost出口仅显式测试配置允许。
- 白名单内增加设置读写命令与页面，主控负责注册。不得进入全局ConfigDialog或共享config_profiles。
- 验收：读取→编辑→保存→新run冻结→消费→同原预算恢复；未知字段/非法枚举/0与null语义、跨项目、撤权与过期负向。

### E3：聚类/缓存接入真实采集与页面

- 在观察落库后、工作项终态前接入生产簇持久化，保证崩溃重试幂等；失败不得假称已继承覆盖。
- 簇代表选择有确定规则，记录观察、run、规则版本、实际观测与继承依据；URL path/query不能全量小写归并。
- 页面展示代表、成员、直接证据/继承证据、缺口和缓存时间；筛选分页不得全量拉进前端。
- 缓存键绑定项目/scope、来源、规范化URL、配置/规则版本、期限；授权变化与撤权优先于命中缓存。
- 验收：实际orchestrator→适配器→observation→cluster→IPC→SFC；同内容不同scope不得串用，过期/撤权缓存不授权请求。

### E4：规模、故障和恢复

- 4,000/20,000/100,000规模分别记录计划构建耗时、数据库大小、内存、分页时延；不使用真实资产数据，测试种子可重现。
- 总量不依赖前200/500条窗口；有硬上限就返回截断与未覆盖原因，不能报完整完成。
- 覆盖同项目并发启动、不同项目隔离、租约过期/迟到worker、取消/撤权后零新请求、进程死亡、响应已收到但终态未写、恢复预算不重置。
- 故障点：plan提交前后、claim后、网络返回后、observation后、cluster前后、finish提交/ACK丢失。持久前后快照和请求计数需证明不重复花费/不串scope。
- 真实磁盘证据不可跟随符号链接越界，包完整性与来源可核验；不得将敏感凭据原文写入页面、日志、CSV。

### E5：复核与整改复测（先确定性部分）

- 保留观察历史；人工复核更新有版本冲突检查、操作者/时间/原因与引用，不覆盖旧证据。
- 整改记录绑定原finding/observation/run；复测创建新暴露面run并重新验权，不把原观察修改成“已消失”。
- 区分已确认暴露、待复核、来源不可达、未覆盖、规则命中与未知；无证据不能定性漏洞。
- CSV与页面一致，公式注入转义、凭据脱敏；这是暴露面finding导出，不允许修改资产导出asset_export.rs。
- 验收：复核→整改→新轮复测→新证据关联→历史可读，跨项目/撤权/旧版本/丢失证据拒绝。

### E6：可选模型裁决依赖，禁止独立复制执行框架

只交付候选裁决输入、结构化输出、原证据引用、费用和授权需求清单。Native ExternalSurface/Broker未就绪前，保持明确not_ready，不伪造模型结果、Root或预算。模型不能修改资产归属、自动授权域名、跳过确定性规则或把推测当观察。主控后续集成共享模型/预算后，再做实际模型调用正向、未知费用/过期/撤权/取消负向。未就绪时继续E1–E5独立工作。

## 5. 文件规划与依赖交付

建议在现有exposure内核增量扩展，避免另起平行库：配置contract/settings repository；clusters production hook与分页repository；verification/remediation关系；规模与故障测试。前端子组件放src/features/assets/exposure，不移动其他资产页面。

每新增命令提交：名称、Rust函数位置、TS参数/返回、只读/写入、权限、错误码、全局装配位置建议、已跑测试。需要共享schema时列出表名、外键、迁移/回滚与保留历史规则，交主控处理。禁止把依赖“先临时绕过”写成实现。

## 6. 门禁与交付格式

- 仅自有文件rustfmt与git diff --check；前端node --test --test-concurrency=1 tools/test_asset_exposure_*.cjs。
- vue-tsc/build只作兼容检查，不修改全局脚本或依赖。
- Rust过滤测试须先向主控登记队列；不结束其他进程，不在同树无限叠加重型cargo。
- 每阶段提交MD：文件清单及开工/交付SHA、生产调用路径、测试命令/退出码/准确计数、失败与未做范围、外部依赖、可复现验收步骤。
- 不删/ignore测试，不弱化scope/撤权/费用守卫换绿，不声明整Master完成，不自动发布安装包。

## 7. 可以直接发给Qoder的指令

请按本文件在现有Q0–Q6基础上继续E1→E5，只修改第1节白名单。首先复核本轮九个IPC已由主控注册，补配置冻结恢复闭环和生产簇写入/页面。禁止触碰其他任何功能及共享文件；缺注册/schema/类型/模型预算支持就列依赖交主控。分阶段交付真实功能、骨架、只读诊断与测试证据，不用局部绿称框架完成，不运行用户真实采集，不结束其他代理进程。

## 8. 事实交付记录（2026-10-08，Qoder）

E1–E5 已在白名单内交付完毕，细节、证据等级、失败与外部依赖见 `docs/NEST_ASSET_EXPOSURE_E1_E5_DELIVERY_2026-10-08.md`。要点：本轮重跑门禁为 rustfmt 0、`cargo test commands::exposure` 94/0、clippy 自有文件 0 诊断、`node --test tools/test_asset_exposure_*.cjs` 59/0、vue-tsc 0、`npm run build` 通过、`git diff --check` 0。九个已注册 IPC 复核通过（`src-tauri/src/lib.rs:370-378`），共享文件零改动。E2 的冻结配置改动暴露并修掉了两个既有测试的真实回归（`tests_q3` 恢复谓词、`tests_q6` 规模额度），另修掉 `page_clusters` 把游标算进过滤总数的实现错误。待主控处理：E2/E3/E4/E5 共 14 个新命令注册、`package.json` 的 `test:asset-exposure` 只列 3 个文件（漏 4 个文件 34 项）、旧 `review_exposure_finding` 无 project/revision 字段需决定切换或只读兼容。缓存复用消费方与安装 App 端到端验收仍未接入，E6 保持 not_ready。
