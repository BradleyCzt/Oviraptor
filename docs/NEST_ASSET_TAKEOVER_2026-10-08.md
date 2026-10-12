# Asset 接管与 Qoder 下一步指示（2026-10-08）

用户最新纠正：主控仅接管点名的导出、Strix清理、实时日志和版本/缓存交付；暴露面专属功能仍由Qoder负责。本轮保护未提交改动，未清理真实数据、未删除资产、未提交或推送。

## 急用修改

- 资产 CSV 的 sentinelStatus 列标题改为“是否发送扫描”，值由当前项目扫描关联次数/发送时间生成“是/否”；保留字段键以兼容导出选择。页面仍保留详细调查状态。
- 配置页删除前端数据包预算编辑区，原生运行环境标题改为“多 Agent 运行环境”。底层证据裁剪设置保留兼容，不绕过运行时预算。
- 当前 src 前端无 Strix 文案；原有历史退役测试、旧数据库迁移过滤不是本地环境或版本检测，不删除历史证据。未核验用户当前安装 App 的版本；旧界面需要使用新构建才能更新。
- 主控注册 Qoder 的九个分页/范围/恢复/证据 IPC，并新增 test:asset-exposure 门禁脚本。

## 日志实时性证据

jobs.rs 的 log_line 经 asset_logs::append 持久写入后发送 asset-log-committed。useAssetLogPanel 经 useCommittedRefresh 订阅，50ms 合并事件并重新读取可信快照；15秒补偿丢失事件，隐藏页面不轮询，切换项目/卸载有代次隔离。日志不依赖 Python。33项前端测试通过，其中日志9项、暴露面24项。此为源码与模拟 IPC 证据，尚非安装 App 的实时观察。

## Qoder 交付核对与下一步

交付记录声称 Q0–Q6，后端62项和前端24项通过。源码此前九个命令没有 invoke_handler 注册，本轮已补；当前完整 Rust 测试在修改前启动，其结果不能证明新注册编译通过。

下一步优先级：

1. 验收新 IPC 的真实安装 App 路径：分页、项目隔离、只读证据、恢复；先临时数据库和 localhost，不能直接用用户真实资产运行采集。
2. Q6 完成生产采集到 cluster_repository::save 的调用，并补簇视图、代表/继承来源展示；当前只有仓储和测试写入，不算生产闭环。
3. 补采集额度、来源集合、出口策略的可编辑界面与持久化合同；当前默认 exposure_scan_settings 不是用户明确配置。
4. 补100,000条规模、真实磁盘证据与崩溃恢复测试；交付只覆盖20,000计划物化及4,000终态。
5. Q7 模型裁决/整改复测暂不自建模型客户端和预算。先列出共享 Native 授权/Tool Broker/预算依赖，由主控统一集成；ExternalSurface 未就绪不能翻开开关宣称完成。

并行边界：若 Qoder继续，只修改 exposure 内核/暴露面专属页面与测试；lib.rs、package.json、配置、资产导出、日志及 Native 共享合同由主控负责。当前未向外部 Qoder 自动发送消息，本文件可作为下一步任务书。

## 本轮最终检查

- npm run build：通过（vue-tsc 与 Vite）。
- cargo check -j 1 --lib：退出码0，新IPC注册与导出改动编译通过。
- 前端日志/暴露面测试33/33通过；未运行真实公网采集。
- 环境检查中仍有通用Python探测/安装逻辑，但已无Strix探测；资产实时日志本身是Rust持久事件链。
- 原Master整库测试仍在运行，启动于本次改动前；不能作为本次代码的全量回归证明。收尾后按用户先前要求暂停Master目标。

用户纠正后的严格边界及详细任务书：`NEST_ASSET_EXPOSURE_QODER_FRAMEWORK_2026-10-08.md`，优先于本文此前宽泛“接管Asset”的表述。

## 1.1.60 本地交付及安装态验证

当前Mac arm64/debug/custom-protocol App已打包并安装到/Applications/Oviraptor.app，桌面Oviraptor-1.1.60目录有ZIP及delivery.json；ad-hoc签名校验通过，源/安装包16文件树一致。真实安装版启动加载原有项目与资产页；实际CSV导出19666行，“是否发送扫描”仅是/否（以delivery.json精确计数为准），无Strix标题。数据库资产总数107558仍保留。

缓存仅将node_modules/.vite及应用WebKit缓存移入/Users/swyiic/oviraptor/application-backups/asset-urgent-1.1.60-20261008-120022；同目录SQLite在线备份可恢复。未删除资产、测试证据、target或真实任务记录。构建输入1794文件运行中观察到终态SHA不变；观察采于构建启动后，不冒称开跑前完整封存。

不是公证/优化发布或完整Master验收。原整库测试尚运行且已有失败。Qoder严格边界按最新框架任务书，不接管其暴露面业务。
