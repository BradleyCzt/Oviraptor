// Real setup + compiled templates in Vue's host renderer. Only local IPC, icons
// and native DOM directives are replaced; controls dispatch their compiled events.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const transpile = source => ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const flush = async () => { for (let i = 0; i < 20; i++) await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const asset = (id = 1, projectId = 9001) => ({ id, projectId, title: `Example ${id}`, host: `example-${id}`, extra: {}, decision: 'pending' });
const rule = (projectId = 9001, enabled = true) => ({ id: projectId, projectId, kind: 'title_contains', value: 'example', enabled, exampleTitle: 'Example' });
const preview = (input = { projectId: 9001, sourceAssetId: 1, kind: 'title_contains', value: 'example' }) => ({ rule: input, matched: 2, eligible: 1, protected: 1, alreadyIsolated: 0,
  sample: [{ assetId: 1, title: 'Example', link: 'https://example.test', eligible: true, alreadyIsolated: false }], previewToken: `synthetic-preview-${input.projectId}` });

async function mountReview(name = 'PageIsolationDialog', overrides = {}, initialProps = {}) {
  const calls = [], events = [], records = [], cache = new Map();
  const transport = {
    listPageRules: async projectId => [rule(projectId)],
    getAssetRuleCatalog: async () => ({ projectRules: [rule()], legacyRules: [] }),
    previewPageRule: async input => preview(input),
    applyPageRule: async () => ({ ruleId: 1, isolated: 1, protected: 1, backupPath: '/synthetic/backup' }),
    disablePageRule: async () => ({ restored: 1, retained: 1, backupPath: '/synthetic/backup' }),
    ...overrides,
  };
  const api = new Proxy({}, { get(_, method) {
    return (...args) => {
      calls.push([method, ...args]);
      assert.equal(typeof transport[method], 'function', `Unexpected review IPC: ${String(method)}`);
      return transport[method](...args);
    };
  } });
  const renderVue = { ...vue, vModelText: {}, vModelSelect: {}, vModelCheckbox: {} };
  const modal = { props: ['title', 'wide'], setup(props, { slots }) {
    return () => vue.h('section', { class: 'review-modal' }, [vue.h('h2', props.title), slots.default?.(), slots.footer?.()]);
  } };
  function load(componentName) {
    if (cache.has(componentName)) return cache.get(componentName);
    const filename = path.join(__dirname, `../src/features/assets/components/${componentName}.vue`);
    const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
    const script = compileScript(descriptor, { id: componentName });
    const template = compileTemplate({ source: descriptor.template.content, filename, id: componentName,
      compilerOptions: { bindingMetadata: script.bindings } });
    assert.deepEqual(template.errors, []);
    const exports = {};
    const local = imported => {
      if (imported === 'vue') return vue;
      if (imported === '../api') return { assetApi: api };
      if (imported === '../../../i18n') return { useI18n: () => ({ tr: (_zh, en) => en }) };
      if (imported === '@lucide/vue') return new Proxy({}, { get: () => () => null });
      if (imported === '../../../shared/components/ModalShell.vue') return { default: modal };
      if (imported === './PageIsolationDialog.vue') return { default: load('PageIsolationDialog') };
      throw new Error(`Unexpected review component import: ${imported}`);
    };
    new Function('require', 'exports', transpile(script.content))(local, exports);
    const rendered = {};
    new Function('require', 'exports', transpile(template.code))(module => module === 'vue' ? renderVue : local(module), rendered);
    const component = { ...exports.default, setup(props, context) {
      const bindings = exports.default.setup(props, context);
      const record = { name: componentName, bindings, active: true };
      records.push(record); vue.onUnmounted(() => { record.active = false; });
      return () => rendered.render({}, [], props, vue.proxyRefs(bindings), {}, {});
    } };
    cache.set(componentName, component); return component;
  }
  const makeNode = (type, text = '') => ({ type, text, props: {}, children: [], parent: null });
  const renderer = vue.createRenderer({
    createElement: type => makeNode(type), createText: text => makeNode('#text', text), createComment: () => makeNode('#comment'),
    insert(node, parent, anchor = null) {
      if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
      node.parent = parent;
      const index = anchor ? parent.children.indexOf(anchor) : -1;
      if (index < 0) parent.children.push(node); else parent.children.splice(index, 0, node);
    },
    remove(node) { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); node.parent = null; },
    setText(node, text) { node.text = text; },
    setElementText(node, text) { node.text = text; node.children = []; },
    patchProp(node, key, _old, value) { node.props[key] = value; },
    parentNode: node => node.parent,
    nextSibling(node) { return node.parent?.children[node.parent.children.indexOf(node) + 1] || null; },
  });
  const props = vue.reactive({ projects: [{ id: 9001, name: 'Main', status: 'active' }, { id: 9002, name: 'Other', status: 'active' }], projectId: 9001,
    ...(name === 'PageIsolationDialog' ? { asset: asset() } : {}), ...initialProps });
  const component = load(name), root = makeNode('root');
  const app = renderer.createApp({ render: () => vue.h(component, { ...props,
    onNotify: (...args) => events.push(['notify', ...args]), onChanged: () => events.push(['changed']), onClose: () => events.push(['close']),
  }) });
  app.mount(root); await flush();
  const text = node => node.text + node.children.map(text).join('');
  const nodes = () => { const result = []; function walk(node) { result.push(node); node.children.forEach(walk); } walk(root); return result; };
  const buttons = label => nodes().filter(node => node.type === 'button' && (typeof label === 'string' ? text(node).includes(label) : label.test(text(node))));
  return { props, calls, events, records, root, text: () => text(root), nodes, buttons,
    state: (componentName = name) => records.findLast(record => record.name === componentName && record.active)?.bindings,
    async click(label) { const node = buttons(label)[0]; assert.ok(node, `Missing rendered button: ${label}`); assert.ok(!node.props.disabled, `Rendered button disabled: ${label}`); await node.props.onClick(); await flush(); },
    async model(type, value, index = 0) { const node = nodes().filter(n => n.type === type)[index]; assert.ok(node); assert.equal(typeof node.props['onUpdate:modelValue'], 'function'); node.props['onUpdate:modelValue'](value); await flush(); },
    unmount: () => app.unmount(),
  };
}
module.exports = { assert, vue, flush, deferred, asset, rule, preview, mountReview };
