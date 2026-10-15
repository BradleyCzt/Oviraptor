const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { messageHarness } = require('./conversation_message_harness.cjs');
let h;
test.before(async () => { h = await messageHarness(); });
const walk = nodes => nodes.flatMap(node => [node, ...('children' in node ? walk(node.children) : [])]);
const code = source => walk(h.markdown.parseMessageMarkdown(source)).find(node => node.kind === 'code');
const highlightText = nodes => nodes.map(node => 'text' in node ? node.text : highlightText(node.children)).join('');
const flush = async () => { for (let index = 0; index < 5; index++) await vue.nextTick(); };
function clipboard(t, writeText) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: writeText ? { clipboard: { writeText } } : {} });
  t.after(() => previous ? Object.defineProperty(globalThis, 'navigator', previous) : delete globalThis.navigator);
}

test('assistant Markdown renders headings, emphasis, lists, quotes, tables, inline and fenced code', async () => {
  const html = await h.render('# Summary\n\n**Bold** and *emphasis* and ~~old~~ with `inline()`\n\n- First\n- Second\n\n3. Third\n\n> A quote\n\n| Name | Count |\n| :--- | ---: |\n| A | 2 |\n\n```js\nconst answer = 42;\n```');
  for (const pattern of [/<h1>Summary<\/h1>/, /<strong>Bold<\/strong>/, /<em>emphasis<\/em>/, /<s>old<\/s>/, /<ul>/, /<ol start="3">/, /<blockquote>/, /<table>/, /text-align:right/, /<code>inline\(\)<\/code>/, /hljs-keyword/, /Copy code/, /Copy message/]) assert.match(html, pattern);
  assert.match(html, /role="region" aria-label="Table, scroll horizontally"/);
  assert.match(html, /<pre tabindex="0"/);
});

test('user messages are escaped plain text and never interpreted as Markdown', async () => {
  const source = '# title\n<script>alert(1)</script>\n```js\n1\n```';
  const html = await h.render(source, 'user');
  assert.match(html, /# title/); assert.match(html, /&lt;script&gt;/); assert.match(html, /```js/);
  assert.doesNotMatch(html, /<h1>|<script|message-code-block|hljs-/);
});

test('raw HTML, SVG, events, iframes and images remain inert escaped text', async () => {
  const html = await h.render('<script>alert(1)</script>\n<img src="https://tracker.invalid/pixel" onerror="alert(1)">\n<svg onload="alert(1)"><a href="javascript:alert(1)">bad</a></svg>\n<iframe srcdoc="<script>oops</script>"></iframe>');
  assert.doesNotMatch(html, /<(?:script|img|svg\s+onload|iframe|object|embed|link|style)\b/i);
  assert.match(html, /&lt;script&gt;alert/);
  assert.match(html, /&lt;img/);
});

test('Markdown images never create resource requests, including tracking and data images', async () => {
  const html = await h.render('![private picture](https://tracker.invalid/pixel?private=true)\n\n![x](data:image/svg+xml;base64,PHN2Zz4=)');
  assert.match(html, /\[Image: private picture\]/);
  assert.doesNotMatch(html, /<(?:img|video|audio|iframe|source)\b|\ssrc(?:set)?=/i);
  assert.doesNotMatch(html, /href="https:\/\/tracker/);
});

test('safe links use isolated external destinations with no referrer', async () => {
  const html = await h.render('[Docs](https://example.org/docs?q=hello "Documentation") and [Email](mailto:team@example.org)');
  assert.match(html, /href="https:\/\/example.org\/docs\?q=hello" target="_blank" rel="noopener noreferrer" referrerpolicy="no-referrer" title="Documentation"/);
  assert.match(html, /href="mailto:team@example.org"/);
});

test('unsafe and disguised protocols, credentials and local app paths cannot become links', async () => {
  for (const url of ['javascript:alert(1)', 'JaVaScRiPt:alert(1)', 'vbscript:msgbox(1)', 'data:text/html,test', 'file:///etc/passwd', 'blob:https://example.org/id', 'tauri://localhost', '//example.org/a', '/settings', '#fragment', 'https://user:password@example.org', 'https:\\example.org', ' https://example.org', 'https://ex\u0000ample.org', 'https://example.org\u202e/test']) assert.equal(h.markdown.safeMarkdownHref(url), undefined, url);
  for (const source of ['[x](javascript:alert%281%29)', '[x](jav&#x61;script:alert%281%29)', '[x](data:text/html;base64,PHNjcmlwdD4=)', '[x](file:///etc/passwd)', '[x](//example.org)', '[x](/settings)', '[x](https://user:pass@example.org)']) assert.doesNotMatch(await h.render(source), /<a\s/, source);
  assert.equal(h.markdown.safeMarkdownHref('https://example.org'), 'https://example.org');
});

test('Tauri link clicks use the system opener instead of navigating the app', async t => {
  globalThis.__TAURI_INTERNALS__ = {}; t.after(() => { delete globalThis.__TAURI_INTERNALS__; });
  const mounted = h.mount(t, 'src/features/conversations/ConversationMessage.vue', { message: { role: 'assistant', content: '[Docs](https://example.org)' } });
  const link = mounted.all().find(node => node.type === 'a'); let prevented = false;
  link.props.onClick({ preventDefault() { prevented = true; } }); await flush();
  assert.equal(prevented, true); assert.deepEqual(h.calls, ['https://example.org']);
});

test('code copy source preserves Unicode, trailing spaces, blank lines and original line endings', () => {
  for (const [source, expected] of [
    ['```js\nconst x = "你好 🦖";  \n\n```', 'const x = "你好 🦖";  \n\n'],
    ['```python\r\nprint("你好")\r\n\r\n```', 'print("你好")\r\n\r\n'],
    ['```text\ra\rb\r```', 'a\rb\r'],
    ['~~~text\nraw\n~~~', 'raw\n'],
    ['```js\nnoFinalNewline', 'noFinalNewline'],
    ['> ```js\n> const x = 1;\n> ```', 'const x = 1;\n'],
    ['- ```js\n  const x = 1;\n  ```', 'const x = 1;\n'],
    ['    a\r\n    b', 'a\r\nb'],
    ['```text\nnull\u0000byte\n```', 'null\u0000byte\n'],
  ]) assert.equal(code(source).text, expected, JSON.stringify(source));
});

test('unknown or hostile language metadata and unfinished fences stay readable', async () => {
  const source = '```unknown-language\n<script>"雪"</script>';
  assert.equal(code(source).language, 'unknown-language');
  assert.equal(code(source).text, '<script>"雪"</script>');
  const html = await h.render(source);
  assert.match(html, /unknown-language/); assert.match(html, /&lt;script&gt;/); assert.doesNotMatch(html, /hljs-|<script>/);
  assert.equal(code('```"><img/onerror=x>\nhello\n```').language, '');
  assert.equal(code('```js onclick=alert(1)\na\n```').language, 'js');
  assert.doesNotMatch(await h.render('```"><img/onerror=x>\nhello\n```'), /<img\b|onclick=/);
});

test('syntax highlighting returns only safe spans and exact code text', () => {
  const source = 'const html = "<img src=x onerror=alert(1)>"; // 🦖\r\n';
  const tree = h.markdown.highlightCode(source, 'javascript');
  assert.equal(highlightText(tree), source);
  assert.ok(tree.some(node => 'classes' in node));
  for (const node of walk(tree)) if ('classes' in node) for (const name of node.classes) assert.match(name, /^[a-zA-Z_][a-zA-Z0-9_-]*$/);
  assert.deepEqual(h.markdown.highlightCode(source, 'not-a-language'), [{ text: source }]);
});

test('large code and imported histories remain complete without expensive highlighting', async () => {
  const codeSource = '"'.repeat(65_536);
  assert.deepEqual(h.markdown.highlightCode(codeSource, 'javascript'), [{ text: codeSource }]);
  const source = '# title\n'.repeat(30_000);
  const parsed = h.markdown.parseMessageMarkdown(source);
  assert.equal(parsed[0].plain, true); assert.equal(parsed[0].children[0].text, source);
  const html = await h.render('```js\n' + codeSource + '\n```');
  assert.doesNotMatch(html, /hljs-/); assert.match(html, /Copy code/);
});

test('empty and incomplete messages have stable readable output', async () => {
  assert.deepEqual(h.markdown.parseMessageMarkdown(''), []);
  assert.match(await h.render(''), /Empty message/);
  assert.match(await h.render('```js\n'), /Copy code/);
  for (const source of ['**partial', '[unfinished](https://', '| name |\n| ---', '```', '- a\n  - b']) assert.ok(await h.render(source));
});

test('polling cloned messages does not reparse unchanged Markdown history', async t => {
  const original = h.markdown.parseMessageMarkdown; let parses = 0;
  h.markdown.parseMessageMarkdown = source => { parses++; return original(source); };
  t.after(() => { h.markdown.parseMessageMarkdown = original; });
  const message = { role: 'assistant', content: '# Saved response\n\n```js\nconst x = 1;\n```' };
  const mounted = h.mount(t, 'src/features/conversations/ConversationMessage.vue', { message });
  assert.equal(parses, 1);
  for (let index = 0; index < 4; index++) { mounted.props.message = { ...message }; await flush(); }
  assert.equal(parses, 1);
  mounted.props.message = { ...message, content: '# New response' }; await flush();
  assert.equal(parses, 2); assert.match(mounted.text(), /New response/);
});

test('message copy writes exact Markdown source once and reports success accessibly', async t => {
  const writes = []; let finish;
  clipboard(t, text => { writes.push(text); return new Promise(resolve => { finish = resolve; }); });
  const source = '# Title\r\n```js\r\nconst x = "🦖";\r\n```\r\n';
  const mounted = h.mount(t, 'src/features/conversations/MessageCopyButton.vue', { source, label: 'Copy message', compact: true });
  const click = mounted.button().props.onClick; const first = click(); await click(); await flush();
  assert.deepEqual(writes, [source]); assert.equal(mounted.button().props.disabled, true);
  finish(); await first; await flush();
  assert.equal(mounted.button().props['aria-label'], 'Copied');
  assert.equal(mounted.all().find(node => node.props.role === 'status').text, 'Copied');
});

test('rendered code and message copy buttons use their distinct original sources', async t => {
  const writes = []; clipboard(t, async text => { writes.push(text); });
  const source = '# Example\r\n\r\n```python\r\nprint("你好 🦖")  \r\n```';
  const mounted = h.mount(t, 'src/features/conversations/ConversationMessage.vue', { message: { role: 'assistant', content: source } });
  const buttons = mounted.all().filter(node => node.type === 'button');
  assert.equal(buttons.length, 2);
  await buttons[0].props.onClick(); await buttons[1].props.onClick(); await flush();
  assert.deepEqual(writes, ['print("你好 🦖")  \r\n', source]);
});

test('copy failure is visible to assistive technology and can be retried', async t => {
  let attempts = 0;
  clipboard(t, async () => { if (++attempts === 1) throw Error('denied'); });
  const mounted = h.mount(t, 'src/features/conversations/MessageCopyButton.vue', { source: 'copy me', label: 'Copy code' });
  await mounted.button().props.onClick(); await flush();
  assert.match(mounted.button().props['aria-label'], /Copy failed/); assert.match(mounted.text(), /Copy failed/);
  await mounted.button().props.onClick(); await flush();
  assert.equal(mounted.button().props['aria-label'], 'Copied'); assert.equal(attempts, 2);
});

test('unavailable clipboard never falsely reports success', async t => {
  clipboard(t);
  const mounted = h.mount(t, 'src/features/conversations/MessageCopyButton.vue', { source: '', label: 'Copy message' });
  await mounted.button().props.onClick(); await flush();
  assert.match(mounted.button().props['aria-label'], /Copy failed/);
});

test('changing message source fences a late copy completion and clears old status', async t => {
  let finish; clipboard(t, () => new Promise(resolve => { finish = resolve; }));
  const mounted = h.mount(t, 'src/features/conversations/MessageCopyButton.vue', { source: 'first', label: 'Copy message' });
  const pending = mounted.button().props.onClick(); mounted.props.source = 'second'; await flush(); finish(); await pending; await flush();
  assert.equal(mounted.button().props['aria-label'], 'Copy message');
  assert.equal(mounted.all().find(node => node.props.role === 'status').text, '');
});

test('renderer sources contain no dynamic HTML insertion or resource-loading elements', () => {
  for (const name of ['ConversationMessage.vue', 'ConversationCodeBlock.vue', 'MessageCopyButton.vue', 'markdown.ts']) {
    const source = fs.readFileSync(path.join(__dirname, '../src/features/conversations', name), 'utf8');
    assert.doesNotMatch(source, /v-html\s*=|innerHTML\s*[:=]|insertAdjacentHTML\s*\(|document\.write\s*\(/, name);
    assert.doesNotMatch(source, /<(?:img|iframe|object|embed|video|audio)\b/i, name);
  }
});
