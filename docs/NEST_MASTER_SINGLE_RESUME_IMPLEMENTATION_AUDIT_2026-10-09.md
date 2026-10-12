# Single paid Resume 实施核对（2026-10-09）

状态：生产闭环未完成。当前镜像 3104 项；原 round 恢复用例独立失败 0/1，错误 web_start_paid_history_requires_explicit_financial_contract，1.63s。证据：/tmp/oviraptor-master-capacity-concurrency-20261008/single-resume-current-post-queue.log。没有修改原断言、取消普通启动门禁或产生新执行授权。

## 当前实际断点

- commands/scan_startup.rs 的 prepare_web_startup_in 在写状态、分配 attempt 文件以前调用财务历史门禁；Resume/Retry 均被覆盖。
- commands/native_web_resume_finance.rs 逐历史 Root 查 obligation、原 owner、determinate、remaining capacity 与已发布 Tick 后，遇到任何 progressed 历史仍无条件拒绝。只消除未知债务不足以恢复。
- commands/native_financial_continuation_preview.rs 的查询要求 orchestration_policy=multi，模式必须 Multi；模型合同包含 DecisionSummary schema 和 Coordinator 输出上限 1024/2048。
- budget/continuation.rs 的 settled_source_with_deadline_on 从原合同 coordinator 字段构造 CoordinatorLease，并要求原 Coordinator。Single 不可虚构该 lease 或套用 Multi receipt。
- commands/agent_native/single_budget.rs 在执行前要求原 Single Root live；单独绕过启动门禁仍不足以继承旧消耗，Root 出生与原预算合同必须同时成立。

## 原正向验收必须保留

原 agent_tests_browser.rs 的 e2e_pause_resume_and_reexecute_keep_the_budget：attempt1 实际模型与目标请求后暂停；attempt2 继承累计消耗并再次暂停；attempt3 继续新增工作，目标请求总数精确加一且保留部分覆盖证据；每个新 Root 轮次从1开始。attempt4 Retry 的本地状态 parent=None、target_requests=1 保持。该局部账本语义不能删除历史 Root 费用或退款。

resumed_single_root_refuses_inherited_or_skipped_model_round：实际先暂停，再通过生产入口创建新的原 Single Root；继承或跳号轮次拒绝，零新增 reservation/journal/SDK/目标请求。不能将“无法启动”误作轮次守卫已验收。

## 按依赖实施

1. 原 Single 财务来源能力：以真实 SingleExit、原 call/dispatch/received/费用/HTTP 物理证明为基础，绑定 scan/attempt/root/目标/原模式与完整十维。只读构造的类型不能签出生或发放权利。未知费用继续 blocked；供应商账单确认用独立不可变事实，不改旧退出证明。
2. 显式新合同：区分 Resume 和 Retry 的局部工作状态；两者保留全历史费用及义务审计。Resume 携带累计已用量与原截止时间。Retry 若需新额度，必须用新的显式批准合同；不能由当前默认配置静默刷新额度。保存独立 request UUID 和内容哈希，绑定有效 Single 输出 cap、完整输入上界、模型/endpoint/key/proxy/工具模板和报价版本。旧 null 输出合同不补签为可执行合同。
3. 原子出生：在同一写事务核原来源/批准/撤销/截止/权限/原 worker 退出，创建 attempt、原模式出生、冻结计划、Single Root 与共享预算绑定；未知提交结果只读对账，不能二次出生。普通启动门禁仅接受这一专属新能力，不能接受冷 JSON 或任意布尔开关。
4. 生产消费者：Single SDK 与目标工具均要求同一新合同，首次派发与后续每次派发核冻结参数和全扫描共享预算；先预留、收到账单后结算，缓存来源须认证，未知费用不归零。禁止原语义重放或旧 Root 重新授权。
5. 入口/UI：生产“继续扫描”走只读预览、明确确认、专属出生、真实执行和实时状态；模拟 IPC 只作组件证据。自动连续新 Root 不等于用户批准。

## 必要验证顺序

先增加真正的 Single 来源生产读取与负向；随后合同批准与原子出生；最后恢复两个原正向用例并跑关联全组。覆盖双连接/双进程、提交应答丢失、出生前后崩溃、旧权限撤销、跨 scope/模型/输出 cap 冷改、原 deadline 过期、累计预算不足和未知费用。每次判定新 SDK/目标次数、旧账和旧退出签名不变、唯一新出生与拒绝前零副作用。

本文件是源码核对与实施顺序，不是已实现功能，不表示完整 Master 完成。Exposure 业务、真实 DB/CAS/asset 未操作。

## 并行实施状态

用户明确授权多agent后，测试agent仅新增 agent_tests_single_source_negative.rs 和一行 include；预算agent仅新增 continuation/single_remaining.rs；主控统一接入。原 Single history_ 6/6（4.93s），扫描级 recovery_history_ 3/3 已收齐。新来源能力接入生产历史审计，读取原 SingleExit 前后相等且使用原物理费用核验，不伪造 Coordinator。十维历史剩余核验已接入来源构造；新工作 deadline 入口暂为 cfg(test)，没有批准消费者，不宣称生产批准已实现。新负向两项（Multi policy、SingleExit 六类冷改）正在统一编译/运行；新增十维正向断言随后复验。证据位于既有临时目录 single-source-* 日志。没有恢复新预算或实际 Single Resume 完成声明。


合并源码复验：single_source_negative_ 4/4（3.81s）、single_history_ 6/6（7.10s），严格lib+tests Clippy零警告（45.72s）。初版六字段冷改可能止于canonical字段顺序检查，因此仅算拒绝/不变证据，不算物理证明深层校验，也不称HMAC（SingleExit为物理hash证明）。测试agent正在保持原字段序重写该负向，并将3秒deadline夹具的监听器/client准备移至Root出生前；原cap、真实等待和过期拒绝不变。重新编译验证待收齐。来源/历史余额仍只读，不是Single批准/出生完成。

## 下一段并行文件边界

测试agent已保持原SingleExit JSON字段顺序与其他字节进行六类冷改，并把短deadline独立初始化挪到出生前；统一构建句柄75199仍在运行，日志 single-source-canonical-negative.log，未提前宣称通过。预算agent正在独占新增 commands/native_single_continuation_preview.rs，实现原Single来源与十维余额的生产只读预览；测试agent独占新增 agent_tests_single_continuation_preview.rs。主控负责source remaining getter、共享include和Tauri注册。批准/新出生/共享USD没有消费者，预览必须明确blocked，零SDK/写入/新权限。


预览生产接入复验：single_continuation_preview_ 2/2（0.72s），同镜像来源负向4/4（7.02s），零忽略；测试镜像编译40.11s。严格lib+tests Clippy仍在运行，日志single-continuation-preview-clippy.log，不提前声明通过。后续两agent独占模型合同及其测试新文件，主控核实际Single输出cap与共享接入；完整输入token上界仍缺，批准/出生/执行继续blocked。


Single预览严格lib+tests Clippy已通过（1m35s，零警告）。下一版模型合同已接入预览：实际Single派发和候选合同共用native_single_model_output_cap（configured值或8192），不改变生产cap语义；核原plan/mode/birth/模型调用物理行，当前upstream模型/endpoint/key/proxy只输出hash，不当原历史transport绑定，原output认证与完整input上界认证仍false。当前single_continuation_模型+预览组正在编译，日志single-continuation-model-contract-tests.log，尚未算通过。


模型合同初次组合复验3/4，原历史预览在当前模型列表为空时报配置缺失（single-continuation-model-contract-tests.log）。该配置问题不能遮蔽已核原来源的只读历史。已修复仅model_runtime_env配置失败分支：返回modelConfigurationState=unavailable/modelCandidates=[]、持续blocked/no execution；原scope/plan/mode/birth/物理来源检查仍在其前并精确拒绝。新增预览断言，修复组4/4通过（1.41s，零忽略），编译20.25s；日志single-continuation-model-contract-fixed.log。原失败日志保留，不提前算当前绿。自定义cap=37的真实Single wire验证将单独追加；不是产品配置已支持37的证据。
