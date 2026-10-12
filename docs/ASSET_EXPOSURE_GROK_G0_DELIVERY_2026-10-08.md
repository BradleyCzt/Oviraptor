# Asset 暴露面 Grok G0 交付：基线（实测能力矩阵、共享依赖、P1 复核与修复）— 2026-10-08

- 执行者：Grok
- 仓库：`/Users/swyiic/Desktop/Rust/Oviraptor`，HEAD `14a7875`（开始和结束都已核对，未 commit/push/stash/reset/checkout/merge）
- 编译与测试：在隔离副本 `~/.ovi_o3/speccopy` 中进行（rsync，排除 target/node_modules/.git），`CARGO_TARGET_DIR=~/.ovi_o3/spec_target`。阶段结束后两者均已删除（见 §5）。
- 前端测试：在真实仓库运行（只读 node_modules，不写数据库）。
- 真实用户库 `/Users/swyiic/oviraptor/oviraptor.sqlite3` 全程未打开。

## 1. 用户行为与生产调用链

G0 改动只影响四条既有链路，没有新增 IPC。

**P1-1 保存配置**
- 设置面板 `ExposureSettingsPanel.vue` → `useExposureSettings.save` → `api.saveExposureSettings` → IPC `save_exposure_settings` → `save_exposure_settings_in` → `settings::save`。
- `settings::save` 用一个 IMMEDIATE 事务完成全部步骤：
  - 读当前 `version`
  - 与编辑器读到的 `expected_version` 比对；不一致返回 `[exposure_setting_conflict] 当前版本是 vX，编辑器读到的是 vY，已拒绝覆盖`
  - 把旧行归档进 history
  - CAS upsert（`WHERE exposure_settings.version=?`），写入行数不为 1 也判为冲突
  - 提交
- 用户可见效果：两个窗口同时保存同一版本，只有一个成功；另一个看到冲突提示，需要刷新后重新编辑。

**P1-2 版本历史**
- 面板 → IPC `exposure_setting_versions` → `exposure_setting_versions_in` → `settings::history_window(limit)`。
- 实现：读 limit+1 行，`truncated` 是真实计算结果；limit 被夹在 1..200。
- 单行解析失败时，该行显示 `corrupt:true`、`settings:null`，原字节留在库里。
- 截断时面板显示"只显示最近 N 个版本，更早的版本未列出（不是完整历史）"；损坏行显示"该版本内容无法解析，原字节保留在历史中"。

**P1-3 结构与损坏配置**
- 所有暴露面命令都先经过 `exposure_connection`，即 `db::open` 加 `ensure_exposure_schema`。
- 结构不可用时返回 `[exposure_schema_unavailable] 暴露面结构不可用，本次未读写任何暴露面数据：…`，不再 `.ok()` 吞掉后继续读写。共替换 13 处。
- 已保存配置损坏时：
  - 查看配置返回 `[exposure_settings_corrupt]`
  - `start_exposure_scan` / `start_exposure_retest` 拒绝启动，不再用默认值扫描
  - 用户可以按当前版本重新保存一份有效配置来修复（测试已覆盖）

**P1-4 旧复核入口**
- 发现列表的"复核…"按钮调用 `openReview(id)`，切到复核整改面板，走 `review_exposure_finding_scoped`（带 projectId、队列版本、操作者和依据）。
- 页面和 composable 已不再引用无版本的 `review_exposure_finding`。暴露面私有 api.ts 的导出也已删除。
- lib.rs 注册和共享 `src/features/assets/api.ts:66` 不在白名单内，已写成 IR-01。

## 2. 文件、SHA、白名单检查、依赖

**改动文件（SHA-1 为结束时的值，全部在白名单内）**

| 文件 | SHA-1（前 10 位） | 改动 |
|---|---|---|
| src-tauri/src/commands/exposure.rs | 6e0bf24aa5 | `exposure_connection`/`exposure_schema_ready`；替换 13 处 `.ok()`；`exposure_scan_settings` 返回 Result；新增 `*_in` 连接级函数；`mod tests_g0` |
| src-tauri/src/commands/exposure/settings.rs | fa8a5908d3 | IMMEDIATE 事务 CAS；`history_window`；`SettingVersionRow` 类型别名；corrupt 显式报错 |
| src-tauri/src/commands/exposure/tests_e2..e5.rs | 96c8420118 / a898106135 / bc7a1ce18f / 4245551102 | helper 适配 Result（`.expect`），不改断言 |
| src-tauri/src/commands/exposure/tests_g0.rs（新） | a0b8315917 | 6 个 G0 测试，256 行 |
| src/components/ExposureSurface.vue | 7dd242ff4b | 三个快捷写状态按钮改为"复核…"打开 scoped 面板 |
| src/features/assets/exposure/useExposureSurface.ts | c689ac88cd | 删除 `review()` 和旧入口依赖 |
| src/features/assets/exposure/api.ts | d9d96dd5f7 | 删除 `reviewExposureFinding` 导出；历史项类型加 `corrupt`、`limit` |
| src/features/assets/exposure/ExposureSettingsPanel.vue | a441c1cf68 | 截断提示、损坏行提示 |
| tools/test_asset_exposure_surface.cjs / page.cjs / settings.cjs | 7f96543ddf / 6a13746dcf / 3f5454ba86 | 新增 3 个前端测试 |
| docs/ASSET_EXPOSURE_GROK_G0_DELIVERY_2026-10-08.md、docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md | — | 本文与集成请求 |


**白名单与并发写入核对**
- G0 开始时对 57 个白名单文件做了指纹 `~/.ovi_o3/g0/whitelist_t0.sha`。
- 结束时差异恰好是上表 13 个文件，加上新文件 tests_g0.rs，没有任何非本人修改。
- 未触碰：归属文件、db_schema.rs、commands.rs、lib.rs、db.rs、Native/agent_*、assets.rs、asset_export.rs、App.vue、package/Cargo 文件、共享 PSL、Ownership* 组件、Master/progress 文档、规格原文。
- `git status` 里 agent_runtime 等文件的修改属于 Codex 的独立任务，与本片无交集。
- 曾观察到两个疑似 Qoder 的 python/node 进程（启动于 2026-10-07 17:12 和 2026-10-08 08:03，父进程为 launchd），核实为挂起的旧 `node --test` 孤儿进程：
  - evidence.cjs 的 mtime 是 15:20，与本片无关
  - surface.cjs 的最后写入是本人 22:18:07
  - 结论：无冲突，按规则未 kill

**共享依赖表（只读复用，不复制、不修改）**

| 依赖 | 来源 | 暴露面可达方式 | G0 实测 | 用途 |
|---|---|---|---|---|
| 公共后缀解析 | `crate::commands::ownership_domain_parts`、`OWNERSHIP_PSL_VERSION` | exposure 由 commands.rs `include!`，是 commands 的子模块，可直接调用私有项 | 可调用，版本 `psl-2026-10-07_07-28-19_UTC-3929462` | G1 替换 `clusters::registered_domain` |
| 组织目录 | `org_directory_version`、`load_org_entities` | 同上 | 可调用，只读 | 单位投影、版本号 |
| 项目→单位绑定 | `load_project_org_binding`（`project_org_bindings`） | 同上 | 可调用 | AssetUnitProjection 的项目级边 |
| 资产→单位逐条边 | 不存在（`asset_ownership_decisions` 无 entity 列） | — | 缺失 | IR-02；G1 其余资产标 `unassigned` |
| 暴露面表 | `exposure/schema.sql` + `state.rs::ensure_exposure_schema` | 本模块 | 结构失败现在显式报错 | 全部 |
| GitHub 凭据 | 无秘密引用端口 | — | 缺失 | IR-04（G2） |

可见性：G0 不需要改任何范围外文件就能引用上述函数。这依赖 include! 结构，所以 IR-03 请求把它们升为声明过的 `pub(crate)` 端口（不阻塞）。

**实测：旧聚类键与共享 PSL 的差异**

| host | 旧 `clusters::registered_domain` | 共享 PSL | 结论 |
|---|---|---|---|
| team.github.io | github.io | team.github.io | 旧实现把所有 github.io 站点聚成一簇（私有后缀被忽略） |
| a.b.gov.cn | gov.cn | b.gov.cn | 旧实现把所有政府站聚成一簇 |
| shop.example.co.uk | co.uk | example.co.uk | 旧实现按公共后缀聚类 |
| www.zj.10086.cn | 10086.cn | 10086.cn | 一致 |

这是 G1 的数据正确性问题，G0 只做测量，不改聚类逻辑。

## 3. 来源能力矩阵（实测）

| 来源 | 已实现 | 已注册（start/resume） | 生产可达 | 凭据 | 外部实测（2026-10-08 约 22:14 UTC+8） |
|---|---|---|---|---|---|
| passive_web（站点被动访问） | 是 | 是 | 是 | 无 | 未对真实目标扫描（G0 不发起扫描）；仅单测和夹具覆盖 |
| certificate_transparency（crt.sh） | 是 | 是 | 是 | 无 | crt.sh 200 |
| web_archive（Wayback CDX） | 是 | 是 | 是 | 无 | Wayback 200 |
| GitHub 仓库元数据 | 否 | 否 | 否 | 匿名可用 | `search/repositories` 匿名 200 |
| GitHub 代码搜索 | 否 | 否 | 否 | 需要 token | `search/code` 匿名 401 → IR-04 |
| document_import / document_link | 否 | 否 | 否 | — | — |
| public_intel_import | 否 | 否 | 否 | — | — |

**限额与设置的真实性（G0 测量，G1-0 修复）**

面板上的部分设置没有被执行，或者与执行值不一致：
- `concurrency` 在面板上标为"并行度"，但编排器没有使用，实际是串行访问。
- `ct_page_size` 没有使用。
- CT 域名上限没有强制执行。
- 访问队列上限：代码写 40，设置写 80。
- Wayback 条数：代码写 200，不随设置变化。
- 另外，`load_plan(...).ok()` 和 `from_frozen` 在出错时静默用默认值（T40–42）。

按规格的"不显示不真实能力"原则，这些都归入 G1-0。

## 4. 测试命令、结果、计数、重复

| 阶段 | 命令 | 结果 |
|---|---|---|
| 基线（未修改副本） | `cargo test --lib exposure` | 95 passed，0 failed（44.08s） |
| 修复后 | 同上 | 101 passed（新增 6 个 g0 测试） |
| g0 重复 | `cargo test --lib tests_g0` ×5 | 每次 6/6，用时 0.73–0.83s |
| 变异后最终（clippy 修正后再跑） | `cargo test --lib exposure` | 101 passed，0 failed（36.35s） |
| clippy | `cargo clippy --lib --tests` | exit 0。共 70 个 warning，与基线相同；暴露面文件 0 个（中途 2 个已修：类型别名、`expect_err`） |
| 前端 | `npm run test:asset-exposure` | 62/62 pass |
| 类型 | `npx vue-tsc --noEmit -p tsconfig.json` | exit 0 |

**G0 新测试（含负向）**
1. `g0_two_connections_saving_the_same_version_write_exactly_once`：12 轮，每轮两个连接用 barrier 同时保存同一版本，恰好一个成功，另一个得到 `[exposure_setting_conflict]`，history 每轮只多一行。
2. `g0_a_save_behind_a_held_write_lock_rereads_and_conflicts`：另一连接持有写锁并提交后，被阻塞的保存重新读取版本，然后冲突。
3. `g0_history_window_reports_real_truncation_at_the_boundary`：251 次保存；limit 200 / 199 / 500 三个边界；损坏行显示为 corrupt；空项目返回 `truncated=false`。
4. `g0_schema_failures_are_reported_and_existing_data_is_kept`：两种注入方式（用 VIEW 替换表；全新库加 `query_only`）都返回 `[exposure_schema_unavailable]`，已有数据保持不变。
5. `g0_corrupt_saved_settings_refuse_view_and_scan_but_can_be_repaired`：查看、扫描、复测都拒绝；按当前版本重新保存后恢复。
6. `g0_shared_psl_and_directory_are_reachable_read_only`：输出 `G0_MEASURE` 行（§2 的表格）。

另有前端 3 个测试：旧入口零调用；列表打开 scoped 面板；截断事实和损坏行不被吞掉。

**变异（12 个，11 个被杀死）**
- 方法：在副本中逐一注入变异，跑全量 exposure 测试或对应的 cjs 文件，然后用 `copy2` 加 `utime` 恢复，并核对 SHA。
- 每次都确认真实仓库没有被改动。

| ID | 变异 | 结果 |
|---|---|---|
| M01 | 设置保存事务改为 DEFERRED | 红：2 个 g0 并发测试 |
| M02 | 删除事务内的版本检查 | 红：4 个测试，含原有的 `saving_is_versioned_and_a_stale_editor_cannot_overwrite` |
| M03 | 删除 CAS 的 `WHERE version=?` | 存活。等价变异：IMMEDIATE 事务加事务内检查已经保证同样的语义，WHERE 是纵深防御，保留 |
| M04 | `truncated` 改为常量 false | 红 |
| M05 | 只读 limit 行 | 红 |
| M06 | 吞掉结构错误 | 红 |
| M07 | `exposure_connection` 跳过结构检查 | 红 |
| M08 | 扫描出错时回退默认配置 | 红 |
| M09 | 损坏 JSON 当作默认值 | 红 |
| M10 | composable 恢复旧复核入口 | 红（surface.cjs） |
| M11 | 列表按钮直接写状态 | 红（page.cjs） |
| M12 | 面板隐藏截断提示 | 红（settings.cjs） |

日志在 Mac 的 `~/.ovi_o3/logs/g0/`：base / after1 / g0_tests / repeat / mut / final / final2 / clippy / clippy2。

## 5. 规模、性能与真实库声明

- 规模：历史测试写入 251 个版本，单次读取上限 200；并发测试 12 轮。全部在临时库中完成。
- 性能：g0 测试 0.73–0.83s；全量 exposure 测试 36–44s。
- 真实库：未打开 `/Users/swyiic/oviraptor/oviraptor.sqlite3`，测试全部使用 tempdir 库，也没有发起真实扫描。
- 磁盘：

| 位置 | 删除前 | 删除后 |
|---|---|---|
| `~/.ovi_o3/spec_target` | 6.6G | 已删除 |
| `~/.ovi_o3/speccopy` | 36M（node_modules 只是指向真实仓库的符号链接，只删了链接） | 已删除 |
| `~/.ovi_o3` 合计 | 6.7G | 1.2M（日志、备份、指纹） |
| 真实仓库 `src-tauri/target` | 12G | 按规则未清理（Codex 在用） |

## 6. 四类证据

1. **代码证据**：§2 的文件与 SHA；`settings::save` 的 IMMEDIATE 事务加 CAS；`history_window`；`exposure_connection`；前端 `openReview`。
2. **测试证据**：基线 95 → 101 全绿；6 个 g0 测试重复 5 次全绿；前端 62/62；vue-tsc exit 0；clippy 在暴露面文件中 0 warning。
3. **负向与变异证据**：冲突、截断、结构失败、损坏配置都有负向断言；变异 11/12 被杀死，M03 是等价变异，已说明。
4. **外部与实测证据**：crt.sh 200、Wayback 200、GitHub repo 搜索匿名 200、code 搜索匿名 401；旧聚类键与共享 PSL 的差异表；白名单指纹核对；陈旧进程核实记录。

## 7. 下一阶段（G1 计划）

P1 已在 G0 修完，G1 不再把 P1 当作第一片。IR-01 的 lib.rs 退役由主控装配。

1. **G1-0 设置和限额的真实性**
   - concurrency 要么实际生效，要么在面板上改成"顺序访问（并行未实现）"。
   - 执行 `ct_page_size` 和 CT 域名上限。
   - 访问队列和 Wayback 条数改为读取设置。
   - `load_plan(...).ok()` 和 `from_frozen` 出错时显式报错，不静默用默认值。
   - 每项有负向测试和变异。
2. **G1-1 聚类改用共享 PSL**：用 `ownership_domain_parts` 替换 `clusters::registered_domain`，记录 `psl_version`；为旧聚类键补迁移或重算说明。测试至少覆盖 §2 的四个 host。
3. **G1-2 AssetUnitProjection**：只用项目级 `project_org_bindings` 加 `load_org_entities`，其余资产标 `unassigned`，不从候选组或地名推断。需要新 IPC 的，在 IR-06 登记，前端显示 `integration_pending`。
4. **G1-3 分类与招标排除**：按规格做站点分类，把招标采购类页面排除出资产暴露面，规则版本化，并有反例测试。
5. **G1-4 闭环**：站点 → 观察 → 发现 → 复核（scoped）→ 整改 → 复测，端到端测试，覆盖版本冲突和损坏数据的负向路径。

每片都用独立的 `CARGO_TARGET_DIR`，结束后删除，交付文档按本文七节结构编写。
