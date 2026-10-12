# Asset 暴露面 Q0–Q6 交付记录（2026-10-07 起，2026-10-08 补 Q2–Q6）

任务书：`docs/NEST_ASSET_EXPOSURE_QODER_PLAN_2026-10-07.md`，本轮只做 §11 指定的首片
Q0（页面真实性）+ Q1（只读分页合同/仓储/汇总与测试）。范围外内容一律未动。

开工前核对：HEAD `14a78753b95aad65e8fd8611048e6d6ff8ddb9a2`，分支
`nest/master-checkpoint-2026-10-05`，工作树 273 个未提交条目（其他代理在开发财务链）。
任务书附录六个文件哈希逐一比对，全部一致，无漂移。

## 1. 文件增量

| 文件 | 状态 | 说明 |
| --- | --- | --- |
| `src-tauri/src/commands/exposure.rs` | 修改（+90） | 命令门面：新增 6 个只读命令，`exposure` 内核模块声明；原有 `list_exposure_findings` 等旧命令与返回形状未改 |
| `src-tauri/src/commands/exposure/contract.rs` | 新增 | 页长/枚举/游标校验、`ExposurePage` 信封、LIKE 转义、7 项已知上限 |
| `src-tauri/src/commands/exposure/repository.rs` | 新增 | findings/runs 的 keyset 分页、真实 total/hasMore、`expectedProjectId` 复验 |
| `src-tauri/src/commands/exposure/summary.rs` | 新增 | 与采集谓词同条件的范围汇总 |
| `src-tauri/src/commands/exposure/tests.rs` | 新增 | 7 条临时 SQLite 正反测试 |
| `src/features/assets/exposure/contracts.ts` | 新增 | TS 合同：页校验、错误码提取、错误文本脱敏、旧接口窗口文案 |
| `src/features/assets/exposure/useExposureSurface.ts` | 新增 | 读取状态机：generation 隔离、stale 保留、轮询门控、真实资产分页、旧接口降级 |
| `src/components/ExposureSurface.vue` | 修改 | 按钮改“开始被动采集”、显示范围与上限、数据源跟随所选任务、线索“加载更多”、stale 提示 |
| `src/features/assets/api.ts` | 修改（追加） | 6 个新 IPC 门面方法，旧方法未动 |
| `src/exposure-surface.css` | 修改（追加） | stale/limits/gap/分页按钮样式 |
| `tools/test_asset_exposure_surface.cjs` | 新增 | 12 条真实 composable + 模拟 IPC 测试 |
| `tools/test_asset_exposure_page.cjs` | 新增 | 3 条真实 SFC 编译渲染测试 |

禁碰清单核对：`lib.rs`、`commands.rs`、`db.rs`、`db_schema.rs`、`db_initialize_columns.rs`、
`models_*.rs`、`src/types.ts`、`src/api.ts`、`package.json`、`jobs*`、
`agent_runtime/**`、`multi_agent/**`、`native_*` 全部未修改。未运行全库 `cargo fmt`、
未运行 `cargo clean`、未 commit、未 push；只对上述自有文件跑 `rustfmt`。

## 2. 事实分类

真实功能（有测试、有生产调用路径）
- 只读分页读取：`page_exposure_findings`、`page_exposure_runs`、
  `list_exposure_source_results_for_project`、`read_exposure_finding`、
  `exposure_scope_summary`、`exposure_known_limits`。keyset 游标 + id tie-break，
  筛选后真实 total；跨项目 id 与非法游标/页长/未知枚举显式拒绝。
- 前端读取状态机：项目/run 双维度代次隔离、失败保留上次可信快照并标 stale、
  隐藏或卸载后停止轮询、绑定资产按 100 一页真读且不足即标截断、
  数据源按所选 run 加载、线索“加载更多”。

需要主控串行集成的骨架
- 6 个命令尚未进 `invoke_handler`（`lib.rs` 属禁碰文件）。在此之前页面走
  旧接口降级路径并显示“旧接口上限：线索最多前 500 条、任务最近 20 条、单页资产最多
  200 条”，集成后自动切到真实分页（已由测试锁住两种模式）。
- 新测试未注册进 `package.json` 门禁（同为禁碰文件）。

只读诊断（本轮未做，按任务书属于 Q2/Q3/Q4/Q5）
- 采集侧的 `LIMIT 200`、每资产 20 次成功读取、CT 20 域、Wayback 每域 200 行仍在，
  现在只是被如实报告，未被消除；`start_exposure_scan` 的 count→insert→spawn 仍非原子；
  无持久工作项、无 resume；CT 结果仍不落线索；`exposure_findings` 仍无 run_id/observation
  关联；fingerprint 仍把整条 URL 小写；Source Map 仍不解析 JSON。
- 授权细粒度合同（scope/版本/期限/撤销/逐跳重验）未实现，属 Q2。

## 3. 门禁结果

| 命令 | 结果 |
| --- | --- |
| `rustfmt --edition 2021 --check`（仅自有 Rust 文件） | 通过 |
| `nice -n 15 cargo test -j 1 --lib -- --test-threads=1 commands::exposure` | 14 passed / 0 failed，exit 0 |
| `cargo clippy --lib --all-features` 中与 `commands/exposure` 相关的告警 | 0 条；该文件集无新增告警 |
| `npx vue-tsc --noEmit` | 无错误 |
| `npm run build` | exit 0 |
| `node --test --test-concurrency=1 tools/test_asset_exposure_surface.cjs tools/test_asset_exposure_page.cjs` | 15 passed / 0 failed |
| `node --test tools/test_asset_log_panel.cjs`（同门面回归） | 3 passed / 0 failed |
| `git diff --check`（自有文件） | 干净 |

基线债（不属本卡，未修改）：`cargo clippy` 在
`agent_runtime/multi_agent/budget*`、`commands/agent_native/coordinator_rust_policy.rs`
等文件有 dead_code 告警；`git diff --check` 在
`src-tauri/src/commands/agent_tests_coverage.rs`（EOF 空行）与
`src-tauri/src/commands/agent_tools_http/surface_discovery.rs`（行尾空格）报错。
按任务书 §8.7 如实区分，未删测试、未放宽 allowlist。

## 4. 交主控的集成补丁

`src-tauri/src/lib.rs`，在 `commands::export_exposure_findings,`（现第 376 行）之后插入：

```rust
            commands::page_exposure_findings,
            commands::page_exposure_runs,
            commands::list_exposure_source_results_for_project,
            commands::read_exposure_finding,
            commands::exposure_scope_summary,
            commands::exposure_known_limits,
            // Q3：按冻结计划继续（run_id + expected_project_id 双条件）
            commands::resume_exposure_scan,
            // Q4：外部索引带回的待归并队列，只读分页
            commands::page_exposure_pending_discoveries,
            // Q5：一条线索的证据包（观察 / 版本 / 需复核），只读且已脱敏
            commands::read_exposure_evidence,
```

注册后可移除 `src-tauri/src/commands/exposure.rs` 里那六个
`#[allow(dead_code)]`（每个都带同一句注释）。

`package.json` 增加独立门禁并在 UI 聚合脚本里引用：

```json
"test:asset-exposure": "node --test --test-concurrency=1 tools/test_asset_exposure_surface.cjs tools/test_asset_exposure_page.cjs tools/test_asset_exposure_evidence.cjs"
```

## 5. 未测项（不得当作已完成）

- 未跑真实 FOFA/CT/Wayback/目标站点请求，未打开安装后的 App，未读取用户数据库；
  全部验证使用临时 SQLite、SSR 渲染与模拟 IPC。
（下列四项已在 2026-10-08 的 Q2–Q6 增量中覆盖，见文末对应小节；仍受“未跑真实数据源、未做
真实 kill -9、未测 100,000 规模”的限制。）

- ~~未做测试子进程崩溃恢复与并发启动唯一性（Q3）。~~ → `q3_two_simultaneous_starters_get_one_active_run`、
  `q3_a_crash_after_a_closed_item_does_not_fetch_it_again`、`q3_an_active_worker_is_never_declared_dead`。
- ~~未做 localhost HTTP 出口、redirect 逐跳、撤销与到期（Q2）。~~ → `tests_q2.rs` 8 项。
- ~~未做 observation/finding 分离、Source Map 正式解析、CSV 公式注入转义（Q5）。~~ →
  `tests_q5.rs` + `rules.rs`/`export.rs`/`fingerprint.rs` 内联测试。
- ~~未覆盖工作项化采集与 20,000 规模（Q6）。~~ → `q6_four_thousand_items_run_to_terminal_state_and_twenty_thousand_are_planned`；
  100,000 仍未测。

---

# Q2–Q6 增量（2026-10-08）

范围仍只在本切片自有文件与临时 SQLite / localhost fixture 上；未 commit、未 push、未打开安装后的
App、未读取用户数据库、未跑真实 FOFA/CT/Wayback/目标站点。§7.2 的共享文件一个都没改，需要的
注册与门禁接线在第 4 节以补丁形式交主控。

## Q2 增量文件

- 新增 `src-tauri/src/commands/exposure/scope.rs`：`ExposureScope::load` / `authorize` /
  `authorize_redirect` / `check_addresses` / `as_json`，`ExposureDenial` 稳定码，
  `Exclusion{asset_id,reason}` 说明“为什么这个资产本轮不在范围内”。
- 新增 `exposure/redaction.rs`：唯一脱敏权威 `redact_text` / `redact_json` / `mask_value` /
  `credential_sightings`（`CredentialGrade::{None,Placeholder,Candidate}`）/
  `evidence_excerpt` / `escape_csv_cell` / `content_hash`，版本 `REDACTION_VERSION`。
- 新增 `exposure/adapters/http.rs`：`PassiveClient`（`redirect::Policy::none()`）、
  `RequestMeter`（请求与字节都在 send 之前计费）、`fetch` 逐跳 revalidate+authorize+计费、
  `fetch_source_endpoint`（公共索引端点白名单，命中不代表新目标被授权）、
  `finish` 有界读取 + `boundary ∈ permission|rate_limited|challenge|''`。
- 新增 `exposure/state.rs`、`exposure/schema.sql`、`exposure/transaction.rs`（结构、状态词表、
  聚合与短 IMMEDIATE 事务）。
- 新增 `exposure/fixture.rs`（测试专用 localhost 镜像）、`exposure/tests_q2.rs`（8 项）。
- 修改 `commands/exposure.rs`：把旧的一次性线程采集器
  （`run_passive_exposure_scan` / `collect_public_indices` / `upsert_*` / `exposure_cancelled` /
  `save_exposure_source_result`）删除，改为
  `execute_exposure_run` → `exposure::orchestrator::run_plan`，`start_exposure_scan` 改为
  `start_or_reuse`；`quoted_resource_urls` 与 `masked_sensitive_evidence` 保留但只由
  `exposure_tests` 固定历史解析行为，生产路径不再调用。

Q2 期间由测试暴露的真实缺陷（不是测试迁就实现）：

1. `ExposureGrant::from_url` 原用 `port()` 与 `port_or_known_default()` 比较来判定默认端口，
   而 `url` crate 对显式端口会返回同一个值，导致**所有显式端口被抹掉**、一个 origin 的批准被
   放宽到该 host 的全部端口。`q2_grants_are_origin_exact_...` 抓到后改为只对
   `http:80`/`https:443` 归一。
2. 脱敏正则原先写了 `(?P=quote)` 反向引用与 `x` 宽松标志，`regex` crate 两者都不支持，
   首次运行直接 panic（“脱敏正则必须可编译”）。改为位置化匹配可选引号。
3. 值的字符类过宽，`api_key = ZZZ…</p><p>api_key = YOUR_…` 会一路吞到下一个标签并被误判为
   placeholder；把 `<`、`>` 排除出取值范围，并让 `credential_sightings` 用
   `captures_iter` 逐条返回，模板值与候选值不再同级。

## Q3 增量文件

- 新增 `exposure/work_items.rs`：`plan`（`ON CONFLICT(run_id,work_key) DO NOTHING` 幂等）、
  `claim_next`（短 IMMEDIATE 事务内选候选 + 带 `status IN ('pending','orphaned')` 守卫的认领，
  owner/claim_version/heartbeat/attempts 一起写）、`heartbeat`、`finish`（owner + claim_version
  守卫，返回 false 表示失去认领）、`reclaim_expired`（**只看心跳**，不看认领时间）、
  `counts`、`enumerate_gaps`、`record_usage`/`usage`。
- 新增 `exposure/orchestrator.rs`：`start_or_reuse`（一个事务内 count/insert/冻结计划/授权收据/
  工作项，并发冲突由 `ux_exposure_runs_single_active` 兜底并返回同一条 run）、
  `planned_seeds`（**每个已授权 origin 一项**，不再有 `LIMIT 200`；额度耗尽的尾巴成为
  `blocked_quota` 缺口）、`run_plan`（网络只在事务外；证据先落盘、终态后写）、
  `save_source_result`、`finalize`（唯一决定终态的地方）。
- 新增 `exposure/recovery.rs`：`load_plan`（含跨项目拒绝）、`verify_frozen`（scope 哈希或
  options 哈希移动即拒绝复用）、`meter_for`（从已花掉的额度继续）、
  `mark_interrupted_if_dead`（**只判定无法证明仍在跑的 owner**，心跳新鲜的轮次不改写）、
  `legacy_projection`（旧轮次标 `legacy_passive_web`，不回填工作项）。
- 新增 `exposure/adapters/passive_web.rs`：按 origin 的工作项执行体，自然链接 BFS、
  每 origin 尝试上限计**尝试**而非成功、越界链接只记 `reference_out_of_scope` 不发请求、
  redirect 跳出即停并留 `redirect_stopped` 观察、逐来源状态回写。
- 新增 `exposure/tests_q3.rs`（10 项）。

Q3 期间由测试暴露的真实缺陷：

1. `reclaim_expired` 原先额外要求 `claimed_at <= cutoff`，于是刚刚认领、心跳已过期的项永远回收
   不掉；`q3_a_crash_after_a_closed_item_does_not_fetch_it_again` 与
   `q3_an_active_worker_is_never_declared_dead` 双双失败后，改为只按心跳判定。
2. `ItemOutcome` 最初没有回写逐来源状态，导致 `passive_web` 在聚合里被当成“计划来源没有终态”，
   恢复后的轮次只能报 `partial`。补上 `source_status/source_reason/observed`，并在
   `finalize` 里给“有计划但零工作项”的来源写 `unconfigured`。
3. 失去认领的 worker 会先落证据再被拒绝写终态：确认这是正确行为（证据是事实，终态是所有权），
   观察行由 `UNIQUE(run_id,source_key,kind,url)` 天然去重。

## Q4 增量文件

- 新增 `exposure/adapters/certificate_transparency.rs`：crt.sh 查询、名字过滤只保留请求根域
  之内、非 JSON/不可解析显式 `parse_error`、命中 `truncated` 记 `partial`，
  每个名字同时产出一条 `ct_name` 观察和一条待归并记录，**不产出线索、不产出资产**。
- 新增 `exposure/adapters/web_archive.rs`：CDX 解析保留 `indexed_at`，
  `current_accessible` 保持 `None`（索引命中不等于当前可访问），命中条数达到 `limit` 或响应体
  被截断即 `blocked_quota` + 来源 `partial` 并写明“达到 200 条上限”。
- `schema.sql` 新增 `exposure_pending_discoveries`（`UNIQUE(project_id,source_key,kind,
  canonical_key)`，默认 `unreviewed`）；`observations::record_pending` 幂等写入。
- 新增 `exposure/tests_q4.rs`（6 项），含“坏 JSON 不被好来源掩盖”“未配置来源不能报 completed”
  “同名跨轮只留一条待归并且不新建资产”。

## Q5 增量文件

- 新增 `exposure/fingerprint.rs`：`canonical_url` 只归一 scheme/host/默认端口，保留 path/query
  大小写；`finding_fingerprint` 为 v2 键，`legacy_fingerprint` 仅作为**读取别名**用于命中历史行，
  不再写 v1 键，也不改写历史指纹（迁移映射改记在 `exposure_review_events`）。
- 新增 `exposure/observations.rs`：`record`（不可变观察，`UNIQUE(run_id,source_key,kind,url)`，
  写入即带 `content_hash`/`headers_hash`/`rule_version`/`redaction_version`/`boundary`/
  `current_accessible`/`indexed_at`）、`merge_finding`（归并到唯一队列行 + 观察引用 + 版本）、
  `version_finding`（内容或规则移动才追加版本并 `review_needed=1`；不变则不刷版本）、
  `review_needed`、`note_review_events`（人工结论进事件表）、`record_pending`。
- 新增 `exposure/rules.rs`：Source Map **按内容解析**（`Business/ThirdPartyOnly/Empty/NotAMap`，
  看 `version`/`sources`/`sourcesContent`/`mappings`，错误页伪 `.map` 降为 `metadata/low/20`），
  凭据分级（候选 high 62、placeholder low 25，且**绝不做密钥有效性验证**）、
  仓库/对象存储只标“仅外链引用”，内部文档语义命中才出 `document` 线索；`natural_links`
  只跟自然链接并跳过 `data:`/`javascript:`。
- 新增 `exposure/export.rs`：`finding_rows`（逐格 `escape_csv_cell(redact_text(...))`，附
  关联观察数 / 需复核数 / 规则版本 / 脱敏版本）与 `write_csv`；表头同样转义。
  门面里 `export_exposure_findings` 改为调用它，`review_exposure_finding` 增加前态读取与
  `note_review_event` 轨迹写入。
- `exposure/repository.rs` 增加 `exposure_evidence`（按 `expectedProjectId` 校验，返回观察 /
  版本 / 复核事件 / `reviewNeeded`，跨项目报 `exposure_scope_mismatch`）。
- 新增 `exposure/tests_q5.rs`（4 项）+ `fingerprint.rs`、`rules.rs`、`export.rs` 内联测试。

Q5 期间由测试暴露的真实缺陷：`merge_finding` 里对 `exposure_findings.asset_id` 的第五列类型
写错导致编译期就拦下；`exposure_evidence` 多读一列导致 `Invalid column index: 5`；
观察与线索的引用数预期写反（同轮重复归并不应新增引用，跨轮应各留一条）——都按事实改正。

## Q6 增量文件

- 新增 `exposure/clusters.rs`：内容簇（同 `content_hash`）与域名簇（同注册域）分组，代表按
  归属置信度优先、`asset_id` 兜底确定性选出；`registered_domain` 把 IP 视为整体（避免
  `10.0.0.8` 被切成 `0.8` 而把无关主机并成一簇——此缺陷由测试发现）；`inheritance_for`
  明确 `direct` 与 `inherited_from_asset_id`。
- 新增 `exposure/cache.rs`：`CacheKey::as_text`（摘要而非可切分拼接）绑定
  project/scope_version/privacy_class/content_hash/rule_version/kind；`lookup` 计命中，
  `invalidate_content` 只清对应哈希，`invalidate_before_rule` 清旧规则结论。
- 新增 `exposure/cluster_repository.rs`：`save`（一个短事务内替换该簇成员，只删该簇成员行）、
  `read`、`invalidate_inherited_from`（代表被撤销 → 成员转 `stale_representative_revoked`，
  资产行一条不动）、`direct_members`。
- `clusters.rs` 增加 `members_from_refinement_csv` / `clusters_from_refinement`：把
  `jobs/native_refinement.rs` 已经写出的 `certificate_clusters.csv` 与
  `name_domain_clusters.csv`（列名常量按实现抄录）作为**观察输入**接进簇层，不重复实现第二套
  证书匹配规则；例外只有能解析到本项目资产台账的才成为成员，解析不到的一律记 `unmapped`，
  表头列名变化直接报错而不是产出空簇。
- 新增 `exposure/tests_q6.rs`（5 项，含 4,000 项跑到终态 + 20,000 项计划物化与有界分页；
  临时测试库把 `synchronous=OFF` 只为把短事务压进可接受时长，生产不变）。
- 前端新增 `src/features/assets/exposure/useExposureEvidence.ts`（待归并分页 + 证据包 +
  按冻结计划恢复），`contracts.ts` 增 `ExposurePending` / `ExposureEvidence` /
  `readEvidence` / `evidenceHeadline` / `resumeRefusalText`，
  `src/features/assets/api.ts` 增 3 个调用，`src/components/ExposureSurface.vue` 增
  “待归并”标签页、证据抽屉、中断轮次的恢复入口，`src/exposure-surface.css` 增对应样式。
  新增 `tools/test_asset_exposure_evidence.cjs`（9 项）。

## Q7 明确未开工（合同缺失，不是进度问题）

Q7 要求把有界脱敏证据包接进模型单次裁决，并复用 Master 正在开发的 SDK 账单、原授权、
十维预算与 ExternalSurface Root 门禁。这些合同当前不在我可读到的稳定形态上：
`commands/multi_agent_external_surface.rs` 与 `coordinator_external_surface_gate.rs` 属主控禁碰区，
Root 侧仍是 `not_ready`、prepare flag 为 false；财务/预算叶子（`budget/**`、
`native_financial_continuation**`、`store/financial_continuation.rs`）正在被其他代理修改。
按 §7.2 与 Q7 的“当前禁止复制一套 reqwest 模型客户端绕开 A02 或翻 ExternalSurface prepare flag”，
本切片没有新增任何模型调用、没有翻任何旗标、没有自建预算；确定性采集不依赖模型即可运行。
整改链（责任人/期限/工单/复测关联/复开历史）同样未开工：它需要 Q5 的证据引用与主控的
生命周期文件配合，当前 `review_exposure_finding` 只到复核结论 + 轨迹，页面也不得把
confirmed 说成整改关闭。

## Q2–Q6 未测项（不得当作已完成）

- 未跑真实 crt.sh / Wayback / 真实目标站点：CT、归档、被动 Web 全部走 localhost 镜像与离线
  响应，真实限流、真实 challenge 页、真实证书数据规模未验证。
- 真实 SIGKILL 子进程恢复已做（`q3_sigkill_of_a_worker_between_claim_and_close_leaves_the_plan_resumable`：
  同一测试二进制再启一个子进程，认领第二项后由父进程 `child.kill()` 发送 SIGKILL，父进程随后证明
  第一项的观察/线索已提交且不会被重做、未完成项被新 owner 接管、恢复只重发那一项）。仍未做的是
  真机断电级 crash 与真实数据源上的长时重启。
- 未打开安装后的 App、未迁移或读取用户数据库；附加结构只在临时库上创建。
- 100,000 规模未测（只到 20,000 计划物化 + 4,000 终态），也未测真实磁盘/CAS 证据文件。
- 未做用户可编辑的采集额度与来源配置界面：本轮额度、来源集合、内部出口开关都来自代码默认值
  `exposure_scan_settings()`（`json!({})`），测试通过显式传入 settings 批准 localhost 出口。
- Q6 的簇与缓存只建了后端与只读合同，页面还没有簇视图；`exposure_clusters` 由测试写入，
  生产轮次尚未调用 `cluster_repository::save`。

## Q2–Q6 门禁结果（2026-10-08，全部为真实命令输出）

| 命令 | 结果 | 退出码 |
| --- | --- | --- |
| `rustfmt --edition 2021 --check`（自有 Rust 文件：`commands/exposure.rs`、`commands/exposure/*.rs`、`commands/exposure/adapters/*.rs`） | 无 diff | 0 |
| `nice -n 15 cargo test -j 1 --lib -- --test-threads=1 commands::exposure` | `62 passed; 0 failed; 2805 filtered out`（测试二进制共 2,867 项，整棵测试树含本切片改动可编译；本切片文件零告警，其余 warning 全在他人文件） | 0 |
| `nice -n 15 cargo clippy -j 1 --lib --all-features` 中与 `src/commands/exposure*` 相关的告警 | 0 条（先修掉 `format!` 冗余与 `match → ?`，两处 `too_many_arguments` 按本仓库既有写法显式 allow 并写明理由） | — |
| `npx vue-tsc --noEmit` | 无错误 | 0 |
| `npm run build` | 构建成功 | 0 |
| `node --test --test-concurrency=1 tools/test_asset_exposure_surface.cjs tools/test_asset_exposure_page.cjs tools/test_asset_exposure_evidence.cjs` | `24 tests; 24 pass; 0 fail` | 0 |
| `node --test tools/test_asset_log_panel.cjs`（同门面回归） | `9 tests; 9 pass; 0 fail` | 0 |
| `git diff --check`（自有文件） | 干净 | 0 |

测试计数按模块（`commands::exposure` 62 项）：Q1 7 + Q2 8 + Q3 11 + Q4 6 + Q5 4 + Q6 5 = 41 项新增，
加上 `contract`/`state`/`fingerprint` 5 / `rules` 5 内联测试与 `commands::exposure_tests` 原 2 项
（未删、未 skip）。

### 基线债与本次中断（如实记录，不属本卡，未修改）

- 全量 `cargo test -j 1 --lib -- --test-threads=1` 跑到 2,275/约 2,863 项时被我主动停掉，
  目的是把构建锁交还给本卡门禁；因此**全量套件的最终 result 行没有取得**，不能声称全量绿。
- 已观察到的 53 项失败全部落在 `commands::agent_tests::*`（reviewer / coordinator / budget /
  gap replay 等），属其他代理在同一工作树上的在制品；`commands::exposure::*` 在该次运行里
  没有失败记录，且单独跑同一过滤条件为 59/0。
- `cargo clippy` 对其他文件仍报 `src/commands.rs:7 unused import: VecDeque`、
  `src/agent_runtime/multi_agent/budget_gaps.rs:41 unused import`，同样未触碰。
- 共享文件未被本卡修改：`src-tauri/src/lib.rs` 里 `page_exposure_*` / `resume_exposure_scan` /
  `read_exposure_evidence` 等新增命令的出现次数为 0；`lib.rs`、`commands.rs`、`db_schema.rs`、
  `package.json` 的 diff 里没有任何 exposure 相关行（它们的 dirty 状态来自其他代理）。

## Q2–Q6 事实分类

- 真实功能（有测试证据，可在主控注册 IPC 后经界面使用）：Q2 授权与出口合同、逐跳 redirect
  复核、请求/字节预计费、统一脱敏；Q3 冻结计划、原子启动、认领/租约/孤儿回收、按已花额度
  继续的恢复、逐项终态聚合；Q4 CT/归档解析 + 待归并队列 + 逐来源终态；Q5 不可变观察、
  observation/finding 分离与引用、Source Map 内容判定、凭据分级、版本与需复核标记、
  脱敏与公式转义的 CSV 导出；Q6 内容/域名簇、代表与继承标注、缓存键绑定与失效粒度。
- 需集成才对用户可见（当前是已实现但未注册的接口）：`resume_exposure_scan`、
  `page_exposure_pending_discoveries`、`read_exposure_evidence` 三个命令的 `lib.rs` 注册；
  以及 `package.json` 的 `test:asset-exposure` 追加第三个测试文件。
- 只读诊断，不宣称闭环：簇与缓存目前只有后端与测试写入路径，页面没有簇视图，生产采集轮次
  也还没调用 `cluster_repository::save`；证据抽屉只读，不含整改工单/复测关联（属 Q7）。
- 明确未开工：Q7 模型裁决与整改链，理由见上一节；没有复制模型客户端、没有翻
  ExternalSurface prepare flag、没有自建预算。

## Q2–Q6 文件哈希（sha256 前 16 位，交付时点）

```text
288ea9d216cc8b50  src/commands/exposure.rs
6a9c18be117d6034  src/commands/exposure/adapters/certificate_transparency.rs
1309bed62a34437b  src/commands/exposure/adapters/http.rs
5223b72afa3e1b0b  src/commands/exposure/adapters/passive_web.rs
d5e84a3e083360aa  src/commands/exposure/adapters/web_archive.rs
08c4301335f7ab1f  src/commands/exposure/cache.rs
7aff8bfefc6cda11  src/commands/exposure/cluster_repository.rs
4cbf41dd0def3a56  src/commands/exposure/clusters.rs
78f8f8d7aeb44d81  src/commands/exposure/contract.rs
6288d54b2b648a91  src/commands/exposure/export.rs
b4c6459fddcd5912  src/commands/exposure/fingerprint.rs
0fc87f919e7769a5  src/commands/exposure/fixture.rs
1a33af430dad279e  src/commands/exposure/observations.rs
48efc64a03591f4a  src/commands/exposure/orchestrator.rs
1d8e0e50fd8d9ad5  src/commands/exposure/recovery.rs
9e6d261861a5be3d  src/commands/exposure/redaction.rs
5eab76679287319b  src/commands/exposure/repository.rs
ef87156b852c0a7b  src/commands/exposure/rules.rs
6c93d5b2df1cd056  src/commands/exposure/scope.rs
f09e183b0707aafb  src/commands/exposure/state.rs
6133a4b2dcbcd4da  src/commands/exposure/summary.rs
a4ba0b4d8f2fef30  src/commands/exposure/tests.rs
f8b324c788fd2f65  src/commands/exposure/tests_q2.rs
58028eb8bfbc1fee  src/commands/exposure/tests_q3.rs
f566630cb62fb972  src/commands/exposure/tests_q4.rs
dd314e2025fa2f19  src/commands/exposure/tests_q5.rs
c4bd3a6741c0993c  src/commands/exposure/tests_q6.rs
f9fc228a98089f51  src/commands/exposure/transaction.rs
35437d09d8a6da02  src/commands/exposure/work_items.rs
cf518ac42c795e04  src/components/ExposureSurface.vue
60fa86662e980d9f  src/exposure-surface.css
e26ec1ee0c676990  src/features/assets/api.ts
2bdb6b2062e9e34a  src/features/assets/exposure/contracts.ts
4c68c81e7b20a154  src/features/assets/exposure/useExposureEvidence.ts
4cef5195d0953b6c  src/features/assets/exposure/useExposureSurface.ts
```
