# 归属门禁 O1：项目预填闭环交付（2026-10-08）

依据：`NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md` §4、§5、§11 第 2 步、§12。本轮未 commit/push，未写真实用户库，未改任何资产归属/授权；O0（页面代次、草稿、保存 CAS、成员资格与结论版本）由并行改动维护，本轮只在其上衔接。

## 1. 交付内容

**唯一共享组织目录**（`db_schema.rs`，纯新增 `CREATE TABLE IF NOT EXISTS`）
- `org_directory_state`（全局目录版本）、`org_entities`（group/headquarters/province_company/city_branch/county_branch/professional_subsidiary/overseas_entity/supplier/unknown；`normalized_name` 唯一；`region_code` 为管理单位地域，与服务/授权地域分开）、`org_entity_identifiers`（简称/英文名/历史名/品牌/官方域名/共享域名，带 source/sourceRef/版本）、`project_org_bindings`（集团总账或单位项目，`binding_version` CAS）、`org_directory_changes`（目录审计，`via_project_id` 仅为出处）。
- 目录只记录组织事实，不携带授权；`exposure_entities` 当前不存在，后续 exposure 应只读引用 `org_entities.id`，不得另建目录（§10）。

**IPC**（`commands/ownership_directory.rs`，已在 `lib.rs` 注册）
- `get_project_ownership_bootstrap`：已保存档案、目录预填及逐字段来源（saved/directory/none）、绑定、目录版本、范围实体、仅本项目授权计数、dataRevision、readiness（rules_only、model_not_ready、directory_unbound、profile_unsaved）。
- `preview_project_ownership_suggestions`：只读。按当前门禁范围 company 标签聚合（规范化全/半角与空白，不删法人后缀），每项含 value/entityId/source/支持行/冲突行/examplesRef/directoryVersion/policyVersion/dataRevision/状态（saved/directory_confirmed/pending/conflict/unknown）。
- `save_project_ownership_directory`：IMMEDIATE 事务内校验目录版本 + 项目绑定版本，任一不一致整批拒绝；拒绝 CIDR/IP/ASN/域名作法人、同名实体重复、别名/官方域名指向两个单位（共享域名允许多单位）、单位项目绑定非本集团单位。只写目录/绑定/审计，不写 decisions/rules。

**确定规则（§5）**
- 省公司仅按法定名称结构“集团X有限公司”识别；市分支按“…有限公司X分公司”且上级只按名称前缀匹配已确认实体，不按地名。
- 括号城市（中移（苏州）/（杭州））只作所在地线索，候选上级为集团，绝不归入江苏/浙江省公司。
- CIDR/IP/ASN 为网络线索、company 为域名（如 cmi.chinamobile.com）为域名线索，状态 unknown，不生成法人。
- 完整“…集团有限公司”标签作为集团候选；仅有前缀时给出前缀候选并要求人工输入正式名称。
- 项目标题不再预填为法人名称（`read_ownership_profile` 未保存时 legal_name 为空）；标题只作“待接受建议”。
- 域名建议仅对已匹配本项目范围目录实体的标签，按精确主机聚合，只建议不生效。

**评估衔接**：`assess_asset_ownership` 对已绑定项目额外使用范围内目录实体名称/别名、精确主机官方域名、共享域名作线索；结果仍为 attributed / authorization_status=unknown / exposure_eligible=0，理由写明目录实体出处。集团总账范围=集团及成员；单位项目=该单位及下级。

**预填优先级**：已保存档案 → 前端未保存草稿 → 已确认目录（`get_asset_ownership_profile` 未保存时返回目录预填，版本仍为“未保存”，expectedVersion 仍为 0）→ 空。jurisdictions 不从目录复制。

**前端**
- 新增 `src/components/OwnershipProjectSetup.vue`、`src/features/assets/ownershipDirectoryApi.ts`（未改 `api.ts`/`types.ts`）。首屏：项目、项目类型、关联集团、单位、范围摘要（仅本项目授权计数）与“查看建议”；接受建议只进入按项目保留的草稿；保存带草稿创建时的目录/绑定版本；冲突保留草稿，可“保留草稿改为基于最新版本”或放弃；所有响应按 projectId + 代次丢弃过期结果；保存在切项目后返回不写新项目页面。
- `AssetOwnershipGate.vue` 最小接入：挂载上述卡片；原档案表单收入“高级：主体档案、别名、域名、排除项”折叠区（摘要行显示草稿/冲突）；目录保存后仅在“档案未保存且无草稿改动”时刷新目录预填；两处文案改为“项目标题不作为法人名称”。

## 2. 验证
- Rust：`ownership_directory_o1_tests` 7 项 + 既有 `asset_ownership` 测试，`cargo test --lib ownership` 全过；`cargo clippy --lib --tests` 无本轮文件告警；新文件已 rustfmt（未格式化其他文件）。
- 前端：`npx vue-tsc --noEmit` 通过；`node --test tools/test_ownership_project_setup.cjs`（6 项：旧响应丢弃、接受只进草稿且标题/网络/域名/冲突不可接受、CAS 与冲突保草稿、按项目草稿与切项目后迟到保存、批量接受只进草稿、门禁刷新保留目录草稿）+ `tools/test_asset_ownership_gate.cjs`（11 项）共 17/17。
- 完整 `cargo test --lib`：2900 通过 / 21 失败，失败均为 Native agent_tests 且与归属无关；详见 `NEST_OWNERSHIP_GATE_O0_O1_CLOSURE_2026-10-08.md` §4。

## 3. 未完成 / 待定
- 目录仅支持新增；实体改名、停用、合并/拆分、CSV/JSON 批量导入未做。
- 未对真实库做只读试跑（未定位到实际库文件）；fixture 覆盖了项目 3/15 的代表性标签（苏州/杭州子公司、省公司、CIDR、ASN、域名型 company）。
- 无公共后缀库，域名建议只做精确主机；子域/租户范围仍需人工（O2/O3）。
- O2（后台分析任务、组分页、批量预览/确认与审计）、O3、O4 未开始；“确认归属并授权”按钮语义未拆分（O0 已在页面明示影响）。
