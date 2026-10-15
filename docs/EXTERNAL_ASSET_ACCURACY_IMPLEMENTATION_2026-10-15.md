# 外部资产准确性与项目精确统计实施记录（2026-10-15）

云端回归已完整完成。受测源码为 clean `34e802a6f8d4fc093efa1d3ee732daf767d0df44`；Linux/default-feature opt1应用库、全量Node、真实Chromium合成IPC、构建及独立退出/事后输入核验均已通过。平台GUI、用户数据及live provider的限制见后文。

## 来源、范围和提交

沿 `main 3102693abc135f4a2b986caca851bbc935378f0b` 中已保留的云端WIP与本地移交整合继续工作，独立分支 `refactor/external-asset-accuracy-20261015`。原始WIP基线为 `d61454cdf73c52d190d3d4a26d6733356a6495af`；本地基准为 `8fddee2321b313a8a6372d0895dd4c1f533ba57c`。过滤恢复根 `0d0f995a2b7312c28589c1d952ae33ffcbe045db` 不作为共同祖先。Library移交包为`Oviraptor-local-source-8fddee2.tar.gz`、10,844,046字节，SHA256 `fc6448afee2399a988679ff64db8d95a2c5b7439e78ecd0bed05d94535fa24af`；原恢复已先检查脚本再在独立目录校验，27个tracked修改与24个新增按owner语义映射到52个目标路径。来源恢复与51项增量整合详见 `LOCAL_INCREMENTAL_INTEGRATION_2026-10-15.md` 及其映射；本轮未重新导入或丢弃它们。

用户先要求文档评审，`6920ec9` 保存该稿；随后明确要求实施阶段1–4，再把实测聊天统计错误纳入阶段4。阶段5–6的结构化AI分析、审批、执行编排没有实施。实施期间在独立分支工作；用户随后明确授权最终测试通过后经正常PR合并main，替代此前不合并main约束。最终合并身份由实际发布回读记录；在此之前不宣称已合并。未改用户Mac、本地源码或原DMG，没有访问真实模型、provider或资产目标。全部测试使用合成来源、临时SQLite、fake gateway或本机HTTP fixture。

| 提交 | 具体范围 |
| --- | --- |
| `3ab0b40` | 27个后端文件；严格分页覆盖、原始观测/协议身份、精确归属、范围绑定missing、有界脱敏收据及诊断日志；相关SQLite/HTTP临时fixture测试。采集、导入与持久化合同相依，作为一致后端批次提交。 |
| `45d5434` | 10个前端/测试文件；覆盖与执行分别显示、历史探测、实际配置、存储范围、真实SFC和Chromium验证。 |
| `d9e2dfb`、`7ff60de` | 先补第10.3节统计/搜索/权限合同，明确回收仅聚合、不开放行内容。 |
| `6398385` | 9个后端文件；Assets精确统计服务、Chat工具、分页及授权边界、20个真实SQLite/fake gateway用例。 |
| `5b4a5b6` | 4个前端/测试文件；未知/失败/旧快照、原URL与展示组分母、unknown状态、点击后计数重读和统计活动标签；原62+新增12个SFC用例。 |
| `e5fed60` | 3个测试相关文件，45增/6删；修正实际定向回归发现的目录版本夹具、v2/www精确断言，分别校验2个逻辑来源和6条查询回执，生产代码不变。 |
| `34e802a` | 仅broker正例夹具，68增/1删；事务内一致的有效120秒期限，续租前后期限/epoch/fence断言，撤销与真实过期拒绝的零写检查；原15断言、其他测试字节保留，生产未变。 |

相对main310的唯一变更为46个源码/测试路径与2篇文档，共48；相对评审设计6920为46个源码/测试路径与1篇设计更新，共47。此计数不把跨提交重复修改的文件累加；最终证据文档批次另计。完整路径、owner与before/after blob见交付来源复核。

## 模块归属与行为

Assets持有实体身份、来源/探测观测、项目成员、归属、采集、库存查询、导出与页面规则。新增统计实现在 `assets/statistics.rs`，由 `commands::assets::statistics` 窄服务对外；复用真正的 `asset_filter` 与查询验证。Chat只持有会话、授权、受控只读工具与有界数据投影，不新建资产SQL、不直接调扫描writer。

Scanning继续持有准入、预算、租约、扫描、原Native Agent与沙箱执行；Skills持有包完整性/导入/资源/撤销；Workspace持有项目身份/归档/删除保护；Shell负责默认Chat、内置资产与扫描入口、跨业务展示；配置保持原profile与WIP默认值。shared/common不加入这些业务类型或流程。原有连贯业务内部include仍在自身namespace，未为拆目录重写通用Agent。细项见 [architecture.md](architecture.md)、[前端归属](FRONTEND_MODULE_OWNERSHIP_2026-10-14.md)、[Rust边界](RUST_BUSINESS_MODULE_BOUNDARIES_2026-10-14.md)及[最终整合映射](local-incremental-20261015/final-module-contract-map.json)。

| 业务owner | 核心路径 | 允许的依赖及本轮边界 |
| --- | --- | --- |
| Shell | `src/shell`、`src/App.vue`、`application/model.rs` | 组合owner API、默认聊天和可信内置入口；不把业务实现移入全局components。 |
| 项目空间 | `features/workspaces`、`commands/workspace_projects.rs`、`workspaces/model.rs` | 项目身份及关联记录保护；向Chat绑定项目，不提供扫描执行权。 |
| Assets | `features/assets`、`commands/assets.rs`、`assets`、`commands/asset_ownership.rs`、`commands/ownership*`、采集jobs | 持有库存/筛选/归属/采集观测；统计是其窄只读服务；不以UI展示组代替原始成员。 |
| Scanning | `features/sentinel`、`scanning.rs`、`scanning`及原Native runtime | 原扫描、调查、预算、租约和执行，按命名端口消费其他owner；不重写通用Agent。 |
| Chat | `features/conversations`、`conversations` | 会话授权、provider披露和工具投影；调用Assets统计/分页服务及Skills授权读口；不持有资产写口。 |
| Skills | `features/skills`、`skills_catalog`、`skill_packages` | 包来源/完整性/导入/资源/撤销；扫描producer与规则仓库仍归Scanning。 |
| Configuration/Environment | `features/configuration`、`features/environment`、对应Native owner | profile保留未知字段与原WIP默认值；显式组合模型/worker/provider；沙箱仍是内部运行能力。 |
| Shared/基础设施 | 复用UI原语、窄模型gateway、DB/日志机制 | 不持有资产/扫描/技能DTO或业务流程；全局types只保留兼容type re-export，领域声明归owner。 |

同一业务内仍有旧 `include!`，不宣称全部消除；扫描已是独立namespace，其跨owner依赖通过命名端口，未恢复旧commands共享作用域。

原WIP六项审查结果继续保留：普通Chat在`agent_enabled=false`时独立验证会话/项目；provider同意绑定完整endpoint/model/deployment；Skills按内容、资源hash及持久generation撤销；实际WIP与当前选择上限为128，可容纳36内置技能，未把未经核验的“24”当事实；资源按UTF-8 4KiB页读取，完整工具JSON预算32KiB；缺失的`skill_packages/tests.rs`与`conversations/access/tests.rs`已用安全夹具恢复并进入实际Cargo。细节及旧基线16KiB/24KiB的核验见[模块历史报告](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md)与[本地整合最终报告](LOCAL_INCREMENTAL_INTEGRATION_2026-10-15.md)，本轮新full结果另列，不借旧结果代验。

采集执行成功与覆盖证明分开；短页、未知/变化total、重复页/记录、字段降级、非法记录、脱敏改写定位字段、预算/额度/取消都不能错误变绿。missing必须完整证明且历史成员属于同一collection binding，不能把不同查询范围或无范围历史判缺失。

明确非Web与unknown协议不靠端口猜成HTTP身份；源字段采用整份观测的稳定选择，来源/探测时钟独立。重放不写now，真实positive原探测时间独立聚合为last_alive；新IP等变化下旧probe保留历史原因，来源IP不冒充实际HTTP peer。未知时间旧CSV保留原字段更新兼容，但时间仍unknown，不赋予collection coverage或missing证明，也不声称确定新鲜度。

归属精确保留www，来源domain/host冲突有反证；旧已丢失www的学习规则不猜测升级，显式人工复核才按新版本记录。人工归属、授权、回收及规则效果保留。

注册的collection原文/CSV/TXT/控制元数据按逻辑文件长度计费，默认每run512MiB、每project1GiB、已登记global2GiB、4096页、7天；原子发布/锁/临时峰值/保守崩溃恢复/tombstone均接真实生产writer。未知未登记旧目录不自动接管清除。回收计数代表durably tombstoned artifacts，物理剩余字节独立测量。日志默认5000/run、100000global、30天，裁剪计数持久可见，不裁剪asset_events/ownership人工审计、不宣称VACUUM压缩SQLite。

## 聊天数量与搜索口径

`assets_get_stats` 默认精确统计全部未回收原始成员，独立提供projectTotal（含回收）、activeTotal、trashedTotal、unknownArchiveState与filteredTotal。decision/probe/HTTP每个互斥维度包含unknown/unclassified并各自合计至filteredTotal；排除轴以filteredTotal为分母、可能重叠，不能跨维度相加；rejected/not_applicable不等同回收。asOf表示当次保存库存读取，不证明实时在线，也不是持久快照。

`assets_search` 返回total/returned/limit/hasMore/nextCursor/scope/asOf，ID升序游标绑定项目及全部规范化筛选。最多20条仍是样本，不通过ID跨度、放大limit或模型遍历估总量。新搜索明确统一到资产页八字段参数化LIKE（含%/_通配），有意替代旧host/title字面搜索。旧分页JSON属性保留，增加边界信息；完整32,768字节守卫、固定JSONencoded scalar预算和先脱敏后截断保留。asset-page游标前缀避免被既有秘钥脱敏器误遮蔽，未改变脱敏规则。

项目只能从服务端会话绑定，仍受read_assets、provider disclosure、endpoint/access revision与dispatch前后live复查约束。回收内容搜索/单条读取继续拒绝，仅可返回明确聚合数量。无资产授权的project_context不附带统计。

UI展示组total、原始memberTotal、忽略人工决定分组的summary.all分别说明。没有读取成功的0不能当真实零；失败、加载、旧筛选快照、缺memberTotal与空probe状态明确unknown；默认review/browser_review/折叠和查询键不变。

## 实际完成的验证

| 项目 | 实际结果 | 源及限制 |
| --- | --- | --- |
| 新all-targets | exit0，87.398s | clean34e802a；native1839/audit2265/tracked2632/union2636，完整输入前后不变 |
| 新opt1测试构建 | exit0，414.610s | clean34e802a；实际Cargo产物SHA256 919a724e414ced1ac04a2e9b8bcdd82558345d4ccdea823e4aa3fccd985bb170；opt1/debug0、debug_assertions/overflow_checks=true、features=[] |
| 新原生定向 | 354pass、3原ignored、0fail，exit0，Rust16.22s | 原356项严格加broker共357；实际3828=3725旧名+103新增，全部103新增实际通过，每项恰一次；独立supervisor与tool实际退出0 |
| 新原生无过滤全量 | **3821pass、7原ignored、0fail、0filtered，exit0**；Rust1687.51s / command1687.668s | clean34e802a；3828各恰一次=3725旧名+103新增；独立supervisor及实际tool session85779均退出0；全部输入/binary/helpers前后不变；独立事后核验通过 |
| 全量Node | 1662=1650pass+12既有skip，0fail，exit0，162.339s | clean34e802a；原1588+74精确occurrence与12skip/reason保留；原97测试源逐字同基线 |
| SFC定向 | 74/74，exit0，4.545s | 原执行HEAD为7ff阶段工作树；随后提交的前端源码逐SHA相同；34e全Node再次包含全部74，原62+新增12，未改旧断言 |
| npm build | exit0，18.493s | 原执行HEAD为7ff阶段工作树；409前端/tools输入前后相同，最终34e前端逐SHA匹配，三组浏览器复用dist前再次核对全部路径/哈希 |
| 真实Chromium准确性集合 | 16/16，exit0，28.852s | clean34e802a；409构建输入及dist逐SHA匹配后复用；合成IPC；0runtime/unknownIPC/external fetch/blocked；2636输入与dist稳定；不是实际Tauri/用户库/模型验收 |

另独立重跑原业务Chromium基线：`legacy28-certified-01` 28/28、exit0、65.136s（clean34e802a）；409构建输入、dist及2636全输入前后稳定。它与16场景是独立集合，存在重叠，不能相加声称44个唯一样本。

全部命令、输入哈希、case occurrence、原ignored/skip、原二进制保护及失败历史已在[最终机器索引](asset-accuracy-20261015/final-verification.json)和[完整证据归档索引](asset-accuracy-20261015/PUBLICATION-INDEX.md)汇总。原生定向、broker单例和84次重复、74个SFC均是对应完整套件的子集或重复，不与full/Node累加为独立覆盖；单纯列举测试不算通过。

## 保留的失败与适用限制

e5第一次原生无过滤full01观察到1项broker用例失败后安全中止：实际1559条=1551ok/1FAILED/7原ignored，211.33s，对实际native进程发送SIGTERM(15)，subreaper映射为143；suite/supervisor/tool实际退出143，summary=null/accepted=false。没有完整footer，不能写成完整full汇总；原失败panic正文未取得。相同binary独立exact1次、顺序100次、8进程160次全部通过，仅为诊断，不覆盖原失败。原测试及5项相关生产/夹具文件与main310逐blob相同；正例SQL把3表各自设为秒精度now+1秒，实际有效窗口可短于1秒，现场DB状态与续租前失效一致，未把目录mtime误当精确SQL时钟。`34e802a`仅修正可证明过窄的正例fixture：一次IMMEDIATE事务计算120秒deadline、绑定3表并要求仍live且<=120秒；真实续租后仍须>300秒，并验证4处同一更晚期限/epoch/fence不变。15个原断言逐字保留，新增撤销和真实过期续租拒绝的全表快照及total_changes零写验证；没有修改生产TTL、timeout或过期续租保护。新34e full02已实际完整通过且通过独立事后核验；这不改变原panic正文未取得的事实。

5b阶段真实定向回归完成356项：347pass、6fail、3原ignored，exit101/15.18s；20项新增统计/聊天用例全部通过，但整体accepted=false，未启动full。3项目录fixture遗漏实际DDL必填directory_version；2项旧断言与批准的v2精确www语义不符；另1项把2个逻辑种子与6条独立查询回执混算。`e5fed60`仅修测试：目录状态、实体、标识符一致置v1；保留PSL/冲突/人工授权断言；来源同时断言逻辑2、回执6、各3条、独立hash/页/原时间/定位/无截断，未放松为任意非零。修正后的e5新二进制定向356项已真实完成353pass/3原ignored/0fail，不用5b失败集内20个通过替代最终结果；e5的full01后来失败中止见上文，最终34e的full02已独立完整通过。

原业务28基线第一次在e5原样实跑为25pass/3fail/exit1/73.720s：旧“Probe queue”标签和两处旧folded footer字面断言，实际UI已按批准合同分别显示“Last probe classification”、2组/53原URL、3组/4原URL；未完成这些断言后的交互不能在01轮记通过。仓外QA仅3literal精确适配，反向还原整个runner逐字节等于01，fixture/名称/顺序/其余断言不变；02轮全部28实际通过，其中17的52成员50+2分页、单成员确认、53/51导出，以及23的401/402打开标记后同锚点重读401/403均实际执行，旧失败保留。

第一轮45 all-targets Cargo退出0，但4个首次生成schema触发输入集合守卫拒绝；保留该receipt后同源重做并通过。45全Node第一轮仅Chromium setuid sandbox启动失败；不改系统权限或断言，使用既有Linux CI环境开关后完整重做通过。浏览器旧轮与本轮QA导航/selector失败原样保留，修测试导航后完整重跑，未作为应用通过掩盖。旧45 check/build/1650Node和62SFC/8浏览器只属于旧阶段，不覆盖新增统计；原native full03仍是历史基线，不转记到本轮。

真实provider分页稳定性、实际模型是否遵循计数说明、用户数据库、真实桌面安装与Mac/Windows包尚未验证；使用合成服务/SQLite/fake模型/Chromium IPC。历史误合并已丢失字段、无时区时间、旧www规则不能无损恢复。本地包排除的3个测试文件和5个旧夹具版本未取得；现有云端版本及安全替代数据的通过，不代表与那些本地排除版本一致。原主动probe的项目准入/重定向专项边界仍保持独立审查，本轮没有扩大目标访问或宣称该问题已闭合。存储预算不覆盖probe/refined输出、整SQLite、用户文件或未登记目录；低层pin不是阶段5持久分析快照业务。

原1.1.66 DMG继续对应main310，SHA de8375c171d262d59ddcb1abfc97a30cb5be24825525e78c1571dee2f848ff5d；本轮未替换/重新包装它。新源分支须独立构建新包。

## 发布与可获取证据

通过正常[PR #7](https://github.com/swyiic/Oviraptor/pull/7)合并main的授权已收到；保留merge commit和全部分批历史。受测源码之后的报告/归档批次仅位于docs，最终发布时核对其与受测源码的差异，并回读远端main、PR状态、父提交和文件树；本文不自引用未来文档或合并SHA，最终身份随PR发布回读提供。原DMG没有重打。

[来源与模块路径复核](asset-accuracy-20261015/delivery-source-review-34e802a.json)列出唯一46个源码/测试路径及before/after blob；[归档set ledger](asset-accuracy-20261015/publication-set.json)提供每片SHA与覆盖清单。完整日志、列表、逐例账本、before/after输入、同轮失败及截图保留在归档，原绝对路径仅表示采集位置，归档内用安全相对路径保存。打包器及校验说明随附，工具自身9项合成自测不计应用测试。

发布包保守排除paused WIP原始patch（疑似URL userinfo形态，仅保留路径/SHA/字节数/原因）以及保留的二进制缓存和未跟踪源码包；云端原件及其proof保持原样，业务源码已在可审查Git提交中。实际失败日志与回归账本未因此省略。隐私扫描为启发式检查，截图沿用合成数据及实际视觉审查记录。
