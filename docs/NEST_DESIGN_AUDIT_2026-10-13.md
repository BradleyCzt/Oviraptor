# Oviraptor 设计层深度审计（2026-10-13，证据复核版）

本文保留原设计审计的范围，并纠正把局部源码搜索结果扩大为运行时结论的部分。
本文本身不改变运行时行为，也不是发布验收记录。

- 原稿来源：[BradleyCzt/Oviraptor @ 16d3782 的同名文档](https://github.com/BradleyCzt/Oviraptor/blob/16d3782ef9889d64197ae9654907f548784d8817/docs/NEST_DESIGN_AUDIT_2026-10-13.md)
- 被审代码基线：`8fddee2`（`Merge current Nest checkpoint with GitHub main`）
- 本次复核：读取上述基线及相关调用链，复核指定源码事实和有限的文件计数；原作者的 macOS 运行及编译时间/零警告报告未独立重现
- 当前修复及实际验证见 §9；下文的基线发现不表示修复后仍存在
- 原稿引用的 10-12 改进审计未包含在本基线中，不把转述结果冒充独立验证

## §0 结论与证据等级

**撤回原稿“单写终态只有注释、28 处生产路径绕过、构成 P0”的结论。**
`store.rs` 的“仅 reducer 写终态”注释与现有职责划分不符，但生产路径存在事务、
状态校验、fencing、收据和后置条件。`reducer::commit` 仅在测试中编译，不能据此
推断生产没有终态保护。原稿举出的 child 完成路径会拒绝已完成 run；gap/review
路径是受约束的原收据交付恢复，不能仅凭 `failed -> completed` 判为任意覆盖。

本次保留的重点：

1. **安全缺陷（基线）**：浏览器登录会话校验请求携带 Cookie，却关闭 TLS 证书校验（§4.2）
2. **测试健壮性缺陷（基线）**：macOS 专用测试 fixture 的析构函数可能二次 panic；这是测试代码缺陷，不是生产运行时 panic 的证明（§5.3）
3. **存储风险**：配置中的模型密钥、Worker 节点访问令牌、浏览器认证材料以明文序列化保存；不能把所有名为 `api_key` 或 `token` 的列都算作提供商凭据（§4.1）
4. **维护性问题**：终态/恢复契约的文档落后、部分命令共用 include 作用域、schema 初始化分散，需要准确的职责入口和针对性测试（§1–§3）

原稿中的完整测试运行约 45 分钟后 SIGABRT，以及 `cargo check --all-targets`
约 30 秒零警告，均为**原作者报告，本次未独立验证**。此前“3066 passed / 318 failed”
也不能据本稿变成可信基线；对 318 个失败的归因仅有抽样，不是逐项确认。

### 计数口径

本次按 `8fddee2` git 树/文本复核了：`src-tauri/src/` 1,593 个 Rust 文件、
`commands.rs` 151 个直接 include 文本位置、`commands/*.rs` 756 个顶层文件、
`docs/*.md` 303 个顶层文档、22 个 `*source_pin*` 文件。这些不是生产路径或缺陷数。
原稿其余行数、表数、调用数与依赖计数保留为历史记录，未独立复算，不用于风险定级。

## §1 终态写入：契约描述需要纠正，未证明 P0 覆盖缺陷

### 1.1 reducer、writer 与测试 helper 的职责不同

`src-tauri/src/agent_runtime/store.rs` 模块注释称终态仅由 reducer 写入。
`reducer.rs` 的 `commit` 确有 `#[cfg(test)]`；纯归约函数与生产持久化入口则仍存在。
例如生产 `commands/agent_terminal_report.rs` 调用 `single_projection::close`，
后者使用 `reducer::backend_exit::reduce_backend_exit` 决定结果，并完成发布。

这说明旧注释需要更新为实际的**归约、授权、记账、持久化**分工。
不能把一个测试 helper 的“first writer wins”注释套用为所有 Root/Child/恢复路径
共有的完整契约，也不能从该 helper 不进 release 推导“不存在生产执行体”。

### 1.2 child 完成不是只有最后一条 UPDATE

`agent_runtime/multi_agent/scheduler/lifecycle.rs` 的最后一条 `UPDATE agent_runs`
确实只显式限制 id/root，但调用链还有下列约束：

- `finish_child` 在 `TransactionBehavior::Immediate` 事务中调用完成逻辑
- 同一事务先校验 coordinator lease，并联结 assignment、run、lane，核对 scope、role、epoch、fencing token
- 前置查询要求 assignment 为 running/waiting_review 且 run 为 running；失败清理另允许 leased/prepared
- 已 completed/terminal 的 run 不满足该查询，在 UPDATE 前返回 `child_finish_binding_conflict`
- 写后检查 assignment、run、attempt、能力撤销、lane 释放、预算差额及协作事件；错误会使事务回滚

因此原稿“这条路径可以覆盖已 completed 子 run”的推导忽略了同一写事务内的
前置状态检查，**不成立**。可以考虑把前置约束进一步显式化以利审阅，但不能把
WHERE 中没有重复所有条件等同于没有守卫。事务内入口也应保留调用者持有原事务的契约。

### 1.3 gap/review 的 failed -> completed 是有意的收据恢复

`commands/multi_agent_gap_recovery.rs` 与 `commands/multi_agent_review_recovery.rs`
明确只交付已保存的原始响应，不重新执行模型，也不恢复撤销的 capability。
其恢复校验包括：

- 原 specialist 收据、角色、assignment/child/root、租约及 fencing 绑定
- frozen request、task/input 或 candidate revision 与原响应的一致性
- failed 路径要求原 `child_failed`、预算已结清、预留归零、无活跃 lane；paused 路径另有状态条件
- 无未撤销能力，按原费用收据检查或结算用量，完成原 attempt
- 外围事务负责交付记录与后置条件，失败时回滚

`gap_receipt_reconciled` / `review_receipt_reconciled` 是这类本地交付完成的来源码。
应把“执行失败后收到/交付原收据”的语义写清楚并测试，不应因来源码不是通用 reducer
生成，就认定它是无条件的历史篡改。此次复核没有证明这些守卫可被绕过。

### 1.4 single_projection 已有结构性保护

`agent_runtime/single_projection.rs` 及其 `writer.rs` 使用私有连接、受限 SQLite
authorizer 和 IMMEDIATE 事务。已有原收据会重新验证；没有原收据的已终态 run 被拒绝。
首次写入要求原非终态、空 terminal_state/finished_at，随后精确重读 run、事件、
快照、财务证据及不可变 projection receipt。

原稿的“353 个 CREATE TRIGGER”仅为历史文本计数，并不能说明这些保护不存在。
Mutex 搜索次数也不能替代对数据库事务与 CAS 的审阅；SQLite 事务是结构性保障，
`busy_timeout` 只是等待策略，两者不能混为一谈。

### 1.5 建议：类型化、收据感知的状态转换契约

**不采用原稿的一刀切 `terminal_state <> ''` 不可变 trigger。**
它会阻断上述合法收据恢复，也不能完整保护 status、terminal_code、reason、finished_at
及相关账本。`TerminalState::Incomplete` 的序列化词汇为 `paused`，进一步说明不能
仅靠某列非空来定义所有生命周期终结。需要同时区分终态发布、暂停/未完成和收据交付。

先明确每类转换的 owner、前置状态、可接受的原始收据、预算结算、撤销和事件后置条件；
沿现有 proof/receipt 类型增量收口，保留正向恢复与拒绝越权的行为测试。
如果增加数据库约束，应基于这一明确契约，并覆盖合法恢复、并发完成、重复收据、
陈旧 fencing、部分写失败，而不是先令合法路径失败再决定如何处理。

原稿“28 处/27 文件”只作为终态文本写入的历史搜索线索保留，不再称为 28 处生产绕过。
需逐一追踪宏/cfg、事务和调用者，才能建立完整写入路径清单。

## §2 命令层 include 作用域：局部耦合真实，范围不应夸大

### 2.1 151 个 include 不等于 756 个文件同一命名空间

基线 `commands.rs` 有 151 个直接 `include!` 文本位置；`commands/*.rs` 的顶层文件
共 756 个。被 include 的顶层项通常共享包含处作用域，因此部分依赖和导入确实不显式。
但被包含文件可定义真实子模块，include 也可能受 cfg(test) 保护；其他文件未必由
`commands.rs` 直接包含。递归目录统计、顶层文件统计和模块作用域是不同指标。

例如 `commands/exposure.rs` 定义 `pub mod exposure` 子模块树；`commands.rs` 本身
还包含 `#[path = ...] mod ...`。同模块内的私有项可以互见，跨子模块仍由 Rust 可见性
规则约束。不能说整个命令层“没有模块系统”或“rustc 不施加任何约束”。

原稿的 139/151 个文件跨文件引用及具体引用次数未提供可复现的符号解析方法，
不能充当精确依赖图。建议以已有模块为参考，按业务边界增量明确接口和最小可见性，
保留宏/cfg、命令注册和调用契约；不要机械改写所有 include 或批量扩大 helper 可见性。

## §3 数据层：分散初始化与迁移可观测性

### 3.1 已有模块入口、版本标记与幂等迁移

schema 分布于 `db_schema.rs`、`db.rs::initialize`、`db_initialize_columns.rs` 和
feature 文件。`db.rs` 有显式初始化顺序；`db_migrate.rs` 以 `app_settings(key,value)`
保存按 key 的迁移版本，`ensure_column` 执行查列、ALTER、重新验证，部分迁移自持事务。
原稿“没有所有者、版本和顺序，无法 diff schema”的表述不准确。

维护成本在于需要跨文件还原初始化、增量列、修复和验证。应先列出每个迁移族的入口、
版本与原子性，测试旧库升级、部分失败重入和备份恢复。`PRAGMA user_version` 不是
正确迁移的必要条件，无 down migration 也不自动构成缺陷。

### 3.2 exposure 按分号切 SQL：兼容性风险会报错，不是静默成功

`commands/exposure/state.rs::ensure_exposure_schema` 对 `EXPOSURE_SCHEMA.split(';')`
逐段执行。带 `BEGIN ... END` 的 trigger 含内部分号，这种拆分不适用。
但除具名容忍项、`already exists` 和 `duplicate column name` 等分支外，错误会
**返回 Err**；不完整 trigger 的语法错误不会自动被当作“已存在”吞掉。

风险应描述为：新增复杂 DDL 时初始化可能失败，且此前成功的语句可能已经落库；
不能描述为必然静默建立了不完整 schema。替换执行方式时还要保留旧库列/索引迁移
顺序和现有告警语义，简单替换为一次 execute_batch 并非已验证方案。

此外，普通 agent schema 经 `db.rs` 的 execute_batch 初始化，**不经过这个 exposure
执行器**。原稿称 agent_runs trigger 必然踩到该拆分器，缺乏调用关系依据。

### 3.3 连接、事务与数据保留

`db::open` 设置 busy timeout、foreign_keys、synchronous，初始化启用 WAL。
多连接竞争值得测量，但 open/execute/transaction 的搜索次数不能证明写入缺少原子性：
事务可由上层持有；也不能由此断言 FULL_MUTEX 无意义。

当前状态、原收据和历史快照有不同保留责任；`_json TEXT` 不代表不可查询，源码使用
`json_extract`。没有 VACUUM 也不能证明没有行级清理或必然无限增长。应逐表检查
真实增长、保留义务和恢复依赖，不能据 grep 设计统一删除方案。

## §4 安全：区分真实凭据、标识符与威胁模型

### 4.1 已确认的明文保存路径

| 数据 | 存储/使用证据 | 结论 |
|---|---|---|
| 模型配置密钥 | `config_profiles.settings_json`；`workspace_config_profiles.rs` 序列化 settings；`db_settings.rs` 处理 modelApiKey/localApiKey/modelProfiles | 已配置的密钥随 JSON 明文保存 |
| 远程 Worker 访问令牌 | `worker_nodes.access_token` | 是访问凭据，应纳入保护范围 |
| 浏览器认证材料 | `browser_auth_sessions.session_json`；`auth_session.rs` 保存 cookies、headers、storage 等 document | 捕获的认证材料按 JSON 明文保存 |

以下原稿例子应撤回“提供商 API 密钥”分类：

- investigation 表的 `api_key` 是被调查 API/端点的业务标识，可用于方法、路径及图节点关联
- ownership 表的 `worker_token` 是本地 worker 所有权/CAS fencing 标识
- `analyzer_container_receipts.owner_token` 是生成后写入容器 label 的所有权 nonce

这并不表示这些标识符不需要完整性保护；只是它们不是 HackerOne/LLM/FOFA 提供商密钥。
不能凭列名把全部 TEXT token 都算作账号凭据泄露。

Unix 上数据库 0600 是有效的访问权限控制，但不是静态加密。可读取该文件或其明文
备份的进程/账户能获得已存认证材料；失窃设备的风险还取决于磁盘加密与解锁状态。
artifact sealing 是独立保护，不应外推为整个数据库已加密。

建议另行设计真实秘密字段的 OS 凭据存储或加密迁移，包含原库/备份、失败回滚、
缺失密钥和跨平台行为验证。本次文档纠正不声称已完成该迁移。

### 4.2 携带真实 Cookie 的校验请求关闭 TLS 验证（基线确认）

基线 `auth_session.rs` 构造捕获会话的 Cookie/可重用 headers，随后使用：

```rust
Client::builder()
    .timeout(Duration::from_secs(20))
    .redirect(reqwest::redirect::Policy::limited(8))
    .danger_accept_invalid_certs(true)
```

忽略无效证书使能够主动介入连接的攻击者可冒充目标并取得凭据，是具体安全缺陷。
Cookie 域筛选不能补偿 TLS 服务端认证缺失；原稿还遗漏筛选允许空 domain 的分支，
所以也不应据局部检查宣称 Cookie 重放范围已全面正确。

基线另有 `commands/native_frontend_recon.rs` 的无效证书开关，及
`jobs/native_probe.rs` 的 `!options.strict_tls` 开关。它们的目标与认证上下文不同，
不能在未审查行为的情况下等同处理。优先确保认证校验客户端保留默认 TLS 验证，
且证书失败不以不安全重试降级。跨主机跳转时 header/Cookie 的实际传播需单独测试。

### 4.3 其余安全观察的范围

request_hash、ordinal 和 SQLite 不可变约束保护应用内一致性，不等于外部锚定的
防篡改证据。离线整库篡改、日志锚定需求需单独威胁建模。

保留现有的容器固定镜像/网络限制、作用域/重定向检查、artifact 路径/符号链接检查、
持久化前脱敏和授权/租约 fail-closed。相关源码观察不能扩大为“所有路径无绕过”。

下列原稿结论仍需定向验证：私网/回环/DNS rebinding 的完整解析路径；每类 Tauri 命令
自身的授权边界；实际锁定依赖的漏洞。目录内无关键字、命令注册数或依赖使用 caret
均不足以证明对应漏洞，lockfile 与版本范围也应区分。

## §5 编排：按现有调用链审查，不从底层文本缺项推断绕过

### 5.1 恢复机制需要一张清晰的契约图

checkpoint、lease expiry、saved receipt、预算 reconciliation 处理的对象和授权
不同。内存队列能够从持久状态重建时，进程退出丢失 Vec 本身不等于丢失恢复能力。
恢复路径较多值得整理，但原稿没有证明其中 14 条可安全删除。

应先明确每条路径的触发条件、原收据来源、是否允许新执行、费用归属和完成证据。
在行为等价和故障注入测试支持下才合并路径，保留合法本地交付与禁止重复花费的语义。

### 5.2 预算在调用方和事务边界强制，未证明无上限花费

`agent_runtime/model/transport.rs` 不直接拥有全部预算逻辑，不代表模型调用不受预算
约束。已读取的真实入口包括：

- `scheduler/budget_reservation.rs` 在调度事务内检查 Root 容量、预留总量上限、fencing，并调用 `budget::model::reserve`
- `specialist/dispatch.rs` 使用 IMMEDIATE 事务，要求运行状态、原预留、有效能力与确定费用状态，持久化调用后再次校验
- `budget/model.rs` 的 reserve/settle 接受事务，校验费用 owner/assignment/attempt；`child_accounting.rs` 参与原用量结算
- `budget/admission.rs` 拒绝费用不确定或 elapsed 已终结时的新工作；财务补记不自动重授执行权限

model admission 的并发信号量确实不等于成本预算，但不是系统唯一门禁。
原稿“214 个裸 SQL”“新调用必定无限花费”未有逐路径或运行证据，撤回已证明绕过的
表述。未来仍应检查每个新模型入口是否持有匹配的执行/费用授权，并保留缺预算、
陈旧 worker、未知结果、重复 receipt 的拒绝测试。

### 5.3 macOS fixture 析构 panic 是测试缺陷，影响范围有限

基线 `tool_supply/sandbox/macos_static_parser.rs` 中的 `Fixture` 位于：

```rust
#[cfg(all(test, target_os = "macos"))]
mod tests {
    // ...
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
```

如果测试正在 unwind，而清理又返回错误，此 unwrap 可再次 panic，导致测试进程 abort。
这是源码可确认的缺陷。它不进入生产构建，也不进入非 macOS 测试构建。

原作者报告某次 macOS `cargo test --lib` 运行约 45 分钟后，先出现 fixture 断言失败，
再出现删除目录 PermissionDenied 和 SIGABRT，没有整次汇总。本次没有独立运行该环境，
保留为作者报告；不能写成所有环境全套测试都无法运行，也不能据此否定过滤运行、
其他平台结果或所有修复的可验证性。

建议清理失败不 panic，并输出足够信息定位遗留 fixture；应测试清理失败期间的 unwind，
并在 macOS 复跑原 fixture。这个修复不能证明最先发生的沙箱断言失败已修复。

原稿对生产线程 `join` 结果忽略的搜索可作为后续调查入口，但 fixture 不能证明
“任意生产线程 panic 后必定留下 running assignment”。也不建议统一添加 catch_unwind
后继续运行：要先定义该边界的回滚、未知费用、资源和租约恢复契约。

### 5.4 状态投影与 IPC

run、assignment、attempt、worker/task 的不同状态需明确原事实、投影和验证 owner，
不能仅因跨表重复就判错。Rust 命令、`lib.rs` 注册、`src/api.ts` 和 `src/types.ts`
需要同步验证；TS 检查不能单独证明跨语言契约一致。原稿具体 wrapper/struct 数未复算。

## §6 过程与测试证据

### 6.1 文档与 source-pin

原稿“255 个文档含未完成等字样”可能包含历史、已解决或否定语句，不是当前任务数，
也不能支持对开发者动机的判断。基线 README 链接的 `docs/architecture.md` 确实缺失；
本次补充当前入口说明，不声称恢复被删除的原文。

source-pin 子串测试会因无语义变化的重构失败，也可能漏掉真实行为错误，但有些用于
保护危险 API/门禁接线或架构边界。应标明各自契约并补行为、故障和拒绝测试；
没有依据推断其唯一动机是审计变绿，也不应统一删除。

### 6.2 unwrap 与 cfg 的统计纠正

原稿把生产 unwrap 修正为约 43 处，但没有可靠的 Rust 条件编译分析。
`ownership_review.rs` 第一个 `#[cfg(test)]` 只修饰一个 helper，之后又有生产函数，
不能把该位置以下全部划为测试段。按文件名排除测试也会漏判内嵌测试与 include。

因此 43 和此前 1184/941 等数字均不能在这里充当准确生产总数；本次不据它们调整
生产崩溃风险等级。应按 item/module/cfg/target 逐处分类，分析可达性与错误前提。

已读的 `commands/tests.rs`、`commands/agent_tests.rs` 内部测试模块有 cfg(test) 门禁，
这支持这些模块不进 release，不能扩大为所有测试代码均已完成 cfg 审计。
`_actual` 也常是实际成本等业务词汇，单凭后缀不能判断新旧实现残留。

## §7 建议优先级与验收边界

| 优先级 | 事项 | 所需证据 |
|---|---|---|
| 优先 | 认证会话客户端恢复严格 TLS，拒绝不安全降级 | 无效证书请求在发出认证数据前失败；有效连接/错误呈现验证 |
| 优先 | macOS 测试 fixture 清理不 panic | 清理失败/unwind 测试；macOS 原 fixture 单独复跑 |
| 优先 | 修正文档与验证报告，补架构入口 | 来源、基线、统计口径和未运行项明确 |
| 后续设计 | 类型化终态/收据恢复契约 | 合法恢复成功、重复/陈旧/越权写被拒绝、预算与事件不变式 |
| 后续设计 | 真实认证秘密的存储迁移 | 跨平台凭据库、旧库/备份、密钥丢失与回滚策略 |
| 增量维护 | schema 入口说明、复杂 DDL 与中断恢复测试 | 旧库升级、部分失败重入、告警与错误传播 |
| 增量维护 | include 边界、IPC 契约和 source-pin 补强 | 小范围迁移与行为等价测试 |

不把大规模恢复路径删除、统一长连接 RunWriter、全面 catch_unwind 或日志外部锚定
列为已证明必要的本次修复。它们需要独立需求、威胁模型和兼容性设计。

## §8 复核入口与可重跑命令

源码行号会随修复变化，以下使用基线 ref 和符号名定位。搜索只提供阅读入口，不证明
匹配全部可达、生产使用或缺陷成立。

```bash
# 有限计数，不是模块/cfg/生产可达性分析
git show 8fddee2:src-tauri/src/commands.rs | grep -c 'include!'
git ls-tree -r --name-only 8fddee2 -- src-tauri/src/commands | grep -E '^src-tauri/src/commands/[^/]+\.rs$' | wc -l
# 用 git show 读取上文路径，确保审查的是固定基线与完整事务
git show 8fddee2:src-tauri/src/agent_runtime/multi_agent/scheduler/lifecycle.rs
git show 8fddee2:src-tauri/src/agent_runtime/single_projection/writer.rs
git show 8fddee2:src-tauri/src/commands/multi_agent_gap_recovery.rs
git show 8fddee2:src-tauri/src/commands/multi_agent_review_recovery.rs
```

以下是需要适当依赖/平台的验证命令，**不是本次文档复核的已通过结果**：

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml --all-targets --message-format short
cargo test --manifest-path src-tauri/Cargo.toml --lib
# 只在 macOS 上验证对应平台测试；非 macOS 的零匹配不算通过
cargo test --manifest-path src-tauri/Cargo.toml --lib macos_static_parser -- --nocapture
```

记录每次运行的 commit、平台、命令、退出码、日志及过滤条件；区分通过、失败、被阻断、
零匹配和未运行。原作者的 SIGABRT 与编译报告、318 项失败抽样归因、依赖漏洞状态、
跨主机认证跳转和完整生产 unwrap 数量，本次均未独立验证。

## §9 本次窄范围修复与实际验证（2026-10-13）

首批只修复两个具体问题，并纠正文档/过期注释：

- 认证会话校验移除 `danger_accept_invalid_certs(true)`；提取实际命令使用的 helper 供同路径测试，无不安全重试
- 测试专用 Fixture 清理失败不再 panic，诊断写入失败也不 panic；实际 Seatbelt 探针仍仅在 macOS 测试中运行
- 纠正 `store.rs` / `reducer.rs` 模块注释，补充本审计与架构入口；没有修改终态、恢复或预算数据库逻辑

下表是本次修复工作树在 **Linux x86_64、Rust 1.99.0、Debian GTK/WebKit 用户态依赖**
下的实际结果，区别于上文原作者的历史报告。Rust 测试使用 `CARGO_BUILD_JOBS=2`、
`CARGO_PROFILE_TEST_DEBUG=0`，回环测试通过 NO_PROXY 绕过代理；凭据与证书均为合成 fixture。

| 验证 | 结果与边界 |
|---|---|
| `npm run build` | 通过 |
| `npm run test:agent-workbench` | 54 通过 |
| `npm run test:ui` | 934 通过；与其他前端测试有覆盖重叠，不相加宣称唯一用例数 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib auth_session::validation_tests -- --test-threads=1` | 3 通过；真实库编译与实际校验 helper 路径 |
| 同一已编译测试二进制，过滤 `auth_session:: --test-threads=1` | 13 通过，包含上述 3 项 |
| 同一已编译测试二进制，过滤 `cleanup_tests --test-threads=1` | 3 通过；正常清理、清理/诊断失败、unwind 保留原 panic |
| 隔离副本仅恢复 `danger_accept_invalid_certs(true)` 的变异验证 | 两个 TLS 拒绝测试均失败，0 通过 / 2 失败，退出 101；证明测试能捕获该回归，不是修复版测试失败 |
| `cargo clippy --lib --tests -- -D warnings` | 修复版与精确 `8fddee2` 基线均退出 101；两者相同的 27 组规范化诊断，无新增或消失项。此检查未发现本补丁新增 lint，但严格 Clippy **未通过** |

TLS 测试覆盖直接请求和跳转至不可信证书：对端未收到 HTTP 认证字节，且没有不安全
重试；失败保留原会话状态并记录错误。成功对照使用**回环 HTTP**验证合成凭据及状态
处理，不是受信任 HTTPS 站点验收，也不覆盖所有跨主机 Cookie 传播行为。

未运行完整 Rust 测试套件、独立 macOS Seatbelt 探针、已安装应用或真实站点/真实密钥
验收。类型化状态机进一步收口、keychain 迁移、schema/模块重构等仍属后续设计；
先前本地 PR #2 的改动不包含在本批次。不能据上述通过项称为“全应用通过”或“全部审计项完成”。
