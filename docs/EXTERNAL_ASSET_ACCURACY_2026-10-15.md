# 外部资产抓取准确性：差距与实施记录

> 顺序已更新：用户要求先完善设计再继续源码。当前业务WIP已暂停并保留；以[完整设计与冻结状态](EXTERNAL_ASSET_ACCURACY_AND_PROJECT_ANALYSIS_DESIGN_2026-10-15.md)为后续评审依据。以下为早期差距和原实施计划，不代表已经完成修复。

基线：`3102693abc135f4a2b986caca851bbc935378f0b`，tree `51023994975002ef5330b8a59891441ac7a3754c`。
独立分支：`refactor/external-asset-accuracy-20261015`。本轮只处理外部资产收集准确性；不接内部台账、不新增采集器/通用 Agent、不扩展漏洞扫描，不合并 main。
已交付 DMG 固定原基线，后续改造不追写其构建记录。

## 目标与现状

实际生产链为 `assetApi.startJob → jobs::start_job → execute_job → collect_fofa → candidates.csv 导入 → refine → 按配置 probe → optimize → 再导入`。
`asset_fofa` 是账号/Key 预检，不是另一套采集器。现有原 runId 恢复、查询/凭据/字段/分页绑定、原子页面检查点、内容哈希、错误分类和脱敏可复用。

| 目标 | 已有能力 | 基线中已核实的缺口 | 本轮方向 |
| --- | --- | --- | --- |
| 完整性真实 | complete/coverage_complete、取消/错误、分页上限；mark_missing 有完整性门禁 | native_fofa 的缺失/非数字 size 回退当页数，矛盾短页直接停止；汇总只看 limited，可能误报覆盖完整并生成 missing | 分离传输成功与覆盖证明；未知总数、矛盾短页、非法行、分页达限均保守报告，禁止错误 missing |
| 身份正确 | endpoint 区分 HTTP/HTTPS、端口、www/裸域、路径和查询；旧 key 原位复用并保留人工状态 | jobs/asset_identity::endpoint_url 对无 scheme 的非 HTTP 协议按端口猜 scheme，可能合并不同服务 | 保留已知协议身份；推测访问方案不成为实体等价依据；不批量删除或盲并历史资产 |
| 观测时间真实 | FOFA lastupdatetime、probe_checked_at、原 probe journal 已存在 | import_csv 把重放时刻写入 last_seen/last_alive；旧 probe 可搭配新 IP/协议及新“最后发现”显示 | 原观测时间与本地导入/任务时间分开；旧观测不得覆盖新探测；历史结果明确标注 |
| 归属可解释 | 候选标签、人工有效性、归属和授权互相独立；弱线索不授予权限 | host_equals_rule 去 www，精确规则并不精确；来源 domain 优先于实际 endpoint 时可误判根域证据 | 精确 host 保留边界，以真实端点评价归属；冲突降级，保留人工决定与授权门禁 |
| 可回溯且有界 | 保存原始页面 payload、fields、哈希、query_log、来源列表；单页有上限 | 缺少明确 receivedAt/规则版本/保留预算；重复缓存和长期跨日恢复的时效未说明 | 在既有收据上补来源/时间/版本和有界保留，不保存凭据，不另造无限日志 |
| UI 真实简洁 | 任务终态/错误/恢复入口、资产详情、人工/归属状态 | completed 100% 掩盖覆盖不完整；没有 typed 收集摘要；默认旧 probe 结果配新发现时间；文案无条件承诺自动探测 | 保留任务执行终态，增加独立覆盖摘要；历史探测时间按需展开；展示实际配置阶段 |

依据：`jobs/native_fofa.rs` 的 `collect_fofa`（基线约817、872、1027行），`jobs.rs` 的 `import_csv`/`mark_missing`/`execute_job`（约608、737、1017行），`jobs/asset_identity.rs` 的 `endpoint_url`，`commands/asset_ownership.rs` 的 `normalize_ownership_domain`/`host_equals_rule`/归属分析，以及 `features/assets/workspace/AssetsPluginWorkspace.vue`、`AssetWorkspace.vue`、`QueryPanel.vue`。
设计约束继承 `NEST_GROUP_ASSET_EXPOSURE_PRODUCT_DESIGN_2026-10-08.md` 的候选、归属、执行资格分离，以及 `NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md` 的精确主机、弱线索和人工决定保护。内部台账及更广的泄漏渠道不在本轮范围。

## 分批实施及验证约束

1. 收集端：覆盖证明、输入有效性、分页/额度/失败分类和原始页面收据；使用离线合成 FOFA fixture 验证，不请求真实目标。
2. 资产端：协议身份、精确归属规则、观测时间和安全历史兼容；SQLite 真实导入及时间倒序/重放回归，不重置人工状态。
3. 结果与界面：Assets 专属 typed 摘要，从后端持久结果读取，不能解析错误文案猜完成度；任务执行终态与覆盖完整性分别显示。
4. 集成：前端构建、all-targets check、必要原生定向/完整回归和 Node 守卫；记录实际 commit、输入及失败。只运行本地临时库和本机 fixture，不接用户数据库或外部探测。

此提交仅记录已核实差距与计划，尚未宣称任一缺口已修复。后续每阶段结果附于本文，未执行的检查不计通过。
