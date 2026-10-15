const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const { transpile } = require('./shell_module_harness.cjs');

async function messageHarness() {
  const lowlight = await import('lowlight');
  const cache = new Map(), calls = [], timers = new Map(); let timerId = 0;
  function load(relative) {
    const filename = path.resolve(__dirname, '..', relative);
    if (cache.has(filename)) return cache.get(filename);
    const source = fs.readFileSync(filename, 'utf8');
    const resolve = name => {
      if (name === 'vue') return vue;
      if (name === 'markdown-it') return { default: require('markdown-it') };
      if (name === 'lowlight') return lowlight;
      if (name === '@lucide/vue') return new Proxy({}, { get: () => () => vue.h('svg', { 'aria-hidden': true }) });
      if (name === '@tauri-apps/plugin-opener') return { openUrl: async url => { calls.push(url); } };
      if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: (_, en) => en }) };
      if (name.startsWith('.')) {
        const target = path.resolve(path.dirname(filename), name);
        return load(path.relative(path.join(__dirname, '..'), path.extname(target) ? target : `${target}.ts`));
      }
      throw Error(`Unexpected message dependency: ${name}`);
    };
    const exports = {};
    if (filename.endsWith('.vue')) {
      const { descriptor } = parse(source, { filename });
      const script = compileScript(descriptor, { id: relative });
      const template = compileTemplate({ source: descriptor.template.content, filename, id: relative, compilerOptions: { bindingMetadata: script.bindings } });
      if (template.errors.length) throw Error(template.errors.join('\n'));
      new Function('require', 'exports', 'setTimeout', 'clearTimeout', transpile(script.content))(resolve, exports, fn => { timers.set(++timerId, fn); return timerId; }, id => timers.delete(id));
      const rendered = {};
      new Function('require', 'exports', transpile(template.code))(require, rendered);
      exports.default = { ...exports.default, render: rendered.render };
    } else new Function('require', 'exports', transpile(source))(resolve, exports);
    cache.set(filename, exports);
    return exports;
  }
  const markdown = load('src/features/conversations/markdown.ts');
  const component = load('src/features/conversations/ConversationMessage.vue').default;
  async function render(content, role = 'assistant') {
    return renderToString(vue.createSSRApp(component, { message: { role, content } }));
  }
  function mount(t, relative, initialProps) {
    const props = vue.reactive(initialProps);
    const node = type => ({ type, children: [], props: {}, text: '', parent: null });
    const renderer = vue.createRenderer({
      createElement: node, createText: text => ({ ...node('#text'), text }), createComment: () => node('#comment'),
      insert(child, parent, anchor) {
        if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
        child.parent = parent;
        const index = anchor ? parent.children.indexOf(anchor) : -1;
        if (index < 0) parent.children.push(child); else parent.children.splice(index, 0, child);
      },
      remove(child) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); },
      setText(child, text) { child.text = text; }, setElementText(child, text) { child.text = text; child.children = []; },
      parentNode: child => child.parent, nextSibling: child => child.parent?.children[child.parent.children.indexOf(child) + 1] || null,
      patchProp(child, key, _, value) { child.props[key] = value; },
    });
    const target = node('root');
    const mounted = load(relative).default;
    const app = renderer.createApp({ setup: () => () => vue.h(mounted, props) });
    app.mount(target); if (t) t.after(() => app.unmount());
    const all = (root = target) => [root, ...root.children.flatMap(all)];
    return { props, target, app, all, text: () => all().map(item => item.text).join(''), button: () => all().find(item => item.type === 'button') };
  }
  return { markdown, render, mount, calls, timers };
}
module.exports = { messageHarness };
