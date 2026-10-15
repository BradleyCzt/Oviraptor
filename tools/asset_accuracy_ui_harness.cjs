// Compile and render the real asset SFC setup and template. Relative TS/Vue
// imports execute their production source; only the local host (IPC, icons,
// translations and native DOM directives) is scripted. This is not a desktop,
// database, browser layout, or external-provider acceptance harness.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const repositoryRoot = path.resolve(__dirname, '..');
const flush = async () => { for (let index = 0; index < 20; index++) await vue.nextTick(); };
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const transpile = (source, filename) => {
  const output = ts.transpileModule(source, { fileName: filename, reportDiagnostics: true,
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } });
  assert.deepEqual(output.diagnostics, [], `Transpilation diagnostics: ${filename}`);
  return output.outputText;
};
const storage = () => {
  const entries = new Map();
  return { getItem: key => entries.get(key) ?? null, setItem: (key, value) => entries.set(key, String(value)), removeItem: key => entries.delete(key) };
};

function makeLoader(options = {}) {
  const modules = new Map(), records = [], calls = [], inputs = new Map(), listeners = new Map();
  const locale = options.locale ?? 'en';
  const renderVue = { ...vue, vModelText: {}, vModelSelect: {}, vModelCheckbox: {} };
  const host = {
    localStorage: storage(), sessionStorage: storage(),
    window: { addEventListener() {}, removeEventListener() {}, getSelection: () => null,
      setTimeout, clearTimeout, setInterval, clearInterval },
    document: { elementFromPoint: () => null, addEventListener() {}, removeEventListener() {}, hidden: false }, navigator: {}, ...options.host,
  };
  function read(filename) {
    const bytes = fs.readFileSync(filename);
    inputs.set(path.relative(repositoryRoot, filename), crypto.createHash('sha256').update(bytes).digest('hex'));
    return bytes.toString('utf8');
  }
  function resolveRelative(imported, importer) {
    const requested = path.resolve(path.dirname(importer), imported);
    assert.ok(requested.startsWith(repositoryRoot + path.sep), `Import outside repository: ${imported}`);
    const candidates = [requested, `${requested}.ts`, `${requested}.vue`, path.join(requested, 'index.ts')];
    const filename = candidates.find(candidate => fs.existsSync(candidate) && fs.statSync(candidate).isFile());
    assert.ok(filename, `Cannot resolve ${imported} from ${path.relative(repositoryRoot, importer)}`);
    return filename;
  }
  function localRequire(imported, importer, template = false) {
    if (imported === 'vue') return template ? renderVue : vue;
    if (imported === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (imported === '@tauri-apps/api/core') return { invoke: async (command, args) => {
      calls.push([command, args]);
      assert.equal(typeof options.commands?.[command], 'function', `Unexpected asset accuracy IPC: ${command}`);
      return options.commands[command](args);
    } };
    if (imported === '@tauri-apps/api/event') return { listen: async (channel, callback) => {
      assert.ok(['job-progress', 'asset-log-committed'].includes(channel), `Unexpected asset event subscription: ${channel}`);
      const callbacks = listeners.get(channel) ?? new Set();
      callbacks.add(callback); listeners.set(channel, callbacks);
      return () => callbacks.delete(callback);
    } };
    if (imported === '@tauri-apps/plugin-opener') return { openUrl: async value => {
      calls.push(['openUrl', value]);
      assert.equal(typeof options.openUrl, 'function', 'Unexpected external URL open');
      return options.openUrl(value);
    } };
    assert.ok(imported.startsWith('.'), `Unexpected host import: ${imported}`);
    const filename = resolveRelative(imported, importer);
    if (filename === path.join(repositoryRoot, 'src/i18n.ts')) return { useI18n: () => ({
      tr: (zh, en) => locale === 'zh' ? zh : en,
      t: vue.computed(() => ({ refresh: 'Refresh', newQuery: 'New query' })),
    }) };
    if (filename.endsWith('.css')) { read(filename); return {}; }
    return load(filename);
  }
  function execute(source, filename, exports, template = false) {
    const module = { exports };
    new Function('require', 'module', 'exports', '__filename', '__dirname', 'localStorage', 'sessionStorage', 'window', 'document', 'navigator', transpile(source, filename))(
      imported => localRequire(imported, filename, template), module, exports, filename, path.dirname(filename),
      host.localStorage, host.sessionStorage, host.window, host.document, host.navigator);
    return module.exports;
  }
  function load(filename) {
    if (modules.has(filename)) return modules.get(filename);
    const exports = {};
    modules.set(filename, exports);
    const source = read(filename);
    if (!filename.endsWith('.vue')) {
      const result = execute(source, filename, exports);
      modules.set(filename, result);
      return result;
    }
    const parsed = parse(source, { filename });
    assert.deepEqual(parsed.errors, [], `SFC parsing errors: ${filename}`);
    const id = path.relative(repositoryRoot, filename);
    const script = compileScript(parsed.descriptor, { id,
      fs: { fileExists: ts.sys.fileExists, readFile: filename => fs.existsSync(filename) ? read(filename) : undefined } });
    const template = compileTemplate({ source: parsed.descriptor.template?.content ?? '', filename, id,
      compilerOptions: { bindingMetadata: script.bindings } });
    assert.deepEqual(template.errors, [], `Template compilation errors: ${filename}`);
    execute(script.content, filename, exports);
    const rendered = execute(template.code, filename, {}, true);
    const production = exports.default;
    const component = { ...production, setup(props, context) {
      const bindings = production.setup(props, context);
      assert.equal(typeof bindings, 'object', `Expected actual setup bindings: ${filename}`);
      const record = { filename: id, bindings, active: true };
      records.push(record);
      vue.onUnmounted(() => { record.active = false; });
      return () => rendered.render({}, [], props, vue.proxyRefs(bindings), {}, {});
    } };
    const result = { ...exports, default: component };
    modules.set(filename, result);
    return result;
  }
  return { load: relative => load(path.resolve(repositoryRoot, relative)), records, calls, inputs,
    emitNative: (channel, payload) => { for (const callback of listeners.get(channel) ?? []) callback({ payload }); } };
}

const makeNode = (type, text = '') => ({ type, text, props: {}, children: [], parent: null });
function makeRenderer() {
  return vue.createRenderer({
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
    nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  });
}
const textOf = node => String(node.text ?? '') + node.children.map(textOf).join('');
const collectNodes = root => { const nodes = []; const walk = node => { nodes.push(node); node.children.forEach(walk); }; walk(root); return nodes; };
async function mountAssetAccuracy(relative, initialProps, options = {}) {
  const loader = makeLoader(options), component = loader.load(relative).default;
  const props = vue.reactive({ ...initialProps }), root = makeNode('root'), events = [], warnings = [];
  const listeners = {};
  for (const name of options.events ?? ['notify', 'started', 'projectChange', 'createProject']) {
    listeners[`on${name[0].toUpperCase()}${name.slice(1)}`] = (...args) => events.push([name, ...args]);
  }
  const app = makeRenderer().createApp({ render: () => vue.h(component, { ...props, ...listeners }) });
  app.config.warnHandler = message => warnings.push(message);
  app.mount(root);
  await flush();
  return {
    ...loader, root, props, events, warnings,
    text: () => textOf(root), nodes: () => collectNodes(root),
    matching: (attribute, value) => collectNodes(root).filter(node => Object.hasOwn(node.props, attribute) && (value === undefined || node.props[attribute] === value)),
    state: (filename = relative) => loader.records.findLast(record => record.filename === filename && record.active)?.bindings,
    async model(attribute, value, modelValue) {
      const nodes = collectNodes(root).filter(node => node.props[attribute] === value);
      assert.equal(nodes.length, 1, `Expected one rendered ${attribute}=${value}`);
      assert.equal(typeof nodes[0].props['onUpdate:modelValue'], 'function');
      nodes[0].props['onUpdate:modelValue'](modelValue); await flush();
    },
    async click(attribute, value) {
      const nodes = collectNodes(root).filter(node => node.props[attribute] === value);
      assert.equal(nodes.length, 1, `Expected one rendered ${attribute}=${value}`);
      assert.ok(!nodes[0].props.disabled, `Disabled rendered ${attribute}=${value}`);
      assert.equal(typeof nodes[0].props.onClick, 'function');
      await nodes[0].props.onClick(); await flush();
    },
    unmount: () => app.unmount(),
  };
}

module.exports = { assert, vue, flush, deferred, textOf, makeLoader, mountAssetAccuracy, repositoryRoot };
