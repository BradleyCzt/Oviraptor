# Oviraptor · 窃蛋龙

一个面向授权资产管理场景的跨平台桌面端。
新任务使用 Oviraptor Native Runtime；导入保留当前 Native JSON 和受支持的源码报告，旧版 Strix 运行时和结果格式不再接受。
界面使用 Vue 3，桌面壳和任务编排使用 Tauri 2 / Rust，数据保存在应用私有目录中的 SQLite 文件里；不要求系统预装 SQLite。

## 已实现

- 项目管理：按企业或工作范围建立项目，项目之间隔离目标、人工判定和变化记录。
- 种子导入：支持单条输入和 TXT/CSV 批量导入，可识别公司名、域名、IP、CIDR、ICP 和关键词。
- 内置 Worker：采集、归并、P1/P2/P3 分层、存活探测、SRC 结果压缩和 HackerOne 官方 API 同步脚本随应用打包。
- 任务编排：依次调用内置采集、分层和探测 Worker，实时记录进度和输出日志，可取消任务；也可以显式配置外部脚本目录覆盖内置版本。
- 原生 Agent 运行时：`src-tauri/src/agent_runtime/` 持有运行契约、事件存储、单写终态 reducer、快照/重放和云端/本地模型网关。Web、Code、Greybox、CI/CD 的新任务只使用 Native；旧 `strix` 配置值不再作为 Native 别名接受，也不会选择旧执行器。当前四个独立 run 包含 Coordinator、SPA/API Mapper、Web Executor 和 Evidence Reviewer，聊天式时间线与用户指令草案需经过确认和运行时门禁。Deep Investigator 已限制为只读角色；旧版由其触达目标的在途 attempt 会拒绝再次派发，须新建 attempt。更多专业角色和跨平台发布验收仍在实施中。
- 配置方案：Python 路径、脚本目录、FOFA 配置、采集档位、限速、并发、超时、P1/P2/P3 及其他层级、内容分类规则均可保存为多个方案。
- 资产数据库：候选与探测结果增量写入 SQLite；保存首次发现、最近发现、最近存活、人工结论和软删除状态。
- 镜像对比：每次运行只保存资产当前状态和变化事件，不重复保存整份 CSV 镜像；原始任务输出仍按运行目录保留，便于审计。
- 查询与导出：全文检索、多字段 AND/OR 查询、项目过滤、自定义显示列、自定义字段 CSV 导出。
- 人工确认：批量标记确认/不确定，软删除后可恢复。
- 内容隔离：博彩、色情和反向语境关键词由配置方案生成任务快照，探测脚本实际加载；仪表盘和侧栏可直接进入隔离区查看，规则变化会使旧探测缓存失效。
- 增量去重：同一项目的相同网络端点复用已有资产记录并保留人工结论；历史重复项只做可恢复的软隔离，不物理删除。
- 项目生命周期：空项目可删除；已有资产的项目禁止删除，可改为归档；已有项目可从概览或项目管理直接再次扫描探测。
- 后台与提醒：macOS 关闭主窗口后驻留状态栏，任务运行时状态栏图标动态旋转；启动时提醒超期项目和上次中断的任务。
- 应用设置：可配置项目未更新时间提醒，并上传 PNG 自定义应用和状态栏图标或恢复默认图标。
- SRC 清洗：自动软隔离 5xx、无法访问入口及同站点的 HTTP/HTTPS/www 重复项，原始 CSV 和数据库记录仍保留。
- HackerOne 看板：官方 Hacker API 项目、Policy、Structured Scope、排除项、收藏和变化提醒；可把允许提交的网络 Scope 发送到资产项目。
- 本地代理：配置 Clash HTTP 代理后同时作用于 FOFA、存活探测和 HackerOne 同步。

## 本地开发

开发构建需要 Node.js、Rust 和 Python 3。“配置中心 → 运行环境”可检测运行依赖：macOS 安装流程准备 Python 虚拟环境、模块、Node.js、redis-cli 和 Tailscale；Windows 使用 winget 准备 Python 3.12、Node.js LTS 和 Tailscale，并提示按需手动配置 redis-cli。Native Code 的容器分析器须另行提供已批准且固定 digest 的镜像，缺失时报告 coverage gap，不自动回退到旧执行器。安装过程显示 stdout、stderr 和失败阶段。

```bash
npm install
npm run tauri dev
```

前端检查与桌面端检查：

```bash
npm run build
cd src-tauri && cargo check
```

业务模块边界、Tauri 命令归属和新增代码规则见 [`docs/architecture.md`](docs/architecture.md)。
独立聊天主应用、内置 Assets/Scanning 插件及本地体验说明见 [`内置插件重构说明`](docs/BUILTIN_PLUGIN_HOST_2026-10-13.md)。
Strix 退出、多智能体角色边界和发布验收以 [`Master Plan`](docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md) 为准；当前完成项与未完成项见 [`实施进度`](docs/NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md)。

生成当前平台安装包：

```bash
npm run tauri build
```

### macOS 安装产物核对

产品版本号不等于构建来源证明。当前 `build.rs` 仅调用 Tauri 构建，没有持久记录源码快照与二进制的绑定；开发工作树检查通过不代表已安装应用包含这些修改。发布前须保存当次源码/依赖/资源清单、构建命令及结果，并核对最终签名产物，不能用旧包补填一份“当前源码”证明。

当前可用以下命令记录一次**本地 macOS 构建观察**（会执行构建，不是只读检查）：

```bash
nice -n 15 node tools/release/build_receipt.mjs .
```

工具先报告新的临时证据目录，保存构建前后项目输入清单、命令、受限环境策略、退出码/信号、日志及新 `.app` 文件摘要。每次使用独立 Cargo 输出目录、离线/锁文件模式和单作业，因此不能沿用旧包伪装成功，但冷构建较慢且占用额外磁盘；不会安装或启动应用，也不自动清理记录。`receipt.json`、输入清单和日志权限为 `0600`。记录与产物默认位于系统临时目录，发布归档前须一并保存；不要只保存终端摘要。构建中不要修改源码或依赖。

`build_observed` 仅表示本地构建退出成功、指定输入的前后摘要相同且新产物可读取。失败、异常、输入变化或缺失产物均记为 `failed`；进程被外部终止时可能停留 `in_progress`，不能当作通过。输入范围涵盖源码、资源、图标、capability、工具、脚本及根级构建配置/锁文件，不含文档、生成目录、已安装依赖和用户级工具链配置；拒绝项目 `.env` 覆盖、符号链接和未支持的平台构建配置。前后摘要不能检测“中途改动再恢复”，Cargo 离线也不是网络沙箱。

这是未签名、非 hermetic 的本地记录，**`sourceBinding` 仍为 `not_verified`**：不能证明依赖/工具链可信、签名有效、可复现构建、安装完成或运行验收。不要手工更改记录状态或用它追认旧包；继续做发布审核和下面的安装文件核对。执行记录中的 `bundleRoot` 才是该次新产物，不是默认 `src-tauri/target` 中的旧产物。

已有构建回执时，可一次性只读复核当前项目输入、回执对应构建包和安装目录：

```bash
node tools/release/verify_build_observation.mjs \
  . /absolute/path/to/oviraptor-build-XXXXXX/receipt.json /Applications/Oviraptor.app
```

工具读取回执旁固定名称的三份清单，校验摘要、状态及路径一致性，并分别报告 `currentInputsMatchRecorded`、`builtBundleMatchesRecorded`、`installedMatchesBuiltBundle`。不会构建、安装、启动应用、修改回执或保存报告；需要留档时自行将标准输出重定向。退出码 `0` 表示三项本地观察匹配，`2` 表示发现差异，`1` 表示证据无效或读取失败；**三项匹配仍不是可信来源证明或发布验收**。JSON 读取有大小上限，拒绝证据文件及输出目录中的符号链接；不适用于同时进行构建/安装写入的目录，也不抵御一致篡改整套未签名记录。原始证据目录及其中构建包需完整保留，不要只移动或复制回执。

可对**已完成构建且不再被修改的最终 `.app`**与安装目录执行只读比对：

```bash
node tools/release/compare_app_bundles.mjs \
  src-tauri/target/release/bundle/macos/Oviraptor.app /Applications/Oviraptor.app
npm run test:release-bundle
```

比对器流式计算每个普通文件的 SHA-256，检查缺失/额外文件及执行位差异；不启动程序、不安装、不覆盖、不读用户数据库或联网。退出码 `0` 仅表示两份文件树一致，`2` 表示存在差异，`1` 表示读取/输入失败。JSON 始终注明 `sourceBinding: not_verified` 与 `runtimeValidation: not_performed`，不得据此宣布源码一致、功能通过或正式发布完成。

目前仅用于本项目 macOS 包：要求基本 `.app` 目录结构，拒绝根及包内符号链接和特殊文件；不比较空目录、所有者、ACL、扩展属性，也不验证签名有效性。签名文件本身参与内容比对，因此重新签名也可能产生差异。检查过程中必须停止构建/安装写入；文件修改检测不是对并发恶意替换的安全保证。当前安装差异记录见 [`后端剥离审计`](docs/NEST_BACKEND_RETIREMENT_AUDIT_2026-09-26.md)。

## 首次配置

1. 打开“配置中心”，编辑系统默认方案；需要不同参数时使用“从默认方案创建”。系统默认方案不可删除，普通方案可删除。
2. 在 `FOFA account / email` 和 `FOFA key` 中填写凭据。数据库文件在 macOS/Linux 上强制为当前用户可读写（`0600`）；运行时临时 INI 同样为 `0600`，任务结束后删除，任务快照和日志不会保存明文 Key。
3. `Scripts directory` 默认留空，使用应用内置的 1/2/3/4 Worker。只有需要调试或覆盖脚本时才填写外部目录。
4. `Legacy config path` 仅用于兼容旧版：FOFA Key 留空时才会读取该 INI。
5. 调整采集限速和探测并发。大量目标建议先以保守速率验证配额。
6. 新建项目，导入目标，然后选择“完整流程”。

应用数据库、配置、任务快照和原始输出统一放在用户主目录的 `oviraptor/` 中，数据库文件名为 `oviraptor.sqlite3`。升级后首次启动会把历史数据库安全复制到新位置，并把过渡数据库归档到 `oviraptor/database-backups/`；原历史目录暂时保留作为可恢复备份。导出文件单独放在下载目录的 `oviraptor/` 中。

macOS 默认路径：

```text
数据：~/oviraptor/oviraptor.sqlite3
导出：~/Downloads/oviraptor/
```

## 远程 Worker

推荐两台电脑加入同一个 Tailscale Tailnet，不开放公网端口，也不需要 OpenSSH、端口映射或反向代理。

1. 在 Intel Mac 或 Windows 上安装对应平台的 Oviraptor，完成“运行环境”检测。
2. 在“Worker 节点”中开启“本机 Worker”。服务只监听检测到的 `100.x.x.x` Tailscale 地址。
3. 把页面显示的节点地址和访问令牌粘贴到 M1 主控端。
4. 主控端可检测远端环境、查看与暂停/继续/取消任务，并按项目增量同步扫描结果。

当前工作树未提供 `.github/workflows`，不能依赖旧文档提到的 `Build Oviraptor Workers` 自动构建。`npm run tauri:build:mac-intel` 和 `npm run tauri:build:windows` 只是已配置的目标命令，不代表当前机器具备相应工具链、构建成功或跨平台包已验收。应用本体不捆绑 Python、容器分析器或 Node.js 等大型运行依赖；具体可用性以运行环境检测和各平台安装包实测为准。

## 数据保留策略

数据库不保存每次查询的完整资产副本，而是保存一份当前资产、项目关联和事件差异，因此长期运行时增长主要来自新资产、变化事件和日志。任务原始 CSV 会占据更多空间；确认审计期结束后，可单独归档旧的 `runs/` 目录，不要直接删除 SQLite 文件。

“失去资产”表示某资产在一次完整任务中没有再次出现，并不等价于服务宕机；存活结论以最近一次 probe 结果为准。自动内容分类用于隔离高置信结果，仍建议抽查 `blocked_content`。

已有 P1/P2/P3 探测结果可以用附带的增量导入工具写入数据库，源 CSV 不会被修改：

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features import-tools --bin import-existing-results -- \
  --input-dir /path/to/probe_output \
  --project-name "历史资产"
```

该工具已迁移为 Rust，不需要 Python。默认使用用户目录下的 `oviraptor/oviraptor.sqlite3`，可用 `--db` 指定已有数据库；不会创建空库、修改源 CSV 或恢复已排除的资产。每个文件事务提交后立即输出进度，失败时回滚当前文件并记录失败状态。

内置 Worker 的原生迁移进度、已验证范围与尚未证明等价的高级功能，见 [Rust 原生迁移记录](docs/RUST_NATIVE_MIGRATION.md)。当前工作树不代表全部高级功能已通过迁移验收。


仅对已获得授权的企业和网络范围执行采集与探测。
