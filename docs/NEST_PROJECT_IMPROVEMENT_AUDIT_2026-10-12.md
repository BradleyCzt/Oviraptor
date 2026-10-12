# Oviraptor 项目不足与改进审计（2026-10-12）

本文件是一次**全项目横向审计**，目的在于列出当前仓库在工程化、测试、
文档、代码健康、安全发布等维度上的不足，并给出按优先级排序的改进建议。

与同目录下的专项审计（exposure、worker、web 等）不同，本文件**不针对单一业务链**，
而是从仓库整体视角收敛"还差什么、该先改什么"。

## 读法与证据口径

- 标注 **〔实测 2026-10-12〕** 的数字是本次审计在 `homogeneous-execution-closure`
  分支（HEAD `1c87975`）直接跑命令得到的，命令随行给出，可复现。
- 标注 **〔沿用交接〕** 的数字引自当日交接文档
  （`NEST_PREEXISTING_DEBT_HANDOFF_2026-10-12.md`、
  `NEST_HOMOGENEOUS_EXECUTION_CLOSURE_PLAN_2026-10-12.md`），本次未重跑（成本过高），
  仅作引用，不作为本审计独立验证结论。
- 本文件只做**记录与建议**，不改任何代码，不触碰交接文档点名的冻结区
  （exposure 业务与前端、七动作枚举、legacy 固定角色默认路径、真实任务库、
  `/Applications/Oviraptor.app`）。每条建议都标注了**归属**：可在贡献分支推进，
  还是属于上游 main 维护者的验收职责。

## 优先级总览

| 级别 | 条目 | 一句话 |
|---|---|---|
| **P0** | §1 无 CI 流水线 | 657 个 Rust 测试文件 + 98 个前端测试 + clippy 门禁全靠人工，回归无自动兜底 |
| **P0** | §2 回归基线未固化 | 存量 318 失败 + 7 挂起无基线快照，"不新增失败"无法机器判定 |
| **P1** | §3 无 LICENSE | 面向授权场景的分发物没有任何许可条款 |
| **P1** | §4 工具链/依赖不确定 | 双 lockfile、无 rust-toolchain 钉版、发布 `sourceBinding=not_verified` |
| **P1** | §5 文档漂移 | README 死链（`docs/architecture.md` 缺失）、305 份文档缺索引 |
| **P2** | §6 代码健康 | 超大文件、海量 `unwrap()`、106 处 `allow(dead_code)`、112 行 `unsafe` 无集中不变量说明 |
| **P2** | §7 前端测试形态 | 前端"测试"是 DOM 字符串 node harness，无组件级/E2E 层 |
| **P2** | §8 存量业务债 | exposure 三高危、legacy strix 物删未收口、真实旧数据清理半程（均已在专项文档登记） |

---

## §1 无持续集成（P0，可在贡献分支推进基础骨架）

**现状** 〔实测〕：仓库无任何 CI 配置。

```bash
ls .github/workflows   # => NO .github/workflows
```

**影响**：项目已积累极重的自动化资产——

- 〔实测〕Rust 测试文件 657 个；前端 `tools/*.cjs|mjs` 测试 98 个；
  `package.json` 定义了 `test:ui`（单条即串联 70+ 个前端测试）、`test:exposure`、
  `test:agent-dialog` 等十余个测试套件。
- 〔沿用交接〕clippy 以 `-D warnings` 作为门禁，当日刚清偿 26 条存量告警。

这些全部依赖人工在本地逐条执行。没有 CI 意味着：任何一次提交都**没有机器证明**
通过了 `cargo clippy -D warnings`、`npm run build`、`test:agent-workbench` 等既有门禁；
"不新增失败"（§2 建议的核心验收口径）无处落地。

**建议**（归属：贡献分支可提交骨架，门禁阈值交上游确认）：

1. 新增 `.github/workflows/ci.yml`，至少三个 job：
   - `frontend`：`npm ci` → `npm run build`（vue-tsc + Vite）→ `npm run test:ui`。
   - `rust-lint`：`cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`。
   - `rust-test`：`cargo test --lib -- --test-threads=1 --skip <活网络/规模黑名单>`，
     与 §2 的基线脚本共用跳过清单。
2. 容器相关测试（需钉死 digest 镜像）在 CI 标记为可选 job 或 `#[ignore]`，
   避免 CI 环境无镜像时的设计内 coverage gap 误报为失败。
3. lockfile 唯一化后（见 §4），CI 用 `npm ci` 强约束锁文件。

---

## §2 回归基线未固化（P0，可在贡献分支推进脚本）

**现状** 〔沿用交接 §A/§B〕：全量 `cargo test`（跳过活网络/规模组）为
**3066 passed / 318 failed / 7 挂起**；交接已在 base 提交 `8fddee2` 复测抽样，
确认失败簇为**存量债**，非本分支引入。但目前：

- 没有一份机读的**基线失败清单**；
- 没有脚本能回答"这次提交相对 base 是否**新增**了失败"。

**影响**：当存量红高达 300+，人眼无法区分"老红"和"新红"。任何重构都可能
悄悄引入回归而被淹没在既有失败里。交接文档本身也建议"先固化基线数"。

**建议**（归属：贡献分支）：

1. 在 `tools/` 下新增 `regression_baseline.mjs`：跑指定测试集，解析输出，
   与提交进仓库的 `docs/regression-baseline.json`（失败用例全名集合）对比，
   **仅当出现基线外新失败时非零退出**。
2. 活网络测试（GitHub 匿名搜索、公共文档抓取）加**显式超时或环境开关**
   （如 `OVIRAPTOR_LIVE_NET_TESTS=1` 才启用），消除交接 §A 记录的"网络抖动即无限等待"。
3. 规模测试（10 万/20 万行对账）标 `#[ignore]`，由门禁显式选择执行。
4. **边界**：上述活网络/规模测试多在 exposure 冻结区，按交接黑名单须在
   **独立会话**处置，不得混入执行链交付——本审计只记录，不动手。

---

## §3 缺少 LICENSE（P1，需上游决策授权条款）

**现状** 〔实测〕：仓库根目录无 `LICENSE*` 文件。

```bash
ls LICENSE*   # => no matches
```

**影响**：这是一个"面向授权资产管理场景"的安全工具，却没有任何分发许可、
免责声明或使用边界条款。对一个会执行采集、探测、沙箱内解析的工具而言，
缺少授权与免责声明既有法律风险，也与其"授权使用"定位自相矛盾。

**建议**：由仓库所有者（上游 swyiic）选定许可证并补 `LICENSE`，
同时在 README 增加"仅限授权目标/合规使用"的显式声明段落。

---

## §4 工具链与依赖的确定性（P1，部分可在贡献分支推进）

**现状** 〔实测〕：

- **双 lockfile 并存**：`package-lock.json`（59 KB）与 `yarn.lock`（36 KB）同时存在，
  且 `yarn.lock` 的修改时间更新。README 指引用 `npm install`，但两套锁共存会导致
  不同开发者/CI 解析出不同依赖树。
- **无 Rust 工具链钉版**：无 `rust-toolchain.toml`。而 clippy 的告警集合随版本漂移
  （当日交接专门清偿了"clippy 1.99"的 26 条告警）；无钉版意味着换一台机器跑 clippy
  可能红/绿不一致。
- **edition 2021**：`src-tauri/Cargo.toml` 仍为 `edition = "2021"`（截至 2026-01，
  2024 edition 已可用；升级非必须，但值得纳入技术债清单）。
- **发布来源绑定未验证**：README「macOS 安装产物核对」与
  `build_receipt.mjs` 自述明确 `sourceBinding` 恒为 `not_verified`——
  当前不存在签名、hermetic 或可复现构建证明。

**影响**：依赖与工具链的不确定性直接削弱 §1 CI 的可信度（CI 绿不等于本地绿），
也让"构建回执"链条止步于"本地观察"而非"可信来源证明"。

**建议**：

1. 〔贡献分支〕二选一删除一份 lockfile（README 既然用 npm，建议保留
   `package-lock.json`、删除 `yarn.lock`，并在 `.gitignore` 或 CI 约束防止再生）。
2. 〔贡献分支〕新增 `rust-toolchain.toml` 钉死 channel 与 components（含 clippy、rustfmt），
   使 clippy 门禁跨机一致。
3. 〔上游验收〕签名 + 可复现/hermetic 构建是 Stage 12 发布验收的前置，仍属上游职责；
   本审计只标注其为"发布可信度的关键缺口"。

---

## §5 文档漂移与索引缺失（P1，可在贡献分支推进）

**现状** 〔实测〕：

- README **死链**：正文链接 `[`docs/architecture.md`](docs/architecture.md)`，
  但该文件不存在：

  ```bash
  grep -oE '\]\(docs/[^)]+\)' README.md | ...   # architecture.md => MISS
  ```

  实际存在的架构文档是 `docs/NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md`、
  `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md` 等，README 未指向它们。
- **文档体量失控**：〔实测〕`docs/` 下 305 个文件，以 `NEST_*_AUDIT_YYYY-MM-DD.md`
  为主，高度重叠、时间跨度大、无统一入口索引。新接手者很难判断"哪份是现行有效、
  哪份已被取代"。

**影响**：README 是项目门面，死链直接降低可信度；305 份无索引文档使知识
实际上不可检索，审计结论容易彼此矛盾而无人察觉。

**建议**（归属：贡献分支）：

1. 修复 README 死链：要么补写 `docs/architecture.md`，要么把链接改指向现存的
   `NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md`。
2. 新增 `docs/INDEX.md`（或 README 增设"文档地图"段）：按主题分组列出**现行有效**
   文档，并对已被取代者标注 superseded-by，给出唯一权威阅读顺序
   （DESIGN > SPEC > ARCHITECTURE，与交接链一致）。
3. 约定审计文档的生命周期：过期审计移入 `docs/archive/` 或加 front-matter 状态标记，
   避免 305 → 更多的无边界增长。

---

## §6 代码健康（P2，逐项可在贡献分支推进）

本节均为 **〔实测 2026-10-12〕**。这些不是 bug，是**可维护性**与**健壮性**的技术债。

**超大文件**（单文件职责过载，改动风险与 review 成本高）：

| 文件 | 行数 |
|---|---|
| `src/components/SentinelBoard.vue` | 3553 |
| `src-tauri/src/commands/ownership_review.rs` | 4214 |
| `src-tauri/src/db_schema.rs` | 2339 |
| `src-tauri/src/commands/ownership_directory.rs` | 2315 |
| `src/components/ConfigDialog.vue` | 1235 |

> 近期 git 历史已有 `Refactor: split the fuse zone out of SentinelBoard.vue` 等拆分动作，
> 方向正确；建议把 `SentinelBoard.vue`（仍 3553 行）与 `ownership_review.rs` 列为后续
> 拆分目标。

**错误处理**：

- 非测试 Rust 代码中含 `.unwrap()` 的行 **> 1184**（`grep -rn` 下界，实际 occurrence 更多）。
  其中很大比例是 `.lock()/.read()/.write().unwrap()` 的锁中毒 unwrap（属常规写法），
  但也存在解析/DB 路径上的 unwrap，值得专项梳理哪些应改为 `?` 传播或显式降级。
- 非测试代码 `panic!/unreachable!` 22 处、`.expect(...)` 98 处——建议审查其
  触发条件是否真的是"不可能"，避免生产路径 panic。

**死代码与屏蔽**：

- `#[allow(dead_code)]` 106 处、`#![allow(...)]`/`#[allow(...)]` 合计 199 处。
  交接已承认 `homogeneous/mod.rs` 仍带 `#![allow(dead_code)]`（"有 API 无调用方"）。
  建议把 allow 列为"待核减"清单，随触达各模块时逐步收口，而非长期屏蔽。

**unsafe**：

- 非测试代码 `unsafe` 相关行 112。建议为每个 `unsafe` 块补上
  `// SAFETY:` 不变量说明（Rust 社区惯例），并集中到一处文档说明平台 FFI
  （状态栏/图标等）的安全前提。

**建议**：以上均为渐进式清偿，优先级低于 §1–§5；可在触达相关模块的贡献分支
"顺带核减"，不单独立项大重构。

---

## §7 前端测试形态单一（P2，可在贡献分支推进）

**现状** 〔实测〕：前端 5 个 devDependencies 中无 Vitest/Jest/Playwright/Cypress；
所有"前端测试"是 `tools/*.cjs`，用 Node 内置 `node --test` 驱动 DOM 字符串断言
（如 `test:ui` 串联 70+ 个此类文件）。`src/` 下 156 个 `.vue`/`.ts`、约 3 万行，
**无组件级渲染测试、无端到端浏览器测试**。

**影响**：DOM 字符串 harness 能覆盖结构/文案，但无法覆盖真实渲染、响应式状态、
用户交互路径与跨组件集成；Vue 组件回归主要靠人工点测。

**建议**（归属：贡献分支，需上游认可方向）：引入 Vitest + `@vue/test-utils`
做组件级测试，对核心交互链（配置方案保存、任务编排、资产查询）补 1–2 条
Playwright E2E。现有 `.cjs` harness 可保留为补充层。

---

## §8 已登记的存量业务债（P2，多数属冻结区/上游验收）

以下均已在专项文档登记，本审计仅收敛引用，**不重复处置**：

1. **Exposure 三高危**〔沿用交接 §C.1〕：U-01 复核面板跨项目写入、
   U-02 `review_scoped_cas` 不校验归属、U-04 观察 URL 凭据参数原文入库，
   另有 38 条未完善。**冻结区，须独立会话处置**。
2. **legacy Strix 物理删除未收口**〔沿用交接 §C.2〕：
   `result_ingestion_runs.rs` 旧 marker 删除语义、`SentinelVulnerabilitiesPane.vue`
   `stage==='strix'` 分支、约 46 文件活/字面量混合待逐项审。
3. **真实旧数据清理半程**〔沿用交接 §C.3〕：CAS 59 对象/37 bundle 未证明旧数据独占不删；
   REM-A13 归属盘点/备份/迁移未做。
4. **REM 横切项**〔沿用交接 §C.4〕：A02/A06/A07/A09/A10/A14 仍 active。
5. **Stage 12 发布验收未启动**〔沿用交接 §C.6〕：真实模型验证（有效 DeepSeek flash 凭据）、
   独立安装 App 端到端、跨平台（Windows/Linux）声明——均属**上游 main 维护者验收职责**。

---

## 收尾：建议的推进顺序

1. **先立 CI 骨架（§1）+ 回归基线脚本（§2）**：这两项一旦落地，后续所有改动
   都有机器兜底，是其他一切清偿的前提。
2. **补 LICENSE（§3）+ 修 README 死链与文档索引（§5）+ 唯一化 lockfile / 钉 Rust 工具链（§4）**：
   低风险、高回报的工程卫生。
3. **代码健康（§6）与前端测试层（§7）**：随触达模块渐进推进，不做一次性大重构。
4. **存量业务债（§8）**：严格按冻结区边界与上游验收职责分工，逐簇独立立项。

本审计不改任何代码，不宣布任何验收通过；所有"完成"判定仍以交接文档点名的
文档链与上游维护者为准。
