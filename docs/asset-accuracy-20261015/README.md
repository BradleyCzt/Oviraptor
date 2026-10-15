# 资产准确性与精确统计验证证据

受测源码：`34e802a6f8d4fc093efa1d3ee732daf767d0df44`。应用测试范围为 Linux/default-feature 库与合成 IPC。

- 原生全量：3821通过、7原ignored、0失败、0过滤；全部3828项各一次。
- Node：1650通过、12原skip、0失败；98文件/1662项。
- 真实Chromium两个独立集合：16/16和28/28；不相加为44个唯一场景。
- [最终机器索引](final-verification.json)、[独立审查](independent-review.json)、[来源归属](delivery-source-review-34e802a.json)。
- [归档索引](PUBLICATION-INDEX.md)、[分片SHA与覆盖清单](publication-set.json)、[打包器说明](PACKAGING.md)。

9个分片包含完整实际日志、输入、账本、失败与截图。各分片有独立安全相对根及内部SHA256SUMS，须按索引分别解包，避免将不同分片根混作一个文件夹。原绝对路径为采集出处，实际字节位于归档的对应相对路径。

在本目录验证所有分片（Python标准库，无原云端路径依赖）：

```sh
python3 - <<'PYVERIFY'
import json, subprocess, sys
from pathlib import Path
for a in json.loads(Path('publication-set.json').read_text())['archives']:
    subprocess.run([sys.executable, 'package_evidence.py', 'verify', '--output', a['file'], '--expected-sha256', a['sha256']], check=True)
PYVERIFY
```

packaging的9项合成自测不计应用测试。原ignored/skip及本地移交排除文件、真实provider/模型/平台GUI、原probe专项限制见[实施报告](../EXTERNAL_ASSET_ACCURACY_IMPLEMENTATION_2026-10-15.md)。发布身份及最终main合并通过[PR #7](https://github.com/swyiic/Oviraptor/pull/7)回读。
