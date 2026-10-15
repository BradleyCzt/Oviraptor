<script setup lang="ts">
import { useI18n } from "../../i18n";
import ModalShell from "../../shared/components/ModalShell.vue";

defineProps<{ version: string }>();
defineEmits<{ close: [] }>();
const { tr } = useI18n();
</script>

<template>
  <ModalShell :title="tr(`Oviraptor v${version} 更新说明`, `Oviraptor v${version} release notes`)" @close="$emit('close')">
    <template #eyebrow><span class="eyebrow">RELEASE NOTES · NATIVE</span></template>
    <div class="release-notes">
      <article>
        <strong>{{tr('资产复核与规则管理','Asset review and rule management')}}</strong>
        <p>{{tr('划词隔离后保留当前资产页面、筛选和页码，只刷新列表与统计；复核等级匹配 P1/P2/P3 完整标签，组合 OR 不再越过项目和回收范围。支持拖选多行后批量确认或移入回收站，标题仍可划词。配置中心新增“页面与划词规则”，展示各项目规则及以前保存的全局规则。','Page isolation preserves the asset view, filters and page while updating results and counts. Tier filters match complete P1/P2/P3 labels; OR stays within project and trash scope. Drag rows to select and then confirm or move to trash; title text remains selectable. Configuration now lists project page rules and previously saved global text rules.')}}</p>
      </article>
      <article>
        <strong>{{ tr("按项目隔离无用页面", "Project page isolation rules") }}</strong>
        <p>{{ tr("选择示例页面，按标题特征或完整正文预览全部命中，再启用规则。全部命中都隔离，不保留代表；不同 IP、端口不影响匹配。人工状态优先保留，应用前备份原记录，规则可停用并恢复。后续入库自动应用。", "Select an example and preview all matches by title text or complete body. Rules isolate every eligible match, including representatives, across IPs and ports. Manual state takes priority. Original records are backed up; rules can be disabled and assets restored. Subsequent imports apply enabled rules.") }}</p>
      </article>
      <article>
        <strong>{{ tr("资产入库并发修复", "Concurrent asset import fix") }}</strong>
        <p>{{ tr("入库先等待写入许可，再读取历史资产，避免并发修改让任务报 database is locked。整份文件仍原子入库，失败回滚，保留人工结论、备注与已看记录；错误日志标明任务和结果文件。", "Imports acquire write admission before reading asset history, avoiding snapshot upgrade failures during concurrent edits. Each file remains atomic with rollback on failure; manual decisions, notes and opened-entry records are retained. Errors identify the job and result file.") }}</p>
      </article>
      <article>
        <strong>{{ tr("采集失败原地恢复", "Resume incomplete collection") }}</strong>
        <p>{{ tr("失败或取消的采集可从原目录继续，复用已保存的成功页，只补未完成请求。原种子和数据配置保持固定，错误与重试实时写日志；未完成采集仍明确标为失败。", "Failed or cancelled collections can resume in the original directory, reusing saved successful pages and requesting only missing results. Original seeds and data settings remain fixed; errors and retries are logged immediately, and incomplete collection remains a failure.") }}</p>
      </article>
      <article>
        <strong>{{ tr("完全相同响应折叠与探测总超时", "Identical response groups and probe budgets") }}</strong>
        <p>{{ tr("成功且完整的非空 HTTP 响应按状态码、完整正文与内容类型归组，保留全部 URL 并可展开。不同人工状态分开显示。新配置默认 96 并发、每 URL 8 秒网络总预算，超时保留记录；导出包含全部匹配 URL。", "Complete, nonempty successful HTTP responses group by status, full body and content type. All URLs are retained and expandable, with distinct manual states shown separately. New profiles default to 96 workers and an 8-second network budget per URL; timeouts retain records and exports include every matching URL.") }}</p>
      </article>
      <article>
        <strong>{{ tr("资产发现去重与状态保护", "Asset discovery deduplication and state preservation") }}</strong>
        <p>{{ tr("公司与域名命中同一访问入口时，合并发现证据后探测一次。不同协议、端口、子域名和路径分别保留；新任务仍会重新探测历史入口。保留人工结论、人工回收和已看记录。", "Company and domain hits for the same endpoint merge evidence before one probe. Different schemes, ports, subdomains and paths stay separate; new jobs recheck historical endpoints. Manual decisions, manual archives and opened-entry records are preserved.") }}</p>
      </article>
      <article>
        <strong>{{ tr("macOS 菜单栏状态修复", "macOS menu bar status fix") }}</strong>
        <p>{{ tr("最后一个资产任务结束后清除“资产更新中”，恢复正常图标；失败、取消及任务线程异常同样收尾。多个任务并行时，保持更新提示直到全部结束。", "The last asset job clears the updating title and restores the normal icon. Failure, cancellation and worker panics also release activity. Concurrent jobs keep the indicator until all finish.") }}</p>
      </article>
      <article>
        <strong>{{ tr("当前 Native 执行路径", "Current Native execution path") }}</strong>
        <p>{{ tr("当前任务使用 Native 执行路径。当前 Native JSON 与标准 SARIF 保留导入能力；旧后端专属格式不再导入。历史资料归档在项目文档中，原有数据清理需要单独确认。", "Current tasks use Native. Current Native JSON and standard SARIF remain importable; retired backend formats are rejected. Historical notes are archived in project documentation, and existing data cleanup requires separate confirmation.") }}</p>
      </article>
    </div>
  </ModalShell>
</template>
