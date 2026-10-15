<script setup lang="ts">
import { computed } from "vue";
import type { Asset } from "../types";
import { assetObservation } from "../observationPresentation";
import { useI18n } from "../../../i18n";

const props = defineProps<{ asset: Asset }>();
const { tr } = useI18n();
const evidence = computed(() => assetObservation(props.asset));
const label = computed(() => ({
  candidate: tr("候选 · 尚无探测结果", "Candidate · no probe result"),
  skipped: tr("候选 · 探测已跳过", "Candidate · probe skipped"),
  unverified: tr("历史探测 · 本次采集尚未验证", "Historical probe · collection not yet verified"),
  historical: tr("历史探测 · 时间未知", "Historical probe · time unknown"),
  unknown: tr("探测时间待核", "Probe timing unknown"),
  recorded: tr("最近探测记录", "Last recorded probe"),
})[evidence.value.state]);
const known = (value: string) => value || tr("未知", "Unknown");
const historyMessage = computed(() => ({
  endpoint_changed: tr("来源入口已变化，旧探测不能证明当前入口可用。", "The source endpoint changed. The old probe does not verify the current endpoint."),
  collection_after_probe: tr("这次取得的来源页面晚于探测记录，尚未看到新的探测证据。", "The source page was received after the probe. No newer probe evidence is recorded."),
  unknown_probe_time: tr("旧探测时间缺失或无法比较，不能视为本次已经验证。", "The old probe time is missing or not comparable. It does not verify this collection."),
} as Record<string, string>)[evidence.value.historyReason] || "");
</script>

<template>
  <details class="asset-observation" :data-observation-state="evidence.state">
    <summary :title="tr('查看来源报告、页面取得和探测记录的时间；不将入库时间当作探测时间。','View source observation, page receipt and probe times. Import time is not a probe time.')">{{label}}</summary>
    <dl>
      <dt>{{tr('来源报告时间','Source observation')}}</dt><dd data-observation-time="source">{{known(evidence.sourceObservedAt)}}</dd>
      <dt>{{tr('采集页面取得时间','Collection page received')}}</dt><dd data-observation-time="received">{{known(evidence.collectionReceivedAt)}}</dd>
      <dt>{{evidence.state === 'skipped' ? tr('最近探测尝试','Last probe attempt') : tr('最近探测时间','Last probe')}}</dt><dd data-observation-time="probe">{{known(evidence.probeCheckedAt)}}</dd>
      <dt>{{tr('探测记录 IP','Probe IP record')}}</dt><dd data-observation-time="observedIp">{{known(evidence.probeObservedIp)}}</dd>
      <dt>{{tr('探测记录入口','Probe endpoint record')}}</dt><dd data-observation-time="observedEndpoint">{{known(evidence.probeObservedEndpoint)}}</dd>
    </dl>
    <small v-if="historyMessage" data-observation-history>{{historyMessage}}</small>
    <small v-if="evidence.probeEndpointBasis === 'legacy_candidate_endpoint'" data-observation-endpoint-basis>{{tr('此入口取自旧候选记录，缺少原探测输入证据。','This endpoint comes from a legacy candidate record; the original probe input is unavailable.')}}</small>
    <small v-if="evidence.probeObservedIp">{{tr('该 IP 来自探测记录或输入，不代表已核实远端连接 IP。','This IP comes from the probe record or input; it is not a verified remote peer address.')}}</small>
    <small>{{tr('来源与探测均是记录时的观测，不保证现在仍可访问；来源标签不等于确认归属或授权。','Source and probe results describe their recorded observations, not current availability. A discovery label is not ownership or permission.')}}</small>
  </details>
</template>

<style scoped>
.asset-observation { margin-top:4px; font-size:9px; line-height:1.45; color:var(--muted,#667283); white-space:normal; }
.asset-observation summary { cursor:pointer; overflow-wrap:anywhere; }
.asset-observation[data-observation-state="unverified"] summary { color:#9a5b0d; }
.asset-observation dl { display:grid; gap:2px; margin:6px 0; }
.asset-observation dt { font-weight:600; }.asset-observation dd { margin:0 0 3px; overflow-wrap:anywhere; }
.asset-observation small { font-size:9px; display:block; overflow-wrap:anywhere; }
</style>
