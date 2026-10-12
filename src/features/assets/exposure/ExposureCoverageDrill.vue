<script setup lang="ts">
// P1 UI 闭环 — 覆盖矩阵钻取面板：时间窗口、单位维度（归属模块逐资产投影）、只看缺口、工作项分页。
import { computed, watch } from "vue";
import { exposureIncidentApi } from "./api";
import { useExposureCoverage } from "./useExposureCoverage";
import { COVERAGE_WINDOWS, type CoverageRowView } from "./incidents";

const props = defineProps<{ projectId?: number; rows: CoverageRowView[]; source?: string }>();
const emit = defineEmits<{ notify: [kind: "success" | "error" | "info", text: string]; close: [] }>();

const coverage = useExposureCoverage({
  projectId: () => props.projectId,
  api: exposureIncidentApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const { sourceKey, windowDays, unitEntityId, gapsOnly, view, items, loading, error, available, drill, setWindow, setUnit, setGapsOnly, loadMore, close } = coverage;

watch(() => [props.source, props.projectId] as const, ([source]) => {
  if (source) void drill(source);
  else close();
}, { immediate: true });

const row = computed(() => props.rows.find((item) => item.sourceKey === sourceKey.value) ?? null);
</script>

<template>
  <section v-if="sourceKey" class="panel exposure-coverage-drill">
    <header>
      <strong>钻取：{{ row?.label ?? sourceKey }}</strong>
      <small>{{ row?.window }}</small>
      <button data-action="close-drill" @click="close(); emit('close')">收起</button>
    </header>
    <p v-if="!available" class="exposure-gap">覆盖钻取接口尚未注册：这里显示的不是“没有缺口”</p>
    <p v-else-if="error" class="exposure-gap">钻取读取失败：{{ error }}（不是“没有缺口”）</p>
    <div class="exposure-drill-filters">
      <label>
        <span>时间窗口</span>
        <select data-field="window" :value="windowDays" @change="setWindow(Number(($event.target as HTMLSelectElement).value))">
          <option v-for="option in COVERAGE_WINDOWS" :key="option.days" :value="option.days">{{ option.label }}</option>
        </select>
      </label>
      <label v-if="view && view.unitState === 'reported'">
        <span>单位</span>
        <select data-field="unit" :value="unitEntityId ?? ''" @change="setUnit(($event.target as HTMLSelectElement).value === '' ? null : Number(($event.target as HTMLSelectElement).value))">
          <option value="">全部单位</option>
          <option v-for="unit in view.units" :key="unit.entityId" :value="unit.entityId">{{ unit.line }}</option>
        </select>
      </label>
      <label class="exposure-check"><input data-field="gaps" type="checkbox" :checked="gapsOnly" @change="setGapsOnly(($event.target as HTMLInputElement).checked)" />只看缺口</label>
    </div>
    <template v-if="view">
      <h4>{{ view.windowLabel }}的轮次</h4>
      <ul><li v-for="(line, index) in view.rounds" :key="index">{{ line }}</li></ul>
      <h4>单位维度</h4>
      <p class="exposure-gap">{{ view.unitNote }}</p>
      <ul v-if="view.units.length"><li v-for="unit in view.units" :key="unit.entityId">{{ unit.line }}</li></ul>
      <h4>工作项（已显示 {{ items.length }} / 共 {{ view.total }}）</h4>
      <div v-for="item in items" :key="item.id" :class="['exposure-drill-item', { gap: item.gap }]">{{ item.line }}</div>
      <footer v-if="view.hasMore" class="exposure-more">
        <button data-action="more-items" :disabled="loading" @click="loadMore">加载更多工作项</button>
      </footer>
    </template>
    <div v-if="loading" class="exposure-empty">正在加载…</div>
  </section>
</template>
