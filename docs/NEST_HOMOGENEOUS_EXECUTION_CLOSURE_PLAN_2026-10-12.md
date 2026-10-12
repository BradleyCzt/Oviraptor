# Nest 同质执行链闭环接手计划（2026-10-12）

本文件是新会话的接手执行计划，不是验收结论，不新增设计裁决。
依据与阅读顺序完全沿用 `NEST_HOMOGENEOUS_CONTINUATION_HANDOFF_2026-10-12.md`；
冲突时以该文件点名的文档链为准（DESIGN > SPEC > ARCHITECTURE，意图增量以深度分析 §J 为准）。

## 范围

只推进该交接「必须接着做的闭环」1–8 条，按依赖排序为四个批次。
不改七动作枚举、不改 exposure 黑名单（`commands/exposure.rs`、
`multi_agent_external_surface.rs`、Asset Exposure 业务与前端、`docs/NEST_ASSET_EXPOSURE*`）、
不动 legacy 固定角色默认路径、不碰真实任务库与真实 `/Applications/Oviraptor.app`。

## 批次划分

### 批次 A：target/review 同链正负集成（闭环 1、2）

在真实临时任务上补齐同链集成：付费 Root → Worker 原 SDK → 真实硬沙箱 →
登记三侧 HTTP → 另一 Worker 付费 Reviewer → 主管 accept → stop。
禁止手写 receipt 或直接 UPDATE 状态代替生产通道。

负向矩阵至少覆盖：复核者=产出者、缺控制组、对象/身份/响应绑定变化、缓存/跳转、
篡改原始签名或 SDK 内容、撤销/过期/暂停、未知费用、部分请求已发但结果未知、
超时回收后的新意图。未知效果不得免费重放，不得确认假阳性。

验收：新增集成测试全绿；每条负向场景有确定性断言（拒绝原因码/状态）。

### 批次 B：结算与状态机复查（闭环 3、4）

复查新 target/review 的费用归属、独占槽位、capability 撤销、attempt 结束、
Root finally/重启回收。Phase 1 的绿色用例不作为新 lane 的结算证明。
复查 `goal_ready` 每一项真实来源与签名；禁止凭 SQL 行计数接受伪造结果。
解析回调重复验证运行环境可优化，但不得减少取消/撤销检查。

### 批次 C：孤儿清理接线 + 真实模型验证（闭环 5、6）

孤儿清理接到实际新执行/App 重启入口；只清理 journal 中准确归属的容器，
不复演已 claim 的程序。真实模型验证使用配置中心有效 DeepSeek flash 配置，
新试验用新捕获目录，保留此前真实失败回执；Key 仅本机私有配置，不写 Git、不贴聊天。

### 批次 D：独立安装 App 端到端 + 全量回归（闭环 7、8）

构建并安装独立测试 App（专属 acceptance root），双合成账号从登录、选批准工具、
建草稿、登记控制组、确认启动、观察真实聊天与结果，到唯一发布与收官。
随后按交接表格逐条复跑：homogeneous 全回归（--test-threads=1）、legacy smoke、
tool_execution_readiness、execution_readiness、container_parser、evidence、
`npm run build`、`npm run test:agent-workbench`。精确记录结果，不自行宣布验收通过。

## 已知事实核对（接手时基线，均出自代码实读）

- 执行链挂点：`apply.rs` → `execution_carrier::dispatch_slice`；
  `commands/agent_homogeneous_workers.rs` worker 循环 → `execution_worker::deliver`；
  root 循环在 `commands/agent_backend_homogeneous.rs`。
- 沙箱旗标：`container_parser.rs` `--network=none --read-only --cap-drop=ALL
  --security-opt=no-new-privileges --user=65534:65534 --pids-limit=1` + CPU/内存/swap/时间限制
  与自定义 seccomp；`execution_readiness` 仅接纳 `offline-parser-container-v1`。
- 数据库：homogeneous/schema.sql 10 表 + parser_execution.sql 2 表（不可变触发器）+
  execution_facts.sql 1 表；均由 `db_schema.rs` include_str! 随 `db::open` 应用。
- `homogeneous/mod.rs` 仍有 `#![allow(dead_code)]`（自述「少量有 API 无调用方条目」），
  批次 A 接线时顺带核减，不作为独立目标。
- 文档测试计数与静态 grep 有小偏差（readiness 实测 8 条 vs 文档 3+2），
  批次 D 以实际 `cargo test` 输出为准记录。

## 硬边界（全文沿用交接原文）

仅使用临时数据库、临时 CAS、合成测试身份；不得访问真实任务库；不得重置累积工作。
Worker 只有 homogeneous_v1 业务画像，任务由主管动态选择；七个主管动作不增加；
禁止 Worker 互聊/委派、魔法扩容。Phase 1 仍只读；执行模式须在原始签名 Root 出生之前
显式冻结，不能修改 UI 标志后热升级旧任务。

## 记录义务

每批次完成后在本文末尾追加一节：改动路径、测试命令与精确结果、未完成项。
未完成必须写「未完成」。
