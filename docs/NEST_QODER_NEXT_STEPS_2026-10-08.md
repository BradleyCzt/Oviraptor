## 最新补充：归属门禁、项目预填与后台复核组

详细设计见 `NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md`。这是设计，尚未实施。归属门禁、组织目录共享合同、后台资产复核组和赋权拆分由主控处理；Qoder 保持 exposure 白名单，只提出只读消费字段，并在专属页面展示单位/地域、排查覆盖和泄漏事件。禁止自行修改 AssetOwnershipGate.vue、asset_ownership.rs 或建立第二套组织目录/授权。

当前实际队列只读快照：项目 3 待复核 14280、company 标签 63 类；项目 15 待复核 3960。不是前份设计中全部项目资产关联数；公司标签导航组不等于可整组确认的归属组。项目间复用目录，不复制授权；外部资料平台的信息关联不等于该平台为集团自有资产。

## 最新细化：集团/省市与信息泄漏产品设计

用户明确要求集团资产聚合、省市排查、专业子公司分离、内部资料/员工客户/代码/网盘文库/盗取售卖情报多方向覆盖，并排除正常招标信息。完整设计见 `NEST_GROUP_ASSET_EXPOSURE_PRODUCT_DESIGN_2026-10-08.md`，对该产品范围的细化优先于此前抽象的 E1–E5 描述。

执行顺序：原 G0 编译/并发/错误真实性收敛 → G1 组织目录与只读候选归属预览 → 一省现有来源闭环 → 一个代码平台+一个文档来源真实接入 → 员工/客户与情报来源。组织确认、scope、服务地域分离；苏州/杭州专业子公司不得按地名自动并入省公司；项目间不自动转授权；招标及其转载按策略排除。Qoder 保持原白名单，新增共享 schema/IPC 只提合同由主控装配。以下 P1 等稳定性修复保留。

# Qoder 下一轮指示：先收敛 E1–E5，再提交 E6 合同

2026-10-08。本轮主控只读复核发现此前八项编译错误已在当前 Exposure 源码中修正。主控正在独立编译和测试，不把 Qoder 自报结果当整库验收。

## 文件边界

继续仅修改 NEST_ASSET_EXPOSURE_QODER_FRAMEWORK_2026-10-08.md 第 1 节白名单：exposure 专属后端/前端/测试/专项交付文档。禁止修改 lib.rs、commands.rs、共享 db/schema/model/type、package/Cargo 配置、App.vue、AssetWorkspace、asset_export、日志/环境、agent_runtime、multi_agent、native_*、Master/progress/交接。主控此次注册 14 个新 IPC 并补齐 test:asset-exposure 七个文件；Qoder 不重复改装配。

## P1：配置并发与错误真实性（先做）

1. settings.rs::save 在事务之前读取/比较 current；两个连接能同时读到相同版本，随后依次覆盖。将当前版本加载、expectedVersion 检查、历史写入和 CAS 更新置于同一 Immediate 事务。失败不得覆盖草稿或新增虚假历史。用两独立 SQLite 连接验证只有一个相同 expectedVersion 保存成功，另一个明确冲突，版本只增一次；不要仅顺序重复调用。
2. exposure.rs::list_exposure_setting_versions 对 limit 裁剪后始终 truncated=false。使用 limit+1 或总数判断实际截断；测试超过 200 个历史版本及边界，不把当前历史窗口称为完整。
3. 新命令中 ensure_exposure_schema(...).ok() 会丢弃迁移失败。改为明确传播合同错误，不把损坏或不可写 schema 显示为空记录/默认设置。保留已存在原数据，注入迁移/表错误验收。
4. review_exposure_finding 旧入口缺 project/revision；列出所有生产调用并迁移专属页面到 scoped 入口。提出旧入口退役方案，由主控处理共享调用/注册；Qoder 不私改共享 models 或 lib.rs。保留历史只读，不制造授权绕过。

## P2：补齐 E4/E5 证据

- 按原 4000/20000/100000 档测量进程 RSS（峰值）、库/证据文件大小、耗时、分页时延。用临时库与确定种子；不要降规模、删测试或用 synchronous=OFF 数字冒充用户库性能。标注 SQLite 同步模式与硬件。
- 保留真实 SIGKILL、观察后/终态前恢复、费用不重置、同项目并发启动、撤权/取消后零新请求。补齐缺失的网络返回后/finish 提交与 ACK 丢失边界，不将已收到网络响应重试成新的支出。
- 复核→整改→新轮复测→新观察关联→旧历史可读：localhost fixture 证明跨项目、撤权、旧 revision 拒绝；来源失败、未看见、相同内容都不能关闭整改。
- 证据包核验与页面导出记录 rows/bytes/SHA；包含符号链接、覆写、公式与凭据脱敏负向。禁止触碰资产导出 asset_export.rs。

## P3：E6 仅合同交付

提交模型裁决候选输入、结构化输出、原 observation/finding/run 引用、scope/settings/rule 版本、费用与授权需求、缓存命中/过期/撤权判定合同。保留 not_ready。禁止自行创建 Root、账单、预算 grant、复制 Broker/执行框架，禁止自动授权新域或把推测变成观察。缓存复用目前无生产消费方，应明确保持未接入，待主控 Native 能力就绪后统一集成。

## 门禁与交付

主控排队 Cargo；Qoder 不抢锁、不终止其他代理进程、不 cargo clean。自有前端七组测试可独立跑。交付列修改文件及 SHA、每项生产路径、每个实际命令/退出码/日志、未完成与共享依赖。证据分为生产可达、未接入骨架、只读诊断、测试。实际安装 App 验收与新版交付由主控处理，不操作用户真实数据。

主控验收：npm run build；cargo build -j 1 --bin oviraptor；cargo test -j 1 --lib --no-run；暴露面 Rust filter、七组前端；Master 六项及邻近回归。局部通过不代表全量 Master 完成。
