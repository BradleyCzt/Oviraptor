# 历史环境依赖判断与清理（2026-10-08）

用户授权判断Docker/Redis/Python是否仍需要，并删除不需要的项目依赖。未卸载系统软件、删除系统Python、清除Docker容器/镜像或Redis服务数据；这些可能属于其他项目。

| 环境 | 真实当前调用 | 处理 |
|---|---|---|
| Python宿主运行时 | jobs/native_workers 与 Native生产执行没有调用pythonExecutable；仅环境检测/安装/旧配置字段使用。源码扫描解析Python清单/CodeQL语言名不是宿主Python依赖；tools迁移脚本是开发工具 | 删除项目运行检测、自动安装、venv创建、配置控件和默认值；旧存储值作为惰性历史保留，normalize_settings不再暴露或执行，新增保存不能激活 |
| Redis/redis-cli | 未发现资产/Native/远程Worker执行消费者，只剩检查、安装、文案和字段 | 同上移除检测/安装/配置；无Redis服务清理或数据删除 |
| Docker | native_source_scan::configured_native_analyzer_runtime实际要求locate(docker)，Semgrep/CodeQL使用已批准固定摘要镜像，analyzer生产容器runner仍在 | 保留源码/灰盒/CI可选分析器支持及专属镜像配置，不作为资产/Native HTTP前置；不自动安装Docker，不恢复Strix |
| Node.js/浏览器 | 随包浏览器/CDP及JS AST有真实生产调用 | 保留检测及Node安装；脚本完整性不等于进程/网络沙箱 |
| Tailscale | 远程Worker连接 | 保留远程准备步骤 |

轻量级Native执行/权限/Broker与有界进程机制不能证明已替代Semgrep/CodeQL容器隔离。不能为了删除Docker把源码分析器降级为未隔离宿主执行，也不能删除现有分析器后称功能完成。浏览器/AST独立沙箱目前仍明确unsupported_sandbox。

涉及自有共享环境文件：commands/environment.rs、models_appsec.rs、db_initialize_budgets.rs、db_settings.rs、ConfigDialog.vue、WorkerSettingsPanel.vue、App.vue、src/types.ts；不修改Qoder暴露面业务。

自动安装现在Windows仅Node+Tailscale，macOS仅brew node与Tailscale；保留原持久安装日志、环境准备租约和扫描启动隔离守卫。没有实际执行任何安装器。

## CSV命名

asset_export.rs使用当前query.project_id读取真实项目名，无项目限定用“全部项目”。文件名为项目名_YYYYMMDD-HHMMSS-毫秒.csv，过滤路径/控制/Windows非法字符、限制Unicode长度；使用create_new防止覆盖同名。未知项目报错，不偷偷换项目。内部字段sentinelStatus保留兼容，但标题“是否发送扫描”、内容是/否。项目名清理仅影响文件名，不修改数据库名称。

## 验证与交付

CSV命名3/3、旧设置惰性/保存历史1/1，日志/安装日志/暴露面前端39/39；本轮vue-tsc+Vite及原生App构建成功，ad-hoc签名校验通过。1.1.61已安装并实际启动：香港移动导出3960条，文件名香港移动_20261008-122337-287.csv，是否发送扫描值为是/否。实际环境检测不再显示Python/Redis；原生工具、随包浏览器/AST完整性通过，独立沙箱仍如实显示unsupported_sandbox。数据库仍107558资产。

桌面 /Users/swyiic/Desktop/Oviraptor-1.1.61 有ZIP、SHA256 delivery.json和Qoder任务书；安装App与构建App的16文件树一致。构建输入1794文件从运行中观察到终态SHA不变（观察非开跑前封存）。旧1.1.60 App及数据库/缓存备份在application-backups/asset-urgent-1.1.60-20261008-120022。

这是当前Mac arm64/debug/custom-protocol本地交付，资源内嵌，不需Vite服务器；非公证/优化发布或Master全量验收。原Master测试仍未终态且已有失败，其旧二进制不能证明本次代码全量通过。
