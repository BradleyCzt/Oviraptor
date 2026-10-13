# Oviraptor 设计层深度审计（2026-10-13）

本文件是一次**只读的设计审计**。不改代码、不改配置、不触碰任何业务逻辑，
只记录"设计上是否存在问题"以及可复现的证据。

与同目录其他文档的分工：

- 本文**不重复** [`NEST_PROJECT_IMPROVEMENT_AUDIT_2026-10-12.md`](./NEST_PROJECT_IMPROVEMENT_AUDIT_2026-10-12.md)
  已记录的工程卫生项（CI、lockfile、LICENSE、README 死链）。
- 本文聚焦**设计层**：不变量的声明与实现是否一致、模块边界是否成立、
  架构承诺是否被代码强制、以及开发过程本身是否存在结构性问题。
- 本文对 10-12 审计的两项数据做了**独立复测并修正**，见 §7。

基线：`main` @ `8fddee2`（`Merge current Nest checkpoint with GitHub main`）。
本文所有数字均在该基线上实测，命令见 §8，可复现。

---

## §0 结论

**代码本身不差，但存在一个 P0 级设计缺陷：整个多智能体运行时赖以成立的
"单写终态"不变量，只存在于注释里，代码层没有任何强制机制，且有 28 处生产写入
正在绕过它。**

同时需要指出的是：项目**已经**建成了一套成熟的"用 DB trigger 强制不可变"的技术
（353 个 trigger，见 §1.3），只是这套技术从未被应用到 `agent_runs.terminal_state`
上。因此这是**遗漏而非能力缺失**，修复成本极低。

次要的结构性问题：`include!` 摧毁了命令层模块系统（§2）、数据层没有 schema
所有者与迁移账本（§3）、凭据明文存储且一处带凭据的客户端关闭了 TLS 校验（§4）。

最深层的问题不在代码，而在**过程**：306 份文档对应 43 万行代码，其中 255 份
标注"未完成"，22 个测试文件在断言源码字符串（§6）。

规模基线：

| 指标 | 实测值 |
|---|---|
| Rust 代码 | 379,436 行 / 1,593 文件 |
| `commands.rs` 的 `include!` 语句 | 151 条 |
| `commands/` 目录文件数 | 756 |
| `agent_runtime/` | 352 文件 / 75,291 行 |
| 前端（`.vue` + `.ts`） | 30,944 行 |
| Tauri 命令数 | 269 |
| 独立表数 | 281 |
| `.sql` schema 文件 | 48 |
| `docs/` 文档数 | 303（其中 AUDIT 223） |

---

## §1 P0 · "单写终态"是注释承诺，不是代码约束

### 1.1 声明与实现不一致

`src-tauri/src/agent_runtime/store.rs:4` 的模块文档写着：

```rust
//! Model rounds and tool executions may run concurrently, but they only ever
//! append here. The terminal state is written by `reducer` alone.
```

而实现这条不变量的函数：

```rust
// src-tauri/src/agent_runtime/reducer.rs:202-205
/// Reduce and persist. A run that already holds a terminal state keeps it — the
/// second writer is rejected rather than allowed to rewrite history.
#[cfg(test)]
pub fn commit(
```

**`reducer::commit` 标注 `#[cfg(test)]`，生产代码零调用点。**
`reducer::reduce`（`reducer.rs:31`）是纯函数，从不触碰 SQL。
因此"终态只由 reducer 写入"这一断言**在release 构建中不存在任何执行体**。

### 1.2 实际有 28 处终态写入，各带互不兼容的守卫

实测 `SET status='terminal'` 分布在 **27 个文件、28 处**，各自的 WHERE 谓词互不相同：

| 位置 | WHERE 守卫 |
|---|---|
| `agent_runtime/store.rs:998` | `AND status<>'terminal'` |
| `agent_runtime/single_projection.rs:122` | `AND status IN (...) AND terminal_state='' AND finished_at=''` + 精确重读 |
| `agent_runtime/multi_agent/scheduler/lifecycle.rs:278` | `WHERE id=?4 AND root_run_id=?5` —— **无任何状态守卫** |
| `agent_runtime/multi_agent/directive/reconciliation.rs:310` | `AND status='paused'` |
| `agent_runtime/multi_agent/specialist/homogeneous_closure.rs:31` | `AND status<>'terminal'` |
| `agent_runtime/multi_agent/homogeneous/delivery.rs:230` | `AND status<>'terminal'` |
| `commands/multi_agent/child_accounting.rs:266` | `AND status<>'terminal'` |
| `commands/multi_agent/readonly_assessment.rs:273` | `AND status='paused'` |
| `commands/multi_agent_gap_recovery.rs:131` | `AND (status='paused' OR (status='terminal' AND terminal_state='failed'))` |

由此产生两条可直接推导的正确性风险：

**风险 A —— 静默覆盖已终态的 run。**
`lifecycle.rs:278` 的 UPDATE 只检查"行存在且 root_run_id 匹配"（`run_changed != 1` 才报错），
没有检查既有 `terminal_state`。一个已经 `completed` 的子 run 可以被这条路径改写。

**风险 B —— 显式改写终态并注入合成终态码。**
`commands/multi_agent_gap_recovery.rs:129-133`：

```sql
UPDATE agent_runs SET status='terminal',terminal_state='completed',
  terminal_code='gap_receipt_reconciled', ...
WHERE id=?1 AND root_run_id=?2
  AND (status='paused' OR (status='terminal' AND terminal_state='failed'))
```

它把 `terminal_state='failed'` **改写成 `'completed'`**，并写入
`gap_receipt_reconciled` 这个任何 reducer 都从未产生过的终态码。
这与 `reducer.rs:211-212` 注释中的 "First writer wins: a run that already holds a
terminal state is never rewritten" 直接矛盾。

同族路径还有 `commands/multi_agent_review_recovery.rs`，写入
`review_receipt_reconciled`，属同一模式。

### 1.3 互斥完全依赖 SQLite 文件锁

`agent_runtime` 全目录 `Mutex|RwLock` 仅 19 处命中，**无一处作用于 store**。
互斥来自 `db.rs:21` 的 `busy_timeout=10s` 与 `db.rs:374` 的 WAL，
加上每条 SQL 自己写的 WHERE。换言之：**正确性完全依赖每个调用点的自律**，
没有任何结构性保障。

### 1.4 关键背景：这套技术项目已经会，只是没用在这里

实测全库 **353 个 `CREATE TRIGGER`**，其中大量正是"不可变性守卫"模式，
且实现得相当完整（update / delete / replace 三向覆盖）：

```sql
-- agent_runtime/multi_agent/budget/root/model/tick/schema.sql:72
CREATE TRIGGER IF NOT EXISTS root_tick_terminal_proof_no_update
BEFORE UPDATE ON agent_root_tick_terminal_proofs
BEGIN SELECT RAISE(ABORT,'root_tick_terminal_proof_immutable'); END;

-- agent_runtime/multi_agent/budget/recovery_scope.schema.sql:40
CREATE TRIGGER IF NOT EXISTS recovery_scope_terminals_update
BEFORE UPDATE ON agent_recovery_scope_terminals
BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
```

覆盖面包括 `agent_root_tick_terminal_proofs`、`agent_recovery_scope_terminals`、
`agent_single_projection_receipts`、`agent_root_elapsed_facts`、
`agent_root_cost_reconciliation_receipts` 等。

**唯一的 `agent_runs` 触发器是一个 DELETE 保留守卫**：

```sql
-- agent_runtime/multi_agent/budget/root_paid_run_retention_schema.sql:9
CREATE TRIGGER IF NOT EXISTS root_paid_open_run_retained BEFORE DELETE ON agent_runs
```

也就是说：**写对数据库不变量的能力已经在仓库里被证明、已被使用了 350 多次，
唯独没有用在争议最大的 `agent_runs.terminal_state` 上。**

### 1.5 建议

单条 trigger 即可让不变量不可绕过，且不需要改任何调用方：

```sql
CREATE TRIGGER agent_runs_terminal_immutable
BEFORE UPDATE ON agent_runs
WHEN OLD.terminal_state <> '' AND NEW.terminal_state <> OLD.terminal_state
BEGIN SELECT RAISE(ABORT,'agent_run_terminal_immutable'); END;
```

加上这条之后，`lifecycle.rs:278` 与 `gap_recovery.rs:131` 会立即暴露为真实缺陷
（而不是静默成功），此时再决定是修调用方还是走显式的、带审计的改写通道。
**这是本文档建议中投入产出比最高的一项。**

---

## §2 P1 · `include!` 摧毁了命令层模块系统

### 2.1 事实

`src-tauri/src/commands.rs` 用 **151 条 `include!()`**（`commands.rs:39-213`）
把 `commands/` 下 **756 个文件**拼成**单一扁平命名空间**。

`include!` 是文本拼接，不是模块声明。由此产生的直接后果：

- **无文件级命名空间**。任何两个文件的同名项都会直接编译冲突。
- **无文件级 `use` 卫生**。全部导入被提升到 `commands.rs:1-19`，
  每个文件都能看到 `db`、`rusqlite`、`Sha256`、`Base64`、`Command`、`thread`、`Instant`，
  无论是否使用。
- **依赖关系不可见**。实测 **139/151 个文件引用了其他文件定义的项，
  源码中无任何声明**。例如 `commands/ownership_o3_tests.rs:68` 调用
  `ownership_review.rs:1594` 定义的 `start_ownership_analysis_with_mode` ——
  在正常 `mod` 语义下这需要可见性修饰符；现在不需要，
  所以 grep 定义位置在调用点附近找不到任何东西。

### 2.2 耦合中枢

九个文件成为事实上的 crate级共享库，但**没有 `pub(crate)` 标记**，
rustc 不施加任何约束，评审者也得不到信号：

| 文件 | 被依赖文件数 |
|---|---|
| `commands/agent_contract.rs` | 132 |
| `commands/agent_instruction.rs` | 116 |
| `commands/native_helpers.rs` | 95 |
| `commands/native_source_scope.rs` | 95 |
| `commands/scan_execution_runtime.rs` | 94 |

`agent_contract.rs`（1,335 行，0 个命令，49 个函数）是纯共享类型中枢。
改动它会触发整个命令层重编译与类型检查。

### 2.3 仓库内已有正确样板

**`commands/exposure.rs:8-28` 已经用真正的 `pub mod exposure { ... }` 子模块树**
（34 个子模块、44 个文件）组织，且零跨文件泄漏。
同一仓库内的同一种工作，两种截然不同的封装质量——
说明问题不在团队能力，而在这一层没有统一约束。

### 2.4 建议

按依赖倒序逐文件把 `include!` 翻成 `pub(crate) mod x;`。
隐藏依赖会直接变成**编译错误**，那份错误列表就是项目真实的依赖图。
以 `exposure.rs` 为模板。

---

## §3 P1 · 数据层没有 schema 所有者，也没有迁移账本

### 3.1 四个独立的 schema 定义点

实测 **281 张独立表**分散在至少四个互不感知的定义位置：

| 位置 | 内容 |
|---|---|
| `src-tauri/src/db_schema.rs`（2,339 行） | 单个 `concat!` 巨块，126 张 `CREATE TABLE` + 93 个 `CREATE INDEX`，尾部再 `include_str!` 进 13 个 `.sql` |
| `src-tauri/src/db.rs:382-463` | 25 次独立 `execute_batch`，针对 48 个 per-module `.sql`（共 102 张表） |
| `src-tauri/src/db_initialize_columns.rs` | 11 张表 |
| `commands/exposure/schema.sql` 等 feature 内 | 25 张表（exposure） |

后果：**无法 diff 一次schema 变更，无法回答某张表归哪个模块所有，
改一个表名要在无界文件集里 grep。新增表是各模块的局部约定，不是 schema 变更。**

### 3.2 迁移版本记在 key-value 表里

`db_migrate.rs:47-65` 把迁移版本写进 `app_settings(key,value)`，
**`PRAGMA user_version` 全库零使用**。

`ensure_column`（`db_migrate.rs:25-45`）本身是正确的 check-then-ALTER-then-reverify
幂等实现。问题在于它**没有编号、没有顺序保证、跨迁移集无原子性、无 down 迁移**，
并且 **57 处 `ADD COLUMN` 分布在 4 个不同模块**：

- `db_initialize_columns.rs` 30 处
- `commands/exposure/state.rs` 21 处
- `db_initialize_repairs.rs` 5 处
- `db_migrate.rs` 1 处

由 25 次 `ensure_column` 调用驱动。

后果：中途崩溃会让数据库停在中间形态，下次启动原样重跑；
**无法证明某个用户的数据处于哪个 schema 版本。Schema 状态是涌现的，不是声明的。**

### 3.3 一个已经埋好的地雷

`commands/exposure/state.rs:389-396` 用**按 `;` 切分 SQL 文本**的方式执行 DDL，
并把匹配 `"already exists" || "duplicate column name"` 的错误吞掉（`:406-408`），
另有 3 个具名容忍项（`:398-404`）。

它现在能跑通，只因为该文件恰好 **0 个 `CREATE TRIGGER`**。
一旦有人在该schema 里加一个带 `BEGIN ... END` 的触发器，语句会被静默切成碎片、
执行失败、然后被当作"已存在"吞掉——结果是**部分 schema，且不会向用户暴露任何错误**。

注意 §1.1 建议的 trigger 修复正好会踩到这个执行器：必须同时把它换成
`execute_batch` 原生多语句执行。

### 3.4 连接管理

PRAGMA 配置本身是**正确的**：

```rust
// db.rs:21  busy_timeout(Duration::from_secs(10))
// db.rs:24  pragma_update(None, "foreign_keys", "ON")
// db.rs:31  pragma_update(None, "synchronous", "NORMAL")
// db.rs:374 pragma_update(None, "journal_mode", "WAL")   // initialize 时设一次，附有正确理由注释
```

但 `db::open`（`db.rs:12-35`）**不被缓存也不池化**，实测 **462 处非测试调用点**
（其中 411 处在 `commands/`），外加 81 处直接 `rusqlite::Connection::open`。
`db.rs:17` 的 `SQLITE_OPEN_FULL_MUTEX` 在"每命令一个连接"的前提下没有意义。

SQLITE_BUSY 是**被缓解**（10s 超时 + WAL），不是被**预防**。

### 3.5 事务密度

非测试代码实测 `.execute(` 900 处，事务约 96 处——约 **1 : 9.4**。
不变量相关的多语句写有正确的事务包裹（正面例子：
`commands/ownership_operations.rs:539-620` 把 decision-UPDATE、INSERT、
`asset_events` 追加、item 插入与 `done_items=done_items+?4` 计数器放进同一事务），
但把不变量编码进单条自提交语句是常态。

### 3.6 增长无上界

实测**零 `VACUUM`、零 `auto_vacuum`、零数据库行级保留/清理作业**。
README 声称"只保存当前状态和变化事件"，但实际是**快照 + 历史并存**：
`ownership_operation_items` 按 item 持久化 `old_json`/`new_json`，
另有 `config_snapshot`、`snapshot_json`、`source_snapshot_json` 等。
全库 **201 个 `_json TEXT` 列**（`db_schema.rs` 内 123 个）充当不可查询的伪表。

---

## §4 P1 · 凭据明文存储，且一处带凭据的客户端关闭了 TLS 校验

### 4.1 凭据明文

`db_schema.rs` 中以下列均为明文 `TEXT`：

```sql
:19   settings_json TEXT NOT NULL DEFAULT '{}'   -- 内含 modelApiKey / localApiKey
:33   access_token  TEXT NOT NULL
:412  api_key       TEXT NOT NULL DEFAULT ''
:497  api_key       TEXT NOT NULL
:561  api_key       TEXT NOT NULL
:860  worker_token  TEXT NOT NULL DEFAULT ''
:953  worker_token  TEXT NOT NULL DEFAULT ''
:2317 owner_token   TEXT NOT NULL
```

DB 文件权限 0600（`db.rs:376-381`）是**操作系统层保护，不是加密**。

**README 中的 AES-256-GCM 只覆盖导入 artifact**，实现位于
`artifact_import/service/objects.rs:35` 的 `sealing::Key::load_or_create(context.key_path)`，
密钥为每安装随机 32 字节文件（非口令派生）。它**不覆盖数据库凭据列**。

后果：任何同用户进程、备份同步或失窃的笔记本，
都能直接读出 HackerOne / LLM / FOFA 明文密钥。

### 4.2 TLS 校验被关闭 —— 且其中一处携带真实 Cookie

实测 `danger_accept_invalid_certs(true)` 有 **3 处**：

| 位置 | 上下文 | 是否携带凭据 |
|---|---|---|
| `src-tauri/src/auth_session.rs:1209` | 浏览器会话校验 | **是** |
| `src-tauri/src/commands/native_frontend_recon.rs:432` | 前端侦察 | 否（复核后确认不挂 cookie） |
| `src-tauri/src/jobs/native_probe.rs:764` | `.danger_accept_invalid_certs(!options.strict_tls)` | 用户开关控制 |

`auth_session.rs` 这一处最严重，因为它**确实发送真实会话 Cookie**：

```rust
// auth_session.rs:1193-1204  构造 cookie_header（已按 domain 过滤）
let client = Client::builder()
    .timeout(Duration::from_secs(20))
    .redirect(reqwest::redirect::Policy::limited(8))
    .danger_accept_invalid_certs(true)   // ← 认证被关闭
    .build()?;
...
match client.get(&target).headers(headers).send() {   // headers 含 COOKIE
```

cookie 的 domain 过滤本身是正确的（`target_host == domain || target_host.ends_with(.{domain})`），
但这不能补偿被关闭的 TLS 认证：**关闭证书校验意味着主动中间人可以用自签证书
终止 TLS 并读取该 Cookie**。同时 `Policy::limited(8)` 允许跟随 8 次跳转。

对一款以"授权资产管理"为定位、需要持有真实站点登录态的工具，
这是一条直接的凭据外泄路径。

### 4.3 审计日志不可防篡改

`commands/agent_http_journal.rs` 把请求claim 持久化到
`agent_http_budget_origins` / `agent_http_request_claims`，带 `request_hash`
与 `ordinal` 计数器，但**全文件无哈希链、无 HMAC、无外部锚定**。
完整性完全依赖 SQLite 事务边界。

对一个价值在于**证据性**的暴露面评审工具，本地攻击者编辑或删除日志行
即可伪造或抹除 agent 实际发出的请求。

### 4.4 做得好的部分（应保留）

这些是真实加固，不是文档声明：

- **容器沙箱**：`tool_supply/sandbox/container_parser.rs:15-16` 硬编码 `sha256:` digest，
  以 `--pull=never --network=none --read-only --cap-drop=ALL
  --security-opt=no-new-privileges`（`:399-400`）启动，配 seccomp、
  每次运行 nonce 标签、journal 验证容器身份（`:475-478`）。调用方无法传入镜像。
- **SSRF 作用域门禁**：单一入口 `agent_tools_scope.rs:1-8`
  （"The one scope gate for every URL this run may touch"），
  强制 `http|https`（`:28-30`），冻结的 `allowed_origins` 白名单（`:48-52`），
  域名匹配为 exact-or-dot-suffix（`asset_ownership.rs:29-33`），
  `evil-example.com` 无法匹配 `example.com`。重定向 `Policy::none()`（`exchange.rs:68`）
  且每次重新评估（`agent_tools_scope.rs:122-145`）。**未找到绕过。**
- **artifact 导入路径安全**：`artifact_import/manifest.rs:21-26` 拒绝 `..` 与绝对路径，
  `discovery.rs:94-96`、`:242-244` 用 `symlink_metadata` 拒绝符号链接，
  并做 NFC 归一化以防 Unicode 等价绕过。
- **凭据入库前脱敏**：`agent_runtime/secrets.rs:8-65` 维护 40 条名称 + 13 条后缀
  的拒绝清单；`exchange.rs:340-343` 在写审计记录**之前**生成脱敏视图。
- **授权 fail-closed**：`agent_tools_dispatch.rs:188-234` 无run 台账即拒绝，
  每次请求重验租约；`review-only.json` 能力包将工具绑定 `sideEffectClass: none`。
- **子进程**：全部使用 `Command::new` 参数数组，**未发现 shell 字符串拼接**；
  git 调用显式禁用 hooks/smudge（`native_pipeline/snapshot.rs:265-274`），
  这是针对恶意仓库代码执行的真实缓解。

### 4.5 已知缺口

- **无私网/回环/链路本地地址拦截**。`agent_tools_http/` 下
  `169.254` / `is_loopback` / `is_private` 全部零命中。
  一个解析到 `169.254.169.254`（云元数据）的授权域名是可达的，
  DNS rebinding 路径未在解析器层关闭。
- **Tauri capability 只管插件命令**。`capabilities/default.json` 仅授予
  `core:default` / `opener:default` / `dialog:allow-open`，前端无法起进程或读任意文件，
  但 `lib.rs` 中 269 个 `#[tauri::command]` **全部可被前端调用**——
  前端一旦被攻破即等于全后端。当前由严格 CSP
  （`tauri.conf.json:33`，`default-src 'self'`，`connect-src` 仅 `ipc:`）缓解。
- **依赖钉版不一致**。`rustls = "=0.23.35"`、`rustls-platform-verifier = "=0.6.2"`、
  `toml = "=0.9.12"` 已钉；但 `aws-lc-rs = "1.17.3"`（**承载全部 AES-GCM 的加密后端**）
  与 `reqwest`、`tauri`、`rusqlite` 为 caret 未钉。

---

## §5 P1/P2 · 编排层的其余结构性问题

### 5.1 多套恢复机制并行且不收敛

- **Replay 窄**：生产路径只从 `commands/agent_native/executor.rs:123` 调用一次
  `checkpoint::recover`（`if context.resume`）。`write_checkpoint` 的生产调用者只有 2 处。
- **队列在内存**：`executor.rs:112` 的 `Vec<String>` 每次从 state 重建，
  崩溃即丢。
- **reconciliation 是并行手写族**：9 个 `*recover*/*recovery*/*expired*/*saved*/*reentry*`
  文件，各自独立重新推导终态状态。
  **§1 的 28 处守卫分裂，正是这一结构的直接产物。**

### 5.2 预算没有收口点

唯一的模型出口是 `agent_runtime/model/transport.rs:188`（`call.send()`），
但 `agent_runtime/model/` 中**零处引用预算记账**。
`reserve` / `release` / `settle` 的裸 SQL 出现在 **214 个非测试调用点**。

后果：任何新增调用路径只要不先调 `budget::model::reserve` 就是无上限花费，
没有任何结构性机制阻止。`model/admission.rs:88-89` 的 LOCAL/CLOUD
是**并发信号量，不是成本上限**（其自身文档 `:2` 明确说明）。

### 5.3 panic 无人接管

**`catch_unwind` 全库零次。**
`transport.rs:126-129` 是唯一正确处理的样板（spawn + join + 转为
`TransportError::Network`），但另有 **8 处 `.join().ok()` / `let _ = ... .join()`
丢弃 `Err(panic)`**。

任何非 transport 的 agent 线程 panic 后，
`agent_assignments.state` 停在 `'running'`，只能靠租约过期 reconciler兜底。

### 5.4 状态被重复编码

run 状态同时存在于 `agent_runs.status` / `terminal_state` / `finished_at`、
`agent_assignments.state`、`agent_assignment_attempts.state`、
`agent_homogeneous_tasks.state`、`agent_worker_instances.state` 等多处。
`gap_recovery.rs:131` 不得不用一个 WHERE 把它们全部消歧。

### 5.5 IPC 契约三处手工同步

269 个 Rust 命令 → `lib.rs` 的 `invoke_handler!` 注册 → 257 个 TS wrapper →
`src/types.ts`（1,802 行，手工镜像 385 个 Rust `pub struct`），
**全程无 codegen**。`vue-tsc --noEmit` 只能校验 TS 内部一致性，
**查不出 Rust 侧漂移**。新增一个命令需手工改 5 处。

---

## §6 过程层 · 最深的问题

这一层比任何单点代码缺陷都更值得关注。

### 6.1 文档与未完成项

| 指标 | 实测 |
|---|---|
| `docs/` 文档数 | 303 |
| 其中 `*AUDIT*` | 223 |
| 含"未完成/未收口/待办/遗留"字样 | **255** |
| Rust 代码 | 379,436 行 |

Git 历史仅 32 个提交，从 `Initial import: Oviraptor 1.1.4` 开始，
其后是 `Loop1`–`Loop6`、`Batch A–D`、`Checkpoint`、`Takeover plan`、`handoff`、
`scope the remaining items to the upstream maintainer`。
**最近 11 个提交集中在同一天，其中 4 个是纯文档。**

README 指向的权威边界文档 **`docs/architecture.md` 在 `main` 上不存在**
（README 说"业务模块边界、Tauri 命令归属和新增代码规则见"该文件），
即边界规则文档已随 `Delete docs directory`（`2e0cd53`）丢失而 README 未同步。

### 6.2 `*_source_pin` 揭示的测试范式

22 个 `*_source_pin.rs` 文件采用同一模式：

```rust
let src = include_str!("agent_native/coordinator_rust_policy.rs");
let body = &src[start..end];
assert!(body.contains("NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS"), "...");
assert!(body.contains("NATIVE_A03_PRODUCER_BLOCKERS"), "...");
```

这类测试**把源码当字符串读入，再断言子串存在**：

- 重命名一个常量 → 测试变红，行为完全未变；
- 把逻辑内联到别处 → 测试变红，行为完全未变；
- **真正引入一个缺陷，只要保留这些子串 → 测试依然绿。**

它唯一的驱动力是让审计条目变绿，而不是让代码变对。
同类现象在测试命名上留下了痕迹：`agent_tests_a03_blockers_remaining_*`、
`agent_tests_gate_blockers_source_pin` 等。

### 6.3 已排查的非问题

为免误报，以下几项经复核后**排除**：

- `_actual` 后缀（638 处）主要是财务域术语"实际成本 vs 预估成本"
  （如 `original_actual_cost`、`verify_actual_costs`），生产代码中仅 10 个函数定义，
  **不是新旧实现并存的残留**。
- `include!("commands/tests.rs")` 与 `include!("commands/agent_tests.rs")`
  虽无 `#[cfg(test)]`，但文件**内部以 `#[cfg(test)] mod { ... }` 包裹**，
  其下 128 条 `include!` 全部在该门内，**不进 release 二进制**。
- 生产 `.unwrap()` 实际约 **43 处**（不是按文件名统计得到的 941/1184），
  且多为静态正则编译、`OnceLock` 初始化等不变量断言，**崩溃风险低**。

---

## §7 对 2026-10-12 审计的两项修正

| 项 | 10-12 审计记录 | 独立复测 |
|---|---|---|
| 非测试 `.unwrap()` | "> 1184 行" | **约 43 处生产**。差异来自按文件名而非 `#[cfg(test)]` 边界切分——`ownership_review.rs` 的首个 `#[cfg(test)]` 在第 835 行，其下 157 处 unwrap 全在测试段内。**该风险等级应从 P2 下调。** |
| 测试代码进 release | 未断言 | **未进**。`commands/tests.rs:1`、`commands/agent_tests.rs:1` 均以 `#[cfg(test)] mod` 开头。**无 release 影响。** |

其余结论（无 CI、双 lockfile、无 LICENSE、README 死链、代码健康）本文不复述。

---

## §8 建议优先级

| # | 事项 | 性质 | 成本 |
|---|---|---|---|
| 1 | 给 `agent_runs.terminal_state` 加不可变 trigger（§1.5），让 28 处守卫分裂立即暴露 | 正确性 | 极低 |
| 2 | 移除 `auth_session.rs:1209` 的 `danger_accept_invalid_certs(true)`（§4.2） | 安全 | 极低 |
| 3 | `schema_migrations` 表 +启用 `PRAGMA user_version`，收编 57 处启动期 `ADD COLUMN`（§3.2） | 数据完整性 | 中 |
| 4 | 凭据列迁到 OS keychain（macOS Keychain / Windows Credential Manager），SQLite 只留 handle（§4.1） | 安全 | 中 |
| 5 | 按依赖倒序把 `include!` 翻成 `mod`，以 `exposure.rs` 为模板（§2.4） | 可维护性 | 大（可增量） |
| 6 | 引入单一 `RunWriter` 持一条长连接，terminal 写入与 budget reserve 作为其仅有的两个特权操作（§1.3、§5.2） | 正确性 + 成本 | 大 |
| 7 | 删除 14 条并行恢复路径，收敛为单一 replay + 类型化 reconciliation（§5.1） | 正确性 | 大 |
| 8 | agent 线程边界补 `catch_unwind`（§5.3） | 健壮性 | 小 |
| 9 | 审计日志加哈希链并外部锚定（§4.3） | 完整性 | 中 |
| 10 | 删除 `*_source_pin` 断言源码字符串的测试，改为行为断言（§6.2） | 测试有效性 | 中 |

**顺序建议**：1 与 2 是"低改动、高收益、立即止血"，应先做。
1 的效果尤其关键——它会让 §1.2 列出的两处真实缺陷**从静默成功变为可观测失败**，
后续所有修复才有可靠的判断依据。

---

## §9 复现命令

```bash
# §1.1 单写终态未被强制
grep -n -B2 "pub fn commit" src-tauri/src/agent_runtime/reducer.rs
grep -n "terminal state is written by" src-tauri/src/agent_runtime/store.rs
grep -rn "SET status='terminal'" src-tauri/src --include=*.rs | wc -l      # 28
grep -rn "SET status='terminal'" src-tauri/src --include=*.rs | cut -d: -f1 | sort -u | wc -l  # 27 文件
grep -rn "root_run_id=?5" src-tauri/src/agent_runtime/multi_agent/scheduler/lifecycle.rs
grep -n "terminal_state='failed'" src-tauri/src/commands/multi_agent_gap_recovery.rs

# §1.4 trigger 模式已存在
grep -rn "CREATE TRIGGER" src-tauri/src --include=*.sql --include=*.rs | wc -l   # 353
grep -rn "BEFORE UPDATE ON agent_runs" src-tauri/src --include=*.sql

# §2 include! 结构
grep -c 'include!' src-tauri/src/commands.rs                                   # 151
ls src-tauri/src/commands/*.rs | wc -l                                          # 756
sed -n '8,28p' src-tauri/src/commands/exposure.rs                              # 正确样板

# §3 数据层
grep -rn "PRAGMA user_version\|user_version" src-tauri/src                     # 0 命中
grep -rn "db::open(" src-tauri/src --include=*.rs | grep -viE "test" | wc -l   # 462
grep -rn "ALTER TABLE" src-tauri/src --include=*.rs | grep -viE "test" | wc -l
find src-tauri/src -name "*.sql" | wc -l                                        # 48

# §4 安全
grep -rn "danger_accept_invalid_certs" src-tauri/src --include=*.rs            # 3 处
grep -n "api_key TEXT\|access_token TEXT\|owner_token TEXT\|worker_token TEXT" src-tauri/src/db_schema.rs
grep -rn "load_or_create" src-tauri/src --include=*.rs | grep -i sealing

# §5 编排
grep -rn "catch_unwind" src-tauri/src --include=*.rs | wc -l                    # 0

# §6 过程
ls docs/*.md | wc -l                                                            # 303
grep -rlE "未完成|未收口|待办|遗留" docs | wc -l                                 # 255
find src-tauri/src -name "*source_pin*" | wc -l                                 # 22
git cat-file -e origin/main:docs/architecture.md                               # 失败：死链

# 全量编译验证（基线可信：30 秒完成，零错误零警告）
cd src-tauri && cargo check --all-targets --message-format short
```

---

## 附：未验证项

诚实标注本文的边界：

- **全量 `cargo test --lib` 未完成**。本次尝试运行约 15 分钟未结束，
  因此 10-12 审计引用的"3066 passed / 318 failed"基线**仍未被独立确认**。
  本文所有关于测试的判断均基于代码结构与文件内容，未依赖该基线数字。
- `Cargo.lock` 的已知漏洞状态未审计。
- `resources/workers/*.cjs` Node helper 进程的沙箱强度未评估
  （`commands/environment.rs:71-77` 自述为 unsupported）。
- §4.2 中 reqwest 在跨主机跳转时是否自动剥离 Cookie 头，
  未通过运行时实验确认；但这不影响结论——**TLS 认证被关闭本身已足以让
  同主机上的主动中间人读取该Cookie**。