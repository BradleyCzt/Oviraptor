<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { Check, Copy, AlertCircle } from "@lucide/vue";
import { useI18n } from "../../i18n";

const props = defineProps<{ source: string; label: string; compact?: boolean }>();
const { tr } = useI18n();
const state = ref<"idle" | "copying" | "copied" | "failed">("idle");
let attempt = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
const status = computed(() => state.value === "copied" ? tr("已复制", "Copied") : state.value === "failed" ? tr("复制失败，请选择文本后复制", "Copy failed; select the text to copy") : "");
const label = computed(() => status.value || (state.value === "copying" ? tr("正在复制…", "Copying…") : props.label));
function reset() {
  attempt++;
  clearTimeout(timer);
  state.value = "idle";
}
watch(() => props.source, reset);
onBeforeUnmount(reset);
async function copy() {
  if (state.value === "copying") return;
  const current = ++attempt;
  clearTimeout(timer);
  state.value = "copying";
  try {
    if (!globalThis.navigator?.clipboard?.writeText) throw new Error("clipboard_unavailable");
    // Never copy rendered markup, labels, or syntax-highlighter output.
    await navigator.clipboard.writeText(props.source);
    if (current !== attempt) return;
    state.value = "copied";
    timer = setTimeout(() => { state.value = "idle"; }, 2200);
  } catch {
    if (current === attempt) state.value = "failed";
  }
}
</script>

<template>
  <span class="message-copy-control">
    <button type="button" class="message-copy-button" :class="{ compact, failed: state === 'failed' }" :title="label" :aria-label="label" :disabled="state === 'copying'" @click="copy">
      <Check v-if="state === 'copied'" :size="14" aria-hidden="true" />
      <AlertCircle v-else-if="state === 'failed'" :size="14" aria-hidden="true" />
      <Copy v-else :size="14" aria-hidden="true" />
      <span v-if="!compact">{{ state === 'failed' ? tr('复制失败', 'Copy failed') : label }}</span>
    </button>
    <span class="copy-status" role="status" aria-live="polite">{{ status }}</span>
  </span>
</template>

<style scoped>
.message-copy-control{display:inline-flex;position:relative}
.message-copy-button{display:inline-flex;align-items:center;justify-content:center;gap:6px;min-height:30px;padding:5px 8px;border:1px solid transparent;border-radius:7px;background:transparent;color:var(--app-muted,#687078);font:inherit;font-size:11px;line-height:1.4;cursor:pointer;transition:background .15s,color .15s}
.message-copy-button.compact{width:32px;height:32px;padding:7px}
.message-copy-button:hover{color:var(--app-ink,#24272d);background:color-mix(in srgb,var(--app-ink,#24272d) 7%,transparent)}
.message-copy-button:focus-visible{outline:2px solid var(--blue,#5865a5);outline-offset:2px}
.message-copy-button:disabled{opacity:.6;cursor:wait}
.message-copy-button.failed{color:var(--red,#b63f3f)}
.copy-status{position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap}
</style>
