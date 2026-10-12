# 固定离线解析器包合同（实施中）

本合同用于新批准的 `offline-parser-bundle-v1` 适配器。旧 `offline-parser-v1` 包仍是不透明库存，不因包头相似而升级或执行。当前没有支持完整资源隔离的生产 runner；解析、缓存或批准通过均不授执行权。

## 签名与兼容

保持现有 manifest schema 1 和发布者、管理员签名域。发布者 digest 覆盖完整包；新的管理员决定显式绑定 runtime adapter、manifest hash、package digest 和 runtime digest。runtime digest 是固定运行适配器的实际部署摘要，不能用包摘要替代。新包须新批准、新任务；不得修改旧签名事实。

## 字节格式

所有长度为无符号 little-endian。头为 ASCII `OVFB0001` 八字节、u16 version（必须为 1）、u32 file count（1–16）。每项顺序为 u16 路径长度、u64 内容长度、u8 regular mode、32 字节内容 SHA-256、路径字节、内容字节。mode 仅允许 0（只读数据）或 1（入口代码），没有链接、设备、压缩、owner、xattr 或任意权限字段。

单文件最多 16 MiB，完整编码最多 64 MiB，路径最多 240 字节、8 层。使用可移植 ASCII 路径子集；拒绝绝对路径、空段、点段、反斜线、冒号、NUL、Windows 保留名称及尾点空格。大小写不敏感重复或文件/目录前缀冲突拒绝。恰有一个入口代码项，其路径逐字等于原 manifest entrypoint。长度计算必须 checked；逐项摘要与最终 EOF 精确核对。解析不分配声明大小的内容缓冲，也不将成功解析作为签名证据。

## 生产来源与部署验收

固定包缓存根为 App 配置目录下 `tool-supply-cache`，候选布局沿用 `id/candidate_cache_key/manifest.json` 与 `package.blob`。批准服务已接真实准备 owner 下的惰性发布、原批准事务与来源复核；原批准缺缓存仅同一完整签名请求可补，现有冲突不覆盖。发布、来源及 owned writer 联合测试 11/11 通过，安装 App 全链尚待验收。缺失缓存必须拒绝，不能由数据库批准记录推断包已存在。

新适配器批准来源到包解析的四项真实签名与发布测试已通过，包括旧适配器不升级、撤销、签名损坏和已签但格式无效的包。安全解包目标固定为 App 配置目录下 `tool-supply-deployed`；三项实际签名部署测试已通过，批准服务已接线，但安装 App 全链仍待验收。部署证明不授工具执行权。

先由实际管理员配置和当前批准读桥取得原缓存来源，保持目录及文件 inode、NOFOLLOW、regular/nlink、权限、原 manifest 和完整包摘要校验。解析器只能产生格式证明；来源证明必须另由生产工厂签发，禁止 caller path 或支持 bool 替代。

安全部署须在应用私有目录创建独占 staging；使用目录句柄逐层创建和打开，拒绝链接，文件 CREATE_EXCL。只读数据权限 0400，入口 0500。逐文件复制摘要、fsync，完整核验后同文件系统原子发布并 fsync 父目录；已存在目的地只能精确核验后复用，不能覆盖。失败仅清理本次拥有的 staging，保留原库存，不触及资产目录。

后续 runner 仍须证明固定运行时部署摘要、原角色授权、输入范围、CPU/内存/进程/墙钟/输出硬限制、取消和清理、真实结果签据及模型观察。当前 macOS 硬内存约束未验证，执行仍拒绝。Linux VM+cgroup 是候选方案，尚无批准镜像或实际 runner 验收。

## 必须保留的证据边界

格式正反测试、原批准来源测试、安全部署测试、隔离 worker 测试、实际模型工具循环和 App 验收分别报告。不能把其中任一层通过视为完整 Tool Broker 或 Master 完成。
