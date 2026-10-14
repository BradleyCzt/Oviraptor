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
      <div><strong>{{tr('加入内容规则','Add content rule')}}</strong><small>{{tr('全局规则 · 当前和后续资产','Global · existing and future assets')}}</small></div>
      <button :aria-label="tr('关闭','Close')" @click="emit('close')"><X :size="14" /></button>
    </header>
    <input ref="input" v-model="keyword" maxlength="200" @keyup.enter="keyword.trim().length >= 2 && emit('apply',keyword)" />
    <p>{{tr('确认后只执行一次数据库重分类；匹配标题的资产会进入内容隔离区。','The database is reclassified once; matching titles move to content quarantine.')}}</p>
    <footer>
      <button class="button ghost compact" :disabled="busy" @click="emit('close')">{{tr('取消','Cancel')}}</button>
      <button class="button primary compact" :disabled="busy || keyword.trim().length < 2" @click="emit('apply',keyword)"><Plus :size="14" /> {{busy?tr('应用中…','Applying…'):tr('确认添加','Add rule')}}</button>
    </footer>
  </aside>
</template>
