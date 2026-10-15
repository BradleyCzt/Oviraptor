<script setup lang="ts">
import { computed, defineComponent, h, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "../../i18n";
import { parseMessageMarkdown, safeMarkdownHref, type MarkdownNode } from "./markdown";
import ConversationCodeBlock from "./ConversationCodeBlock.vue";
import MessageCopyButton from "./MessageCopyButton.vue";

const props = defineProps<{ message: { role: "user" | "assistant"; content: string } }>();
const { tr } = useI18n();
// Polling replaces snapshot objects even when their text has not changed.
// Stable primitive dependencies avoid reparsing the entire saved history.
const content = computed(() => props.message.content);
const role = computed(() => props.message.role);
const markdown = computed(() => role.value === "assistant" ? parseMessageMarkdown(content.value) : []);
const linkError = ref("");
watch(content, () => { linkError.value = ""; });

function followLink(event: MouseEvent, href: string) {
  // Tauri links leave the webview through the system browser. The preview uses
  // normal isolated browser tabs. Never navigate the app to generated content.
  if (!("__TAURI_INTERNALS__" in globalThis)) return;
  event.preventDefault();
  const safe = safeMarkdownHref(href);
  linkError.value = "";
  if (safe) void openUrl(safe).catch(() => { linkError.value = tr("无法打开链接，请复制链接地址后重试", "Could not open the link. Copy its address and try in your browser."); });
}

function renderNode(node: MarkdownNode, key: number): ReturnType<typeof h> | string {
  if (node.kind === "text") return node.text;
  if (node.kind === "code") return h(ConversationCodeBlock, { key, source: node.text, language: node.language });
  if (node.kind === "image") return h("span", { key, class: "markdown-image-label", title: tr("图片未加载，避免自动访问外部资源", "Image not loaded to avoid automatic external requests") }, `[${tr("图片", "Image")}${node.text ? `: ${node.text}` : ""}]`);
  const attributes: Record<string, unknown> = { key };
  if (node.tag === "a" && node.href) {
    attributes.href = node.href;
    attributes.target = "_blank";
    attributes.rel = "noopener noreferrer";
    attributes.referrerpolicy = "no-referrer";
    attributes.onClick = (event: MouseEvent) => followLink(event, node.href!);
  }
  if (node.title) attributes.title = node.title;
  if (node.start) attributes.start = node.start;
  if (node.align) attributes.style = { textAlign: node.align };
  if (node.plain) attributes.class = "markdown-plain";
  const element = h(node.tag, attributes, node.children.map(renderNode));
  return node.tag === "table" ? h("div", { key, class: "markdown-table-scroll", tabindex: 0, role: "region", "aria-label": tr("表格，可横向滚动", "Table, scroll horizontally") }, [element]) : element;
}
const MarkdownContent = defineComponent({ setup: () => () => h("div", { class: "message-markdown" }, markdown.value.map(renderNode)) });
</script>

<template>
  <article class="conversation-message" :class="role" :aria-label="role === 'user' ? tr('你的消息', 'Your message') : tr('助手消息', 'Assistant message')">
    <div v-if="role === 'assistant'" class="message-author"><span class="message-agent-mark" aria-hidden="true">o</span><span>Oviraptor</span></div>
    <p v-if="role === 'user'" class="message-user-text">{{ content }}</p>
    <MarkdownContent v-else-if="content" />
    <p v-else class="message-empty">{{ tr('空消息', 'Empty message') }}</p>
    <p v-if="linkError" class="message-link-error" role="status">{{ linkError }}</p>
    <div class="message-actions"><MessageCopyButton :source="content" :label="tr('复制消息', 'Copy message')" compact /></div>
  </article>
</template>

<style scoped>
.conversation-message{min-width:0;max-width:100%;margin:0 0 32px;color:var(--app-ink,#24272d);font-size:14px;line-height:1.8}
.conversation-message.user{max-width:min(82%,660px);margin-left:auto;margin-bottom:30px;padding:12px 17px 6px;border:1px solid color-mix(in srgb,var(--line,#e1e3e7) 65%,transparent);border-radius:18px 18px 5px 18px;background:var(--chat-user-bg,color-mix(in srgb,var(--app-ink,#24272d) 5%,var(--app-surface,#fff)))}
.message-author{display:flex;align-items:center;gap:9px;margin:0 0 14px;font-size:12px;font-weight:650;line-height:24px;color:var(--app-ink,#24272d)}
.message-agent-mark{display:inline-flex;align-items:center;justify-content:center;width:24px;height:24px;flex-shrink:0;border-radius:8px;background:var(--app-ink,#24272d);color:var(--app-surface,#fff);font-size:20px;font-weight:650;line-height:1;padding-bottom:3px;font-family:ui-rounded,system-ui,sans-serif}
.message-user-text{margin:0;white-space:pre-wrap;overflow-wrap:anywhere}
.message-actions{display:flex;gap:4px;margin-top:8px}.user .message-actions{justify-content:flex-end;margin-top:2px;margin-right:-6px}
.message-empty{margin:0;color:var(--app-muted,#687078);font-style:italic}
.message-link-error{margin:8px 0;color:var(--red,#b63f3f);font-size:12px}
:deep(.message-markdown){min-width:0;overflow-wrap:anywhere}
:deep(.message-markdown>:first-child){margin-top:0}:deep(.message-markdown>:last-child){margin-bottom:0}
:deep(.message-markdown p){margin:0 0 14px;white-space:normal;line-height:1.85}
:deep(.message-markdown h1),:deep(.message-markdown h2),:deep(.message-markdown h3),:deep(.message-markdown h4),:deep(.message-markdown h5),:deep(.message-markdown h6){font-weight:650;line-height:1.4;letter-spacing:-.025em;margin:25px 0 12px;overflow-wrap:anywhere}
:deep(.message-markdown h1){font-size:24px}:deep(.message-markdown h2){font-size:20px}:deep(.message-markdown h3){font-size:17px}:deep(.message-markdown h4),:deep(.message-markdown h5),:deep(.message-markdown h6){font-size:14px}
:deep(.message-markdown ul),:deep(.message-markdown ol){padding-left:24px;margin:12px 0 16px}:deep(.message-markdown li){padding-left:3px;margin:5px 0}:deep(.message-markdown li p){margin-bottom:7px}:deep(.message-markdown li ul),:deep(.message-markdown li ol){margin:5px 0}
:deep(.message-markdown a){color:var(--chat-link,var(--blue,#416b9c));text-decoration:underline;text-decoration-color:color-mix(in srgb,currentColor 40%,transparent);text-underline-offset:3px;overflow-wrap:anywhere}
:deep(.message-markdown a:hover){text-decoration-color:currentColor}:deep(.message-markdown a:focus-visible){outline:2px solid var(--blue,#5865a5);outline-offset:3px;border-radius:2px}
:deep(.message-markdown :not(pre)>code){padding:2px 5px;border:1px solid color-mix(in srgb,var(--line,#e1e3e7) 75%,transparent);border-radius:5px;background:color-mix(in srgb,var(--app-ink,#24272d) 4%,transparent);font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace;font-size:.88em;white-space:break-spaces;overflow-wrap:anywhere}
:deep(.message-markdown blockquote){margin:17px 0;padding:3px 0 3px 17px;border-left:3px solid var(--line,#d9dde2);color:var(--app-muted,#687078)}:deep(.message-markdown blockquote>:last-child){margin-bottom:0}
:deep(.message-markdown hr){border:0;border-top:1px solid var(--line,#e1e3e7);margin:25px 0}
:deep(.markdown-table-scroll){max-width:100%;overflow-x:auto;margin:18px 0;border:1px solid var(--line,#e1e3e7);border-radius:10px;scrollbar-width:thin}:deep(.markdown-table-scroll:focus-visible){outline:2px solid var(--blue,#5865a5);outline-offset:2px}
:deep(.message-markdown table){border-collapse:collapse;min-width:100%;font-size:12px;line-height:1.65}:deep(.message-markdown th),:deep(.message-markdown td){text-align:left;padding:10px 13px;border-bottom:1px solid var(--line,#e1e3e7);border-right:1px solid var(--line,#e1e3e7);min-width:90px}:deep(.message-markdown th){background:color-mix(in srgb,var(--app-ink,#24272d) 4%,transparent);font-weight:600}:deep(.message-markdown tr:last-child td){border-bottom:0}:deep(.message-markdown th:last-child),:deep(.message-markdown td:last-child){border-right:0}
:deep(.markdown-image-label){color:var(--app-muted,#687078);font-style:italic}:deep(.message-markdown .markdown-plain){white-space:pre-wrap}
@media(max-width:600px){.conversation-message{font-size:13px;margin-bottom:25px}.conversation-message.user{max-width:92%;padding:10px 13px 4px}:deep(.message-markdown h1){font-size:22px}:deep(.message-markdown h2){font-size:18px}}
</style>
