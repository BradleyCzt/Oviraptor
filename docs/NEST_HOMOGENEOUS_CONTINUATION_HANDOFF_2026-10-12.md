# Nest 同质 Worker 续作检查点（2026-10-12）

本文件是 GitHub 续作入口，不是验收结论。上传包含本工作区此前累积的代码、测试和定稿文档；不要据提交数量推断 Master Plan 已完成。

## 阅读顺序和硬边界

1. `NEST_CODEX_IMPLEMENTATION_TASK_PROMPT_2026-10-11.md`
2. `NEST_HOMOGENEOUS_CODEX_HANDOFF_DEEP_ANALYSIS_2026-10-11.md`（入口 §E.6，意图闸门 §J）
3. `NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md`
4. `NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md`
5. `NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md`
6. `NEST_HOMOGENEOUS_COORDINATOR_PHASE1_REVIEW_2026-10-11.md`（§8 部分过期）
7. `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`（工具与沙箱 §2.3，独立 Reviewer §8）

冲突：DESIGN > SPEC > ARCHITECTURE；意图增量以深度分析 §J 为准。忽略 `NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md` 的设计意见。

仅使用临时数据库、临时 CAS、合成测试身份。不得访问真实任务库；不得重置累积工作。Worker 只有 homogeneous_v1 业务画像，任务由主管动态选择；七个主管动作不增加，禁止 Worker 互聊／委派、魔法扩容。Phase 1 仍只读；执行模式须在原始签名 Root 出生之前显式冻结，不能修改 UI 标志后热升级旧任务。

## 已有证据和本次变更

- 入口让路、意图闸门、mailbox 消费／report／reclaim、动态 Worker、真实付费决策和聊天投影已有回归。
- 2026-10-11 隔离安装 App 曾完成真实模型只读链 `decompose → assign → accept → stop(goal_reached)`，结果是 `completed_with_gaps`，没有目标确认或独立 Reviewer 发布。它不能证明 2026-10-12 新执行链。
- 新增 `tool_supply/sandbox/container_parser{.rs,/tests.rs,/orphans.rs}`：固定本机 Docker CLI／既有镜像，不拉取／安装；无网络、只读根文件系统、不可执行临时输出、进程／CPU／内存／时间／输出硬限制。实际容器测试及独立 guardian 的 EOF／父进程 SIGKILL／guardian SIGKILL 后孤儿回收有单独验证。
- `execution_readiness.rs` 仅接纳 `offline-parser-container-v1` 的原始批准包、当前 Root 冻结计划及精确 runtimeDigest；旧 `offline-parser-v1` 的 macOS 硬内存不支持仍拒绝。
- `homogeneous/{execution_policy,parser_execution,execution_worker}.rs` 与原 SDK 调用接线：模型原 SDK 回执产生代码请求（本轮使用假服务的真实 SDK 传输），原批准包执行，签名实际结果，reported 后由主管 accept；不把解析结果确认成漏洞，不允许同一 attempt 重放。
- 合成 Cookie／Token／密码在显式任务策略下保留原文，包含 evidence graph 入库；不对模型服务密钥应用此展示策略。
- `execution_carrier.rs`、`commands/agent_homogeneous_execution.rs`、`execution_facts.sql` 已接入显式串行 target/review 分支：Rust 候选、控制组范围、原三侧 HTTP Broker、另一 Worker 的冻结材料复核和主管验收发布。**该分支尚缺完整正负集成和安装 App 验证，不能作为已完成交付。**
- 创建入口新增显式执行选项；批准工具和两身份时先保存草稿，复用任务预览的 `AuthorizationControlForm.vue` 登记控制组，再确认启动。
- 本次续作对 73 个保护路径做 SHA 对照，Exposure、旧 homogeneous/worker.rs、delivery.rs 均未改变；本检查点可能包含此前工作区已存在的 Exposure 变更。

## 必须接着做的闭环

1. 在真实临时任务上补齐付费 Root、Worker 原 SDK、真实硬沙箱、登记三侧 HTTP、另一 Worker 的付费 Reviewer、主管 accept 和 stop 的同一链集成。禁止用手写 receipt 或直接更新状态代替生产通道。
2. 增加拒绝／失败场景：复核者与产出者相同、缺控制组、对象／身份／响应绑定变化、缓存／跳转、篡改原始签名或 SDK 内容、撤销／过期／暂停、未知费用、部分请求已发但结果未知、超时回收后的新意图。不得确认假阳性，未知效果不得免费重放。
3. 复查新 target/review 的费用归属、独占槽位、capability 撤销、attempt 结束及 Root finally／重启回收，不能把 Phase 1 的绿色用例当作新 lane 的结算证明。
4. 复查 `goal_ready` 的每一项真实来源和签名；禁止凭 SQL 行计数接受伪造结果。解析回调当前重复验证运行环境，可优化，但不能减少取消／撤销检查。
5. 将孤儿清理接到实际新执行／App 重启入口；清理只针对 journal 中准确归属的容器，不复演已 claim 的程序。
6. 使用 Oviraptor 配置中心的有效 DeepSeek flash 配置做真实验证；不用 Strix 配置或旧注入 pro Key。模型 Key 仅在本机私有配置读取，不写 Git、不贴聊天。新试验用新的捕获目录，保留此前真实失败回执。
7. 构建并安装独立测试 App，启动前验证专属 acceptance root；从界面登录两个合成账号、选批准工具、建草稿／登记控制组／确认、观察真实聊天和结果、确认唯一发布及收官。真实 `/Applications/Oviraptor.app` 与真实库保持不动。
8. 最后跑 homogeneous 全回归、legacy smoke、沙箱／批准包／原出生绑定测试与前端构建，记录精确结果；不要自行宣布验收通过。

执行分支目前只对已验证的 macOS arm64 本机容器运行环境提供测试证据。不得据此宣称 Windows／Linux／浏览器沙箱或整个 Master Plan 完成。

## 本检查点的复测记录

均使用临时测试数据；以下结果只覆盖相应边界。

| 命令 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib homogeneous -- --test-threads=1` | 128 passed，含原只读入口／意图和原 SDK + 实际容器解析／主管 accept |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib tool_execution_readiness -- --test-threads=1` | 3 passed，原出生批准计划／不支持路径 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib execution_readiness::tests:: -- --test-threads=1` | 2 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib agent_runtime::tests::evidence_ -- --test-threads=1` | 8 passed，证据存储／策略边界 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib a_legacy_plan_still_runs_the_bootstrap_dispatch_unchanged -- --test-threads=1` | 1 passed，legacy bootstrap smoke |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib tool_supply::container_parser:: -- --test-threads=1` | 6 passed，真实容器限制／取消／清理 |
| `npm run build` | vue-tsc 与 Vite 构建通过 |
| `npm run test:agent-workbench` | 54 passed |
| 暂存区空白检查 | 默认检查含既有 MD 硬换行／文件末空行提示；关闭 blank-at-eol／blank-at-eof 后通过，保留历史排版 |

此前孤儿 journal／guardian 的真实子进程验证为 4 passed；本次没有重建或重新安装测试 App，不将此前二进制证据归到本检查点的新 target/review 链。
