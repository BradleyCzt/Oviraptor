// Mount the real trace page and its production composables; substitute IPC and time only.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');

const sourceRoot = path.join(__dirname, '../src');
const page = path.join(sourceRoot, 'features/sentinel/components/AgentTraceHub.vue');
const transpile = (code) => ts.transpileModule(code, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const flush = async () => { for (let i = 0; i < 10; i++) await vue.nextTick(); };
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const trace = (scanId, marker = '', status = 'scanning') => ({
  summary: {
    scanId, taskName: scanId, projectName: 'project', scanType: 'web', status,
    sourceAuthority: 'native_ledger', model: marker, tools: [], instructionHash: '',
    createdAt: '', updatedAt: '', exactRequestCapture: false, tokenUsageEstimated: false,
    ...Object.fromEntries(['runCount', 'agentCount', 'messageCount', 'reasoningCount',
      'toolCallCount', 'toolResultCount', 'llmRequests', 'inputTokens', 'outputTokens',
      'cachedTokens', 'totalTokens', 'hookedRequestCount', 'usageEntryCount',
      'usageAgentCount'].map((key) => [key, 0])),
  },
  events: [],
});
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});

async function mountTrace(overrides = {}, options = {}) {
  const timers = new Map(), calls = [], notices = [], cache = new Map();
  const scheduled = new Map(), listeners = new Map(), visibilityListeners = new Set();
  let timerId = 0, unmounted = false;
  const clock = {
    setInterval(fn, delay) { assert.equal(delay, 15000); timers.set(++timerId, fn); return timerId; },
    clearInterval(id) { timers.delete(id); },
    setTimeout(fn, delay) { assert.equal(delay, 50); scheduled.set(++timerId, fn); return timerId; },
    clearTimeout(id) { scheduled.delete(id); },
  };
  const document = { hidden: false,
    addEventListener(name, callback) { assert.equal(name, 'visibilitychange'); visibilityListeners.add(callback); },
    removeEventListener(name, callback) { assert.equal(name, 'visibilitychange'); visibilityListeners.delete(callback); },
  };
  const methods = {
    listAgentTraces: async () => [trace('scan-a').summary, trace('scan-b').summary],
    listAgentKnowledge: async () => [], listAgentLearningCandidates: async () => [],
    listAgentSkills: async () => [], getAgentTrace: async (id) => trace(id), ...overrides,
  };
  const api = new Proxy({}, { get: (_, name) => (...args) => {
    assert.equal(typeof methods[name], 'function', `Unexpected IPC: ${String(name)}`);
    calls.push([name, ...args]);
    return methods[name](...args);
  } });
  function load(filename) {
    if (cache.has(filename)) return cache.get(filename);
    const exports = {};
    cache.set(filename, exports);
    const source = fs.readFileSync(filename, 'utf8');
    const requireLocal = (name) => {
      if (name === '@tauri-apps/api/event') return { listen: options.listen || (async (channel, callback) => {
        assert.equal(channel, 'nest://collaboration-event');
        listeners.set(channel, callback);
        return () => listeners.delete(channel);
      }) };
      if (name === '@tauri-apps/plugin-dialog') return { open: async () => null };
      if (!name.startsWith('.')) return require(name);
      const resolved = path.resolve(path.dirname(filename), name);
      if (resolved === path.join(sourceRoot, 'features/sentinel/api')) return { sentinelApi: api };
      if (resolved === path.join(sourceRoot, 'features/sentinel/learningApi')) return { scanningLearningApi: api };
      if (resolved === path.join(sourceRoot, 'features/skills/catalogApi')) return { instructionCatalogApi: api, knowledgeCatalogApi: api };
      if (resolved === path.join(sourceRoot, 'i18n')) return { useI18n: () => ({ tr: (zh) => zh }) };
      return load(path.extname(resolved) ? resolved : `${resolved}.ts`);
    };
    if (filename.endsWith('.vue')) {
      const { descriptor } = parse(source, { filename });
      const script = compileScript(descriptor, { id: filename });
      const template = compileTemplate({ source: descriptor.template.content, filename,
        id: filename, compilerOptions: { bindingMetadata: script.bindings } });
      assert.deepEqual(template.errors, []);
      new Function('require', 'exports', 'window', 'document', transpile(script.content))(
        requireLocal, exports, clock, document);
      const rendered = {};
      new Function('require', 'exports', transpile(template.code))(requireLocal, rendered);
      exports.default.render = rendered.render;
    } else {
      new Function('require', 'exports', 'window', 'document', transpile(source))(
        requireLocal, exports, clock, document);
    }
    return exports;
  }
  const component = load(page).default;
  let bindings;
  const app = renderer.createApp({ ...component, setup(props, context) {
    bindings = component.setup(props, context);
    return () => null;
  } }, { onNotify: (...args) => notices.push(args) });
  app.mount({});
  await flush();
  return {
    b: bindings, timers, scheduled, listeners, visibilityListeners, calls, notices, document,
    emit: payload => { for (const callback of listeners.values()) callback({ payload }); },
    drain: async () => {
      const pending = [...scheduled.values()]; scheduled.clear();
      for (const callback of pending) callback();
      await flush();
    },
    visibility: async hidden => {
      document.hidden = hidden;
      for (const callback of visibilityListeners) callback();
      await flush();
    },
    tick: async () => { for (const fn of [...timers.values()]) fn(); await flush(); },
    unmount: () => { if (!unmounted) { unmounted = true; app.unmount(); } },
    render: () => renderToString(vue.createSSRApp({ render: () =>
      component.render({}, [], {}, vue.proxyRefs(bindings), {}, {}) })),
  };
}

module.exports = { mountTrace, trace, flush, deferred };
