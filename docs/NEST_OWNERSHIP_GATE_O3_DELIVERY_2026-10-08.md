# NEST 归属门禁 O3（规模与权限）交付记录 — 2026-10-08

分支 nest/master-checkpoint-2026-10-05，HEAD 14a7875（未提交、未 stash/reset/checkout）。只用临时库，未写真实用户库。
未碰 exposure_* / Native / agent_runtime / assets.rs。assets.rs 保持新口径。

## 状态：完成（2026-10-08 22:15 UTC+8 更新）
- 21:31 第一次变异中途中止。用户确认 Codex 是同仓库里的独立任务，之后的工作都在隔离副本里做：
  - 副本 `~/.ovi_o3/mutcopy`，独立 `CARGO_TARGET_DIR=~/.ovi_o3/mutcopy_target`。
  - 真实仓库只改了归属文件（clippy 修复和加强一个测试），没有在真实仓库里做变异，也没有用真实仓库的 target 或锁。
- 每项变异前后都核对 sha：副本还原后 sha 一致，真实仓库对应文件没变（日志列 `real untouched True`）。

## 实现
1. **增量分析（数据变更水位）**
   - 每次运行都生成新快照。
   - 版本（含 rules_version）、目录或画像变化时强制全量，并记录 mode_reason。
   - 资产或项目资产在水位后变化、人工结论变化、删除，都会重新分类。
   - 基线快照和已提交预览不改。
2. **崩溃恢复**
   - 第一次调用归属 IPC 时接管未完成的运行和操作（惰性启动钩子）。
   - 显示真实已处理数。
   - 未完成的批次不会发布"整组允许"。
   - 不会恢复已撤销的授权。
3. **大组写入**
   - 操作状态机为 applying / blocked / completed / failed。
   - 分块 staging，按 seq 续跑，用 CAS 指纹防止重复写入。
   - 用 ux_ownership_operations_group_active 唯一索引互斥。
4. **组级回退与撤销授权**
   - 回退只恢复归属事实，不恢复 allowed（authorization_not_restored）。
   - 撤销授权分两步：
     - 第一步在一个事务里把状态写成 revoked/0，ExposureScope 立刻拦下游。
     - 第二步才更新显示索引；中断后由恢复补完。
5. **按钮拆分**
   - "确认归属"只写 confirmed/manual/100，不建 grant。
   - "授权"只对人工确认过的行生效，并委托 O0 的 `apply_asset_ownership_decisions`。
   - 向导会列出两步各自的影响。
   - 域规则默认 exact_host。只有人选择时才用 subdomains；IP、公共后缀、共享域名一律拒绝。
   - 路径范围不支持，因为暴露授权是 origin 级。
   - 接入既有合同不需要改门禁以外的代码。
6. **规模**：4000、20000、100000 行验收（见下表）。
7. **前端轮询退避**：有进度 1s；无进度 ×1.6 到 10s；出错 ×2 到 30s。
8. **§12**：只确认归属时，主动扫描仍被拒。

## 测试结果
- `cargo test --lib ownership`（副本，独立 target，最终源码）：test result: ok. 59 passed; 0 failed; 3 ignored; 2896 filtered out
- O3 新测试（commands::ownership_o3_tests）：
  - incremental_snapshot_equals_full_and_never_touches_the_base
  - changes_made_while_an_analysis_runs_are_reclassified_by_the_next_increment
  - rule_directory_and_profile_changes_force_a_full_rerun_with_the_reason
  - startup_recovery_takes_over_an_interrupted_analysis_with_real_counts
  - chunked_group_apply_survives_a_crash_without_double_writes_or_partial_allow
  - recovery_never_restores_a_revocation_made_during_an_unfinished_batch
  - concurrent_applies_and_manual_changes_never_double_write
  - parallel_workers_apply_one_group_exactly_once
  - group_revert_restores_ownership_facts_but_never_authorization
  - revoking_authorization_blocks_downstream_before_the_display_index
  - confirming_ownership_alone_grants_nothing_and_active_scan_stays_refused
  - authorization_requires_a_manual_confirmation_and_rule_scope_is_explicit
  - scale_4000_rows_reconciles_and_reports_metrics
  - scale_20000 / scale_100000（#[ignore]，用 --ignored 单独跑，均 ok）
- 前端：
  - test_ownership_review_groups 12/12
  - test_asset_ownership_gate 14/14
  - test_ownership_project_setup 6/6
  - npm run test:asset-exposure 59/59
  - vue-tsc --noEmit exit 0

## 规模（debug 构建，临时库）
| 指标 | 4000 | 20000 | 100000 |
|---|---|---|---|
| 范围内 / 组数 | 3974 / 234 | 19809 / 304 | 98985 / 406 |
| 全量 / 增量分析 ms | 322 / 145 | 1618 / 733 | 8859 / 4053 |
| 变化 = 重分类 / 沿用 | 39=39 / 3935 | 198=198 / 19611 | 990=990 / 97995 |
| 组页 p50 / p95 ms | 9.25 / 9.64 | 8.72 / 9.62 | 8.63 / 9.63 |
| 单页最大字节 / URL | 140928 / 246 | 140947 / 246 | 141042 / 246 |
| 成员页 p50 / p95 ms | 2.97 / 3.08 | 3.23 / 6.54 | 4.11 / 4.84 |
| 最大组 / 可执行 | 588 / 394 | 2941 / 1964 | 14713 / 9810 |
| 预览 ms / 首次调用 ms / 状态 | 13 / 63 / completed | 68 / 350 / completed | 354 / 1390 / applying（共 1766） |
| RSS 开始 / 全量峰值 KB | 21168 / 31312 | 21152 / 32320 | 21280 / 35408 |
| DB 种子 / 全量后 / 结束 B | 4.37M / 4.37M / 5.87M | 11.9M / 15.6M / 21.0M | 50.8M / 68.2M / 102.2M |
| WAL 全量后 / 结束 B | 3.02M / 4.19M | 4.53M / 4.62M | 5.10M / 9.32M |
| 取消延迟 ms（已处理） | 32（500） | 31（500） | 32（500） |

- 10 万行时总数对账一致。页面按 50 组分页，最多 246 个 URL，不会带全部 ID/URL。
- 10 万行时大组首次调用 1.39s：开始事务里一次性评估并写入 9810 条 staging，这一步没有分块。后续分块续跑。

## 变异验证（`~/.ovi_o3/mut_run.py`，副本中执行）：17/17 红
| 编号 | 变异 | 失败的测试 |
|---|---|---|
| M01 | 沿用条件去掉 last_seen | incremental_snapshot_equals_full…、changes_made_while…、scale_4000… |
| M02 | 版本变化不触发全量 | rule_directory_and_profile_changes… |
| M03 | 分析认领不认外来 nonce | 26 个失败，含 startup/chunked/incremental 等 |
| M04 | 操作认领不认外来 nonce | chunked_group_apply…、recovery_never_restores… |
| M05 | staging 重读用 seq>= | 13 个失败，含 chunked/concurrent/parallel 及 O2 测试 |
| M06 | 批量 UPDATE 路径写成 allowed/1 | chunked_group_apply…（加强后） |
| M06b | 批量 INSERT 路径写成 allowed | chunked、recovery_never_restores…，以及 O2 的 group_confirmation…、acceptance_4000…、scale_4000（共 5 个） |
| M07 | 去掉人工改动 CAS 守卫 | concurrent_applies_and_manual_changes… |
| M08 | 组互斥索引不再唯一 | concurrent_applies_and_manual_changes… |
| M09 | 回退恢复 eligible | group_revert_restores_ownership_facts… |
| M10 | 撤销第一步不阻断下游 | revoking_authorization_blocks…、recovery_never_restores… |
| M11 | 确认归属写入 allowed/1 | confirming_ownership_alone_grants_nothing…（§12） |
| M12 | 授权不要求人工确认 | authorization_requires_a_manual_confirmation… |
| M13 | exact_host 规则变成 domain | 同上 |
| M14 | 共享域名允许子域 | 同上 |
| M15 | 轮询不退避 | review_groups cjs "polling backs off…" |
| M16 | 确认总是连带授权 | gate cjs "confirming ownership is its own step…" |

**第一轮 M06 存活。** 原因：测试成员都没有既有结论行，只走 INSERT 路径。处理：
- 加强 `chunked_group_apply_survives…`：41–43 预置未授权结论行，断言 UPDATE 和 INSERT 两条路径都写到，6 条最终都是 confirmed/unknown/0。
- 新增 M06b。复跑后 M06 和 M06b 都红。

**runner 的问题（已修）。** 还原时 copy2 保留旧 mtime，cargo 复用了变异二进制，导致两次"未变异"复跑出现假失败（与上一个变异的失败一致）。
- 把还原文件 touch 后复跑：59 passed。
- runner 已改为还原后更新 mtime。
- 每个变异写入都产生新 mtime 并整 crate 重编，所以变异结果本身有效。

## clippy
- 修了 4 个归属文件的警告：
  - `ownership_o3_tests.rs`：`.err().expect` 改成 let-else。
  - `ownership_review.rs`：复杂类型抽成 `OwnershipStepState` 别名；`start_ownership_analysis_for_connection` 加 `#[cfg(test)]`。
  - `ownership_operations.rs`：`apply_ownership_group_decision_for_connection` 加 `#[cfg(test)]`。
- 复验 `cargo clippy --lib --tests`：exit 0。全项目 70 个警告，归属相关文件 0 个。
  - 其余分布在 multi_agent/agent_native/agent_tests/agent_runtime。
  - commands.rs:7 有一个未使用的 `VecDeque` import，不是本任务引入的，没动。

## 清理
- 副本 36M、副本 target 7.0G，均已删除。删除后 `~/.ovi_o3` 共 672K。
- 真实仓库 `src-tauri/target` 当前 6.9G（本轮开始时 4.8G，增长来自 Codex 构建）。按指示没有 cargo clean。

## 可选项（本轮未做，转遗留）
- **大组首次 staging 分块：** 需要新增 'staging' 操作状态，要改 db_schema.rs 的 CHECK，还要处理恢复和 CAS 语义。风险不低，没做。
- **缩小分页载荷：** 会改 O2 的 DTO 和前端，也没做。

## 需用户决定 / 范围外
1. 哨兵主动扫描（scan_lifecycle / scan_startup）不读归属授权。"授权"只等于暴露面被动采集准入，不是主动扫描授权。若要让主动扫描也受它约束，需要改门禁以外的代码。
2. 现在恢复是在第一次归属 IPC 时触发。若要真正在 app 启动时接管，需要在 lib.rs 的 setup 闭包里加一行 `ownership_ensure_startup_recovery`，lib.rs 只允许改注册部分，需批准。
3. Exposure 每次运行只加载一次 grants。撤销在运行中途发生时，下一次运行才生效；当次运行要靠 ExposureScope 读到的状态。
4. 旧的 learn_domain_rule 改为精确 host。
5. O2 新表没有迁移（尚未发布）。
6. 单页约 140KB。有上界，但偏大。
7. 项目 3 这类未绑定目录的项目仍不按标签合并（未放宽）。

## O4 前遗留
- 决定上面第 1、2 项。
- 10 万行大组开始事务的 staging 改为分块。
- 考虑缩小页面载荷。

## 追加：IR-02 / IR-11 按资产的只读单位投影（2026-10-09，用户 09:40 批准）

- 新增 `commands/ownership_unit_projection.rs`，在 `commands.rs` 加 1 行 include。提供只读函数 `ownership_asset_unit_projection` / `ownership_asset_unit_edge`，按资产给出 ownerEntityId、关系、证据和版本，整个投影另有版本摘要。
- **门禁语义没有改：** 没有新增列或表，不写库。确认归属、授权、准入、分组和恢复的代码都没有改动，投影只读取 O2/O3 已有的 operations/items/groups/events。
- 权威依据有两种：
  - 单位项目：人工确认 + 绑定单位。
  - 集团总账：人工确认，并且最近一次已应用的组操作是一个已完成的 confirm_ownership；该组为 consistent 车道、候选来自 directory；实体已确认、在绑定集团内、是单位类型；有决定事件，之后没有改判。
- 候选组、自动判断、冲突组、伪造的确认都不计入，也不按名称或区划码推断。
- 组撤回、组决定后的人工改判、目录退役、改绑和目录升版本都会反映到投影里。授权的授予或撤销不改变单位边（归属与授权分离，O3 口径不变）。
- 测试：
  - 归属模块新增 4 个 Rust 测试：只算人工确认的目录组决定；撤回/改判/退役/改绑都会掉边；伪造候选组、非单位实体、决定事件缺失都不计入，单位项目按绑定单位计；暴露面逐单位读取投影，专业公司不归省。
  - `exposure/tests_g4.rs` 新增 1 个端到端测试：省下地市归省、专业公司下的分公司单列、无依据进“单位未知”、冲突候选不计入、撤销后统计立即下降且旧链接失效、跨项目零串数。
- 变异 17 个全部被杀死，覆盖候选误算×2、撤销后仍计入×2、专业公司误归省、跨项目串数据，以及进行中操作、绑定范围、目录版本、决定事件、非单位实体、auto_link 失效、统计过滤、auto_link 不读投影和前端×3。首轮有 2 个存活（决定事件缺失、非单位实体），补测试后被杀死。
- 闸门（隔离副本）：
  - `cargo test --lib ownership`：64 passed，3 ignored（原有 60 个 + 新增 4 个，全部通过）。
  - `cargo test --lib exposure`：183 passed，3 ignored。
  - `cargo check --lib` 和 `cargo clippy --lib --tests` 均 0 warning。
  - `npm run test:exposure`：100 通过。
  - 归属前端 3 套测试：32 通过。
  - `vue-tsc --noEmit` 通过。
