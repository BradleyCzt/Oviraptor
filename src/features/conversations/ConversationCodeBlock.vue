<script setup lang="ts">
import { computed, defineComponent, h } from "vue";
import { useI18n } from "../../i18n";
import { highlightCode, type HighlightNode } from "./markdown";
import MessageCopyButton from "./MessageCopyButton.vue";

const props = defineProps<{ source: string; language: string }>();
const { tr } = useI18n();
const highlighted = computed(() => highlightCode(props.source, props.language));
const renderHighlight = (node: HighlightNode): ReturnType<typeof h> | string => "text" in node ? node.text : h("span", { class: node.classes }, node.children.map(renderHighlight));
const HighlightedCode = defineComponent({ setup: () => () => h("code", highlighted.value.map(renderHighlight)) });
</script>

<template>
  <div class="message-code-block">
    <div class="message-code-header"><span>{{ language || tr('纯文本', 'Plain text') }}</span><MessageCopyButton :source="source" :label="tr('复制代码', 'Copy code')" /></div>
    <pre tabindex="0" :aria-label="tr('代码块，可横向滚动', 'Code block, scroll horizontally')"><HighlightedCode /></pre>
  </div>
</template>

<style scoped>
.message-code-block{min-width:0;max-width:100%;margin:18px 0;border:1px solid var(--line,#e1e3e7);border-radius:12px;overflow:hidden;background:var(--chat-code-bg,color-mix(in srgb,var(--app-ink,#24272d) 3%,var(--app-surface,#fff)))}
.message-code-header{display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:38px;padding:4px 9px 4px 16px;border-bottom:1px solid var(--line,#e1e3e7);color:var(--app-muted,#687078);font-size:11px;font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace}
pre{margin:0;max-width:100%;overflow:auto;padding:16px 18px;color:var(--app-ink,#24272d);font-size:12px;line-height:1.75;tab-size:2;white-space:pre;overflow-wrap:normal;word-break:normal;scrollbar-width:thin}
pre:focus-visible{outline:2px solid var(--blue,#5865a5);outline-offset:-2px}
pre :deep(code){font:inherit;white-space:inherit;background:none;padding:0;border-radius:0;color:inherit}
pre :deep(.hljs-comment),pre :deep(.hljs-quote){color:var(--chat-syntax-comment,#737a83);font-style:italic}
pre :deep(.hljs-keyword),pre :deep(.hljs-selector-tag),pre :deep(.hljs-literal),pre :deep(.hljs-built_in){color:var(--chat-syntax-keyword,#8057a3)}
pre :deep(.hljs-string),pre :deep(.hljs-regexp),pre :deep(.hljs-addition),pre :deep(.hljs-attribute){color:var(--chat-syntax-string,#398260)}
pre :deep(.hljs-number),pre :deep(.hljs-symbol),pre :deep(.hljs-bullet),pre :deep(.hljs-variable),pre :deep(.hljs-template-variable){color:var(--chat-syntax-number,#b16b35)}
pre :deep(.hljs-title),pre :deep(.hljs-section),pre :deep(.hljs-selector-id),pre :deep(.hljs-selector-class),pre :deep(.hljs-name),pre :deep(.hljs-attr){color:var(--chat-syntax-title,#356f9c)}
pre :deep(.hljs-deletion){color:var(--red,#b63f3f)}
pre :deep(.hljs-emphasis){font-style:italic}pre :deep(.hljs-strong){font-weight:700}
:global(.app-shell.theme-codex .message-code-block),:global(.app-shell.theme-graphite .message-code-block),:global(.app-shell.theme-midnight .message-code-block){--chat-syntax-comment:#9099a6;--chat-syntax-keyword:#c4a4e4;--chat-syntax-string:#9acaad;--chat-syntax-number:#dfb086;--chat-syntax-title:#93c0e3}
@media(max-width:600px){pre{padding:13px;font-size:11px}.message-code-header{padding-left:13px}.message-code-block{border-radius:10px}}
</style>
