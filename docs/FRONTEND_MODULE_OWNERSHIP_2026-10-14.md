# 前端业务模块归属与迁移边界

本轮从云端 WIP `d61454cdf73c52d190d3d4a26d6733356a6495af` 继续，不以 main 的配置或历史预览替换 WIP。这里记录前端边界；后端执行权限和存储 owner 仍以原领域实现为准。

| 归属 | 实现与契约 | 责任 |
| --- | --- | --- |
| Chat | `features/conversations/` | 独立对话、当前项目上下文、对话权限和工具活动；不接管扫描调查时间线 |
| Assets | `features/assets/components/`、`features/assets/types.ts` | 资产表、采集任务、归属、人工判定；项目关联和授权语义保持原实现 |
| Assets.Exposure | `features/assets/exposure/` | 暴露面页面、设置、复核、证据、轮次；消费资产领域的只读选择接口 |
| Assets 外部来源 | `features/hackerone/` | 保留兼容内置入口和全局 HackerOne 缓存；向项目导入仍显式选择项目 |
| Scanning | `features/sentinel/` | 保留稳定的 sentinel 内部名称；工作台、调查、结果、恢复、收据和扫描日志留在扫描领域 |
| Skills | `features/skills/` | 技能包导入、资源浏览、文本指令编辑、技能及知识目录的 IPC 和类型；由 Chat 和 Scanning 分别消费 |
| 项目空间 | `features/workspaces/` | 项目生命周期、项目对话框、项目聚合视图；`StartupStatus` 显式引用资产中断任务契约 |
| 配置 | `features/configuration/` | 原完整配置对话框与 profile/provider 类型；保留既有字段和默认值 |
| 基础设施 | `features/runtime/`、`features/environment/` | Worker、环境准备和工具管理；沙箱继续是内部执行能力 |
| 主窗口 | `shell/` | 导航、可信内置组件注册、应用设置及发布说明；默认启动 Chat |
| 可复用基础 UI | `shared/components/` | 仅 `ModalShell` 与 `InlineConfirm`；不导入领域类型、业务 API 或 Tauri 执行接口 |

`src/components/` 原有 17 个组件全部归回其 owner。组件迁移更新实际静态和动态导入以及回归测试所编译的真实 SFC 路径，未复制组件实现或增加伪入口。

`src/types.ts` 现在仅提供 type-only 兼容 re-export。128 个声明分属资产、扫描、技能目录、项目空间、HackerOne、配置、环境、Worker 和主窗口契约；非聊天消费者直接导入其 owner。保留 facade 方便既有外部消费者和并行的聊天权限修复，不产生运行时代码。

独立 Skills 之前反向依赖扫描工作台中的目录编辑器与 composable。它们现在归 Skills 所有；Scanning 导入 `useSkillsCatalog` 和 `SkillInstructionsCatalog`。23 个技能/知识目录操作由 `features/skills/catalogApi.ts` 提供，保留原命令、参数和返回值。扫描专属 trace 读取与分析仍归 Scanning；`sentinelApi` 暂时展开 Skills catalog 以兼容既有扫描消费者。

仍保留的兼容边界：`src/api.ts` 聚合原 API，`features/workspaces/api.ts` 仍提供旧项目/profile/应用设置入口，`sentinelApi` 保留目录兼容展开。这些是既有调用的过渡入口，不应继续添加无关业务。设置页面是显式组合多领域配置的产品入口，不放入 shared。下一批可按实际调用逐步收窄这些 facade；本轮没有通过更名命令或重写 Agent 实现目录隔离。

`tools/test_module_boundaries.cjs` 解析真实 Vue/TypeScript import、export 与动态 import，执行以下依赖守卫：全部相对导入存在；shared 不依赖领域或平台执行；Skills 不依赖 Scanning 或聚合 API；owner 类型不反向引用旧 types facade；App 的同步导入闭包不能包含资产、扫描、HackerOne 或 Skills 的业务 Vue 页面。原应用测试继续验证启动、项目切换、权限、扫描恢复和数据操作的实际行为。

依赖守卫和 DOM/IPC 回归不证明真实模型、目标网络扫描或桌面安装包可用；编译、具体测试结果与失败记录由集成验证报告列出。
