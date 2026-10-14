<script setup lang="ts">
import { AlertTriangle } from "@lucide/vue";

withDefaults(defineProps<{
  title: string;
  detail?: string;
  confirmText?: string;
  busyText?: string;
  tone?: "danger" | "warning" | "secondary";
  busy?: boolean;
}>(), { detail: "", confirmText: "确认删除", busyText: "删除中…", tone: "danger", busy: false });

defineEmits<{ confirm: []; cancel: [] }>();
</script>

<template>
  <div class="inline-confirm" role="alert">
    <AlertTriangle :size="18" />
    <div><strong>{{title}}</strong><p>{{detail}}</p></div>
    <button class="button ghost compact" :disabled="busy" @click="$emit('cancel')">取消</button>
    <button :class="`button ${tone} compact`" :disabled="busy" @click="$emit('confirm')">{{busy?busyText:confirmText}}</button>
  </div>
</template>
