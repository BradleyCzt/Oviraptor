<script setup lang="ts">
import type { WorkbenchMode } from "./useWorkbenchTaskCreation";

const props = defineProps<{
  mode: WorkbenchMode;
  modelValue: number | undefined;
  tr: (zh: string, en: string) => string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: number | undefined] }>();

function update(event: Event) {
  const value = (event.target as HTMLInputElement).value;
  emit("update:modelValue", value === "" ? undefined : Number(value));
}
</script>

<template>
  <label class="field">
    <span>{{ mode === 'web'
      ? props.tr("费用预算参考（USD，可选）", "Budget estimate (USD, optional)")
      : props.tr("费用上限（USD，可选）", "Budget cap (USD, optional)") }}</span>
    <input :value="modelValue ?? ''" type="number" min="0.01" step="0.5" placeholder="—" @input="update" />
  </label>
  <p v-if="mode === 'web'" class="form-hint span-two" role="note">
    {{ props.tr(
      "当前 Web 任务按 token、请求与时间限额运行；该 USD 值是预算参考，尚不能限制供应商的实际美元账单。",
      "Web tasks currently run under token, request and time limits. This USD value is a budget estimate; it does not cap the provider's actual USD bill.",
    ) }}
  </p>
  <p v-else class="form-hint span-two" role="note">
    {{ props.tr(
      "源码多智能体尚无可信的美元计费账本：填写 USD 上限会阻止创建任务。只有主动清空该值，才允许按 token 与时间限额运行；这不保证实际账单金额上限。",
      "Source agents have no reliable USD billing ledger yet. A USD cap blocks task creation. Clear it explicitly to run under token and time limits; actual billing is not capped in USD.",
    ) }}
  </p>
</template>
