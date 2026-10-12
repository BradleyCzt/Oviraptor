# 工具批准库存管理员指南（2026-10-10）

本指南按当前 Rust parser、service、IPC 和设置面板编写。已实现任务外签名批准/撤销库存、受保护的惰性缓存发布，以及新任务选择与首次 Root 工具计划冻结，返回 `executionAllowed=false`。新的 `offline-parser-bundle-v1` 适配器已接安全只读文件部署；旧适配器仍仅缓存原包。两者均不运行工具、不提供目标授权，也不为现有 attempt 热换能力。逐请求 Broker、真实隔离适配器及完整执行收据仍属后续闭环。

本文没有部署任何信任密钥、批准任何工具或操作真实数据库。所有样例都是不可直接提交的占位模板。

## 1. 权责和准备材料

管理员在任务外取得固定版本工具包及发布者签名清单，核对来源、许可、平台、入口和离线策略，再用独立管理员私钥签批准决定。Agent 不能提交信任锚、密钥路径、批准布尔值或安装命令。前端只能导入已签材料；应用不会生成管理员签名。

需要三份批准材料：

1. 发布者签名清单 JSON，最大 16 KiB。
2. 工具包原始完整字节，当前交互式入口最大 64 MiB。
3. 管理员签名批准 JSON，最大 16 KiB。

撤销只需要管理员签名撤销 JSON，最大 16 KiB。空文件会被当前前端拒绝。大浏览器分发包不能假借此内存入口处理，需另做有界流式导入。

## 2. 离线信任配置

唯一生产配置来源为 Tauri `AppHandle.path().app_config_dir()`，文件相对位置是：

```text
<应用实际 app_config_dir>/tool-administrator/trust.json
```

不要把上述占位路径当作已确认的本机目录。当前面板没有配置密钥或写信任文件功能；由任务外管理员按实际应用配置目录部署。本仓库尚无现成签名 CLI，本文不提供未经验证的可复制签名命令。

配置按以下**真实字段名**填写，未知字段会被拒绝：

```json
{
  "schemaVersion": 1,
  "administrators": [
    {
      "id": "offline-admin",
      "publicKeyBase64": "<32字节Ed25519公钥的标准Base64>",
      "publicKeySha256": "<解码后公钥字节的64位小写SHA256十六进制>",
      "revoked": false
    }
  ],
  "publishers": [
    {
      "id": "approved-publisher",
      "publicKeyBase64": "<发布者32字节Ed25519公钥的标准Base64>",
      "publicKeySha256": "<解码后公钥字节的64位小写SHA256十六进制>",
      "revoked": false
    }
  ]
}
```

模板中的尖括号值不能通过 parser，必须换成管理员核验后的真实公开材料。**私钥不得放入该文件、工具包、前端、日志或仓库。**

实际约束：

- `schemaVersion=1`；管理员 1–32 个，发布者 1–64 个；各列表内 ID 唯一。
- ID 为 1–80 字节，仅小写 ASCII 字母、数字、`-`、`_`。
- 公钥解码后恰好 32 字节，哈希与原字节匹配。
- `trust.json` 不超过 64 KiB。
- Unix 上应用配置目录、`tool-administrator` 子目录及文件须由当前有效用户持有，不能允许组或其他用户写入；文件只能有一个硬链接。可用目录 0700、文件 0600作为部署权限。
- 两个受检查目录不得为符号链接；最终文件按 `O_NOFOLLOW` 打开。当前实现不支持的系统权限验证会明确拒绝，不自动降级。

没有配置时返回 `tool_administrator_trust_unconfigured`；默认信任锚、默认批准均不存在。配置变化会重新校验，已经撤销或不再受信任的密钥不能继续证明可用库存。当前权限检查和前后内容哈希复核，不应被描述成已完成所有目录替换竞态的验证。

## 3. 发布者清单及绑定

发布者清单外层字段为 `payload`、`signerId`、`signatureBase64`。payload 按 `ToolCapabilityManifest` 的 camelCase 字段编码，包含：

```text
schemaVersion, id, version, platform, source, digest, license, entrypoint,
brokerCapabilities, networkPolicy, filesystemPolicy, timeoutSeconds,
memoryMib, cpuLimit, processLimit, outputBytes, inputSchemaVersion,
outputSchemaVersion
```

`digest` 是原工具包全量字节的 `sha256:<64位小写十六进制>`，不能对解包结果、预览片段或重新压缩后的文件计算。版本不能为空或 `latest`；入口必须是受限相对路径，不能含绝对路径、`..`、反斜杠或驱动器冒号。

批准入口目前只接受 `networkPolicy="none"`，能力集合只能含 `evidence.read`、`source_analysis`；文件策略必须为 `read-only-input-and-private-temp-output`。当前清单 parser 还限制超时、内存、CPU、进程数、输出字节数及 schemaVersion。这些是清单合法性约束，**不是已验证的 OS 限额执行证据**。

平台值与管理员决定中的 `platform` 精确一致；不自动把别的平台包转换成本机包。`runtime_adapter` 和 `runtime_digest` 由管理员签名绑定，但批准库存不会凭这些字符串启动适配器；后续仍须核真实 registry、实际适配器字节、平台和隔离能力。

## 4. 批准决定与签名编码

外层用 camelCase；内部 `decision` 的字段使用下列**snake_case**，不能按前端习惯改名：

```json
{
  "administratorId": "offline-admin",
  "decision": {
    "kind": "approve",
    "decision_id": "<新生成且为小写标准形式的UUID>",
    "manifest_hash": "<完整SignedToolManifest经Rust序列化后的SHA256，不含sha256前缀>",
    "package_digest": "sha256:<工具包全量字节SHA256>",
    "platform": "<与签名清单精确相同的平台>",
    "runtime_adapter": "<已核验的适配器标识>",
    "runtime_digest": "sha256:<实际适配器字节SHA256>"
  },
  "signatureBase64": "<管理员64字节Ed25519签名的标准Base64>"
}
```

批准签名输入是以下字节拼接：

```text
ASCII("OVIRAPTOR-TOOL-ADMINISTRATOR-APPROVAL-V1") + 0x00
+ serde_json::to_vec(AdministratorDecision::Approve)
```

签的是内部 Rust typed decision，**不是外层 envelope、文件原始排版或自行排序后的 JSON**。域末尾是一个 NUL 字节，不是两个文字字符 `\0`。字段順序、紧凑编码及字符串转义必须与当前 Rust 序列化一致；泛用 JSON 工具不能未经互操作测试就当作签名生成器。

`manifest_hash` 覆盖发布者**完整签名清单**的 Rust canonical 序列化（包含签名外层），不是仅 payload。批准返回的 `approvalHash` 则是完整管理员 envelope canonical 字节的 SHA256。UI 可显示清单预览，但预览不验签、不授予批准。

## 5. 当前设置面板操作

`AppSettingsDialog` 已包含 `ToolApprovalPanel`。在工具批准库存面板选择三份批准文件，核对清单预览后使用“验证并保存批准库存”。面板调用 `approve_tool_inventory`，后端重复核验管理员/发布者签名、摘要、配置、原准备租约和受限写事务。

成功返回原批准 ID、工具版本/平台、包摘要、适配器摘要、`approvalHash`、`status="approved_inventory_only"`、`executionAllowed=false`，以及 `preparationFencePresent`。记录这些公开收据标识用于查询和撤销，不记录私钥。

输入批准 ID后可“读取原库存”，调用 `read_tool_approval_inventory`。返回 null 表示该 ID 未找到；签源损坏、信任失效或原批准已撤销会返回明确错误，不伪装成可运行工具。

相同已签请求重入会重读原签据并严格核原请求，不创建新批准或放开扫描门禁。同 ID 的不同签名材料或清单被拒。不要通过更换 ID掩盖一次不确定提交；先读取原 ID和准备状态。

当前批准服务先在真实准备租约下把原签包发布到 App 配置目录的 `tool-supply-cache`，再在受限事务中保存原批准并复核缓存来源。相同原批准缺缓存时可用完全相同三份材料补缓存，批准签行保持不变；已存在的损坏包、链接或权限异常不会被覆盖。新适配器随后在 `tool-supply-deployed` 进行独占只读文件部署，旧适配器不会因包头相似而解包。失败可能保留准备 fence 和本次暂存目录，按下方原准备恢复流程核对，不手工删除缓存或租约。部署失败后原批准可能已保存，错误会明确带原 ID 和“准备未完成”。成功回执不表示工具已执行。

## 6. 撤销

从原批准收据取得 `approvalId` 与 `approvalHash`，使用独立的新 UUID 签以下决定：

```json
{
  "administratorId": "offline-admin",
  "decision": {
    "kind": "revoke",
    "decision_id": "<新的标准UUID，不得等于原批准ID>",
    "approval_id": "<原approvalId>",
    "approval_hash": "<原approvalHash>",
    "reason": "<非空，最多512字节的撤销原因>"
  },
  "signatureBase64": "<管理员64字节Ed25519撤销签名的标准Base64>"
}
```

撤销域是 `ASCII("OVIRAPTOR-TOOL-ADMINISTRATOR-REVOCATION-V1") + 0x00`，再拼 `serde_json::to_vec(AdministratorDecision::Revoke)`。不能复用批准域的签名。

面板导入“签名撤销决定”后调用 `revoke_tool_inventory`。成功返回原撤销 ID、`revoked_inventory_only`、`executionAllowed=false` 和准备 fence 状态。相同原撤销请求可精确重入；冲突材料不能替换已有撤销。数据库不可变记录保持原批准和撤销，不通过删除历史表示撤销。信任配置中的 `revoked=true` 是锚的撤销，作用范围与某个批准的撤销不同。

## 7. 出错与准备 fence 恢复

完整签名/包 preflight 在取得准备租约前执行。未配置、无效签名、包不匹配等前置错误不能创建批准。writer 开始后，错误保留 durable preparation fence，供人工核对，不能自动宣称未发生提交。

- `tool_administrator_receipt_saved_preparation_incomplete:<原ID>:…`：批准收据已保存，准备阶段未完成。
- `tool_administrator_revocation_saved_preparation_incomplete:…`：撤销已保存，准备阶段未完成。
- `preparationFencePresent=true`：库存回放也明确显示 fence 尚在；不会替另一个 owner 自动调用 complete。
- `tool_administrator_service_worker_failed` 或 IPC 回复中断：先查原批准/撤销及准备状态，不能据此判断“没有写入”。

使用现有 `get_environment_preparation_status` 检查：`installing` 表示仍有 OS 锁，`requires_manual_recovery` 表示无锁但原 durable owner 尚在，`idle` 才表示已解除。释放 OS 锁、进程退出或等待超时本身不能证明安装子进程已停止。

管理员核对所有相关准备/安装子进程确已停止、无 queued/scanning/pausing 扫描后，使用现有 `recover_environment_preparation`，传**当前读取的 expectedOwner**及精确确认语 `已确认安装子进程停止`。恢复仍会检查锁、活跃扫描和原 owner；owner 已变时重新读取并核对，不使用旧确认。确认语是人工陈述，不是自动进程检测，也不批准/执行工具。不要手工删除 SQLite 租约行、签据或锁文件来“恢复”。

## 8. 当前证据与边界

2026-10-10 主控最新限定证据：工具批准专项 11/11；真实准备租约新实体/遮蔽等 6/6；原环境准备关联 4/4；固定 OS 边界 2/2。App `cargo check` 通过（5.91s），当前前端 `npm run build` 的 vue-tsc/Vite 通过。对应日志与事实摘要见 `docs/progress.md` 顶部；这些计数不是全库绿色，也不能相加称完整 Master 完成。

批准命令 `approve_tool_inventory` 当前已使用 Tauri Raw 二进制 body：前端 `invoke(command, Uint8Array)`，不再将大工具包展开为 `Vec<u8>` JSON 数组。帧为 ASCII `OVTA0001`（8 字节）+ 三个 little-endian `u32` 长度（decision、manifest、package），共 20 字节 header，后接三段连续原始字节。三段必须非空，分别不超过 16 KiB、16 KiB、64 MiB；header 声明长度之和必须与实际帧长精确一致，禁止缺失或尾随字节。前端先检查所有段才分配一次完整 frame；后端拒绝 JSON body，并在克隆/进入工作队列前解析原 Raw frame。撤销与查询仍使用 JSON 参数。

这减少大包 JSON 展开及业务层重复分段分配，但 **Raw body 到达 command 前仍已由 Tauri 缓冲分配**，不是完整 transport 内存上限。全局单任务 guard 限制排队/执行，不证明底层消息在进入门禁前没有分配。后续大包仍需经过审查的流式或其他有界 transport，不能把 Raw 改造称为完整内存隔离。只读查询当前以 READ_ONLY、query_only 与同一 snapshot 读取原签据及 fence，不创建信任或库存。

当前新增限定证据：原 Vue SFC 与真实 API 模块的 host-DOM/mock-IPC 交互及帧回归 6/6 通过（`tool-approval-panel-raw-frame-all-segment-bounds-runtime.log`）；包括四类 File.size 早拒、busy 双提交只有一次调用、durable fence 展示、已保存但 preparation 未完成错误、撤销失败清除旧批准展示，以及 raw magic/长度/subarray 内容/各段空或超限拒绝。主控前端 vue-tsc/Vite 构建通过（2.59s），后端帧 parser 专项 1/1。host-DOM 不是真实浏览器，模拟 IPC 不证明签名验证或真实 App 调用；实际管理员配置/签名互操作和安装应用真实 DB 仍未验收。

签名 CLI、受信来源自动取得、可靠解包、通用 Broker 与完整 OS 执行链仍待交付。新任务工具选择、原启动策略签名及 fresh Root 工具计划冻结已接生产并有专项运行证据，仍不授予工具执行能力。当前 Darwin 硬内存隔离问题仍以不支持处理，不能因为批准记录或静态解析器检查通过就宣称 sandbox 可执行。

### 新任务选择批准工具（准备阶段）

在普通 Web 多 Agent 新任务中，将管理员库存成功回执中的批准 ID 填入工具选择控件。最多 16 个 ID；重复、无效 ID 或库存查询失败需先纠正。预览展示原工具名称、版本、平台与批准状态，只用于核对。清空选择沿用原任务创建路径；非空选择使用专属创建入口，不允许 Single、Source 或关联任务默默丢弃选择。

启动后，原策略签名绑定本任务的选择和批准库存快照，首次 Root 出生在同一事务冻结供给；再次进入相同任务必须继承原冻结内容，不能热换工具。批准撤销或库存改变时，当前信任检查拒绝新的使用；历史签名来源可以只读核验，但历史有效不恢复执行授权。需要改选择时创建新任务。

当前组件交互验证使用真实 Vue 组件与 API、模拟 native IPC，11/11 通过；真实生产草稿、启动签名与首次冻结另有后端专项通过证据。安装 App 中的完整工具执行仍未验收，请勿将预览、批准或冻结状态作为工具已运行的证明。

## 源码核对入口

- `src-tauri/src/tool_supply/approval.rs`：实际字段、信任文件 parser、域、签名、原签据与撤销校验。
- `src-tauri/src/tool_supply/approval_writer.rs`：独立受限 writer及原准备 owner验证。
- `src-tauri/src/tool_supply/administrator_service.rs`：preflight、免费重入、fence 投影及库存 DTO。
- `src-tauri/src/commands/tool_administrator.rs`：三项 IPC；无信任配置或执行入口。
- `src-tauri/src/db.rs`：真实准备锁/数据库实体绑定、complete 和人工恢复。
- `src/features/environment/ToolApprovalPanel.vue`、`toolApprovalApi.ts`：文件选择、预览、大小与库存状态检查。

本指南只描述当前管理员库存层，不能替代 Master §2.3 的完整工具供应和执行验收。
