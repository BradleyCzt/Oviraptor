<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from "vue";
import { Plus, X } from "@lucide/vue";
import { useI18n } from "../../../i18n";

const props = defineProps<{ text: string; x: number; y: number; busy: boolean }>();
const emit = defineEmits<{ apply: [keyword: string]; close: [] }>();
const { tr } = useI18n();
const keyword = ref(props.text);
const input = ref<HTMLInputElement>();

watch(() => props.text, value => { keyword.value = value; });
onMounted(() => nextTick(() => input.value?.focus()));
</script>

<template>
  <aside class="title-rule-popover" :style="{ left: `${x}px`, top: `${y}px` }" @pointerdown.stop>
    <header>
      <div><strong>{{tr('页面隔离规则','Page isolation rule')}}</strong><small>{{tr('仅当前项目 · 先预览命中','Current project · preview first')}}</small></div>
      <button :aria-label="tr('关闭','Close')" @click="emit('close')"><X :size="14" /></button>
    </header>
    <input ref="input" v-model="keyword" maxlength="200" @keyup.enter="keyword.trim().length >= 2 && emit('apply',keyword)" />
    <p>{{tr('先查看全部命中和受保护记录，再决定是否隔离这类页面。','Preview all matches and protected records before isolating this page type.')}}</p>
    <footer>
      <button class="button ghost compact" :disabled="busy" @click="emit('close')">{{tr('取消','Cancel')}}</button>
      <button class="button primary compact" :disabled="busy || keyword.trim().length < 2" @click="emit('apply',keyword)"><Plus :size="14" /> {{tr('预览规则','Preview rule')}}</button>
    </footer>
  </aside>
</template>
