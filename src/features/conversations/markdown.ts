import MarkdownIt from "markdown-it";
import type Token from "markdown-it/lib/token.mjs";
import { common, createLowlight } from "lowlight";

/** No parser-generated HTML is ever inserted into the DOM. */
export type MarkdownTag = "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "blockquote" | "strong" | "em" | "s" | "a" | "span" | "code" | "table" | "thead" | "tbody" | "tr" | "th" | "td" | "hr" | "br";
export type MarkdownNode =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string; language: string }
  | { kind: "image"; text: string }
  | { kind: "element"; tag: MarkdownTag; children: MarkdownNode[]; href?: string; title?: string; start?: number; align?: "left" | "center" | "right"; plain?: boolean };
export type HighlightNode = { text: string } | { classes: string[]; children: HighlightNode[] };

export const MAX_MARKDOWN_LENGTH = 200_000;
export const MAX_HIGHLIGHT_LENGTH = 20_000;
const markdownOptions = { html: false, linkify: false, typographer: false, breaks: false, maxNesting: 20 };
const markdown = new MarkdownIt(markdownOptions);
const highlighter = createLowlight(common);

/** Deliberately exclude app-relative, file, data, blob and custom protocol links. */
export function safeMarkdownHref(value: string | null): string | undefined {
  if (!value || value !== value.trim() || /[\u0000-\u0020\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u206f\\]/u.test(value)) return undefined;
  if (!/^(https?:\/\/|mailto:)/i.test(value)) return undefined;
  try {
    const url = new URL(value);
    if (!["https:", "http:", "mailto:"].includes(url.protocol) || url.username || url.password) return undefined;
    if (url.protocol === "mailto:" && !url.pathname) return undefined;
    return value;
  } catch {
    return undefined;
  }
}

markdown.validateLink = value => safeMarkdownHref(value) !== undefined;

const openingTags: Record<string, MarkdownTag> = {
  paragraph_open: "p", bullet_list_open: "ul", ordered_list_open: "ol", list_item_open: "li",
  blockquote_open: "blockquote", strong_open: "strong", em_open: "em", s_open: "s", link_open: "a",
  table_open: "table", thead_open: "thead", tbody_open: "tbody", tr_open: "tr", th_open: "th", td_open: "td",
};

/** Preserve original code line endings and a missing final newline for copying.
 * Markdown's semantic content removes enclosing list/quote indentation. */
type SourceLine = { text: string; ending: string };
function originalCode(token: Token, sourceLines: SourceLine[]): string {
  if (!token.map || !token.content) return token.content;
  const start = token.map[0] + (token.type === "fence" ? 1 : 0);
  const lines = token.content.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return lines.map((line, index) => {
    const original = sourceLines[start + index];
    // Markdown replaces NUL with U+FFFD. Restore the exact source suffix when
    // possible, after the parser has removed quote/list indentation.
    const text = line && original?.text.replace(/\0/g, "\uFFFD").endsWith(line)
      ? original.text.slice(-line.length) : line;
    return text + (original?.ending ?? "");
  }).join("");
}

function languageLabel(info: string): string {
  const language = info.trim().split(/\s+/, 1)[0].toLowerCase();
  return /^[a-z0-9][a-z0-9_+.#-]{0,39}$/.test(language) ? language : "";
}

function tokenTree(tokens: Token[], sourceLines: SourceLine[]): MarkdownNode[] {
  const root: MarkdownNode[] = [];
  const stack = [root];
  for (const token of tokens) {
    if (token.hidden) continue;
    const current = stack[stack.length - 1];
    if (token.nesting === -1) {
      if (stack.length > 1) stack.pop();
    } else if (token.type === "inline") {
      current.push(...tokenTree(token.children || [], sourceLines));
    } else if (token.type === "fence" || token.type === "code_block") {
      current.push({ kind: "code", text: originalCode(token, sourceLines), language: token.type === "fence" ? languageLabel(token.info) : "" });
    } else if (token.type === "image") {
      // Never create img, iframe, video or any element that fetches a resource.
      current.push({ kind: "image", text: token.content });
    } else if (token.type === "code_inline") {
      current.push({ kind: "element", tag: "code", children: [{ kind: "text", text: token.content }] });
    } else if (token.type === "softbreak") {
      current.push({ kind: "text", text: "\n" });
    } else if (token.type === "hardbreak" || token.type === "hr") {
      current.push({ kind: "element", tag: token.type === "hr" ? "hr" : "br", children: [] });
    } else if (token.nesting === 1) {
      const tag = token.type === "heading_open" && /^h[1-6]$/.test(token.tag) ? token.tag as MarkdownTag : openingTags[token.type] || "span";
      const node: Extract<MarkdownNode, { kind: "element" }> = { kind: "element", tag, children: [] };
      if (tag === "a") {
        node.href = safeMarkdownHref(token.attrGet("href"));
        if (!node.href) node.tag = "span";
        const title = token.attrGet("title");
        if (title) node.title = title;
      }
      if (tag === "ol") {
        const start = Number(token.attrGet("start"));
        if (Number.isSafeInteger(start) && start > 0) node.start = start;
      }
      if (tag === "th" || tag === "td") {
        const align = token.attrGet("style")?.match(/^text-align:(left|center|right)$/)?.[1];
        if (align) node.align = align as "left" | "center" | "right";
      }
      current.push(node);
      stack.push(node.children);
    } else if (token.content) {
      // Includes any future/unsupported raw HTML token: always visible text.
      current.push({ kind: "text", text: token.content });
    }
  }
  return root;
}

export function parseMessageMarkdown(source: string): MarkdownNode[] {
  if (!source) return [];
  const fallback: MarkdownNode[] = [{ kind: "element", tag: "p", plain: true, children: [{ kind: "text", text: source }] }];
  // Imported histories can be larger than the composer limit. Remain readable
  // without running parsers/highlighters on arbitrarily large responses.
  if (source.length > MAX_MARKDOWN_LENGTH) return fallback;
  try {
    const sourceLines = (source.match(/[^\r\n]*(?:\r\n|\r|\n|$)/g) || []).map(line => ({ text: line.replace(/(?:\r\n|\r|\n)$/, ""), ending: line.match(/(\r\n|\r|\n)$/)?.[0] || "" }));
    return tokenTree(markdown.parse(source, {}), sourceLines);
  } catch {
    return fallback;
  }
}

export function highlightCode(source: string, language: string): HighlightNode[] {
  const fallback = [{ text: source }];
  if (!language || source.length > MAX_HIGHLIGHT_LENGTH || !highlighter.registered(language)) return fallback;
  try {
    const tree = highlighter.highlight(language, source);
    type TreeNode = (typeof tree.children)[number];
    const clean = (node: TreeNode): HighlightNode[] => {
      if (node.type === "text") return [{ text: node.value }];
      if (node.type !== "element") return [];
      // Retain only text and syntax-class spans, never arbitrary HAST attributes.
      const children = node.children.flatMap(child => clean(child as TreeNode));
      if (node.tagName !== "span") return children;
      const classes = Array.isArray(node.properties.className)
        ? node.properties.className.filter((name): name is string => typeof name === "string" && /^[a-zA-Z_][a-zA-Z0-9_-]*$/.test(name)) : [];
      return [{ classes, children }];
    };
    return tree.children.flatMap(clean);
  } catch {
    return fallback;
  }
}
