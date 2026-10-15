// Compile and execute the production SFC; only IPC and timers are substituted.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const { parse, compileScript, compileTemplate, compileStyle } = require('@vue/compiler-sfc');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { liveLogHarness } = require('../live_runner_log_harness.cjs');
const filename = path.join(__dirname, '../../src/features/sentinel/components/NativeRunStatus.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'native-status-test' });
const transpile = (content) => ts.transpileModule(content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const labelsPath = path.join(path.dirname(filename), 'nativeRunStatusLabels.ts');
const labelsExports = {};
new Function('require', 'exports', transpile(fs.readFileSync(labelsPath, 'utf8')))((name) => {
  throw new Error(`Unexpected label import: ${name}`);
}, labelsExports);
const style = descriptor.styles[0];
assert.equal(style.scoped, true);
assert.equal(style.src, './nativeRunStatus.css');
const compiledStyle = compileStyle({ source: fs.readFileSync(path.join(path.dirname(filename), style.src), 'utf8'),
  filename: style.src, id: 'data-v-native-status-test', scoped: true });
assert.deepEqual(compiledStyle.errors, []);
assert.match(compiledStyle.code, /\.native-run-status\[data-v-native-status-test\]/);
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'native-status-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const templateExports = {};
new Function('require', 'exports', transpile(template.code))(require, templateExports);
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});
const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };
const state = (dispatchState) => ({ scanId: 'scan-a', attemptNumber: 1, status: 'scanning',
  branches: [{ branch: 'web', status: 'pending', checkpoint: '<script>not executable</script>',
    updatedAt: '', ...(dispatchState ? { dispatch: { state: dispatchState, claimedAt: null, automaticReplayAllowed: false } } : {}) }],
  timeline: [], unresolvedContainers: [], sourceGaps: [],
  stopDiagnostic: { category: 'active', code: 'in_progress', stage: 'branch', nextAction: 'wait_for_receipts', obligations: [] },
});
async function mount(getStatus, recover, close, administrative, options = {}) {
  const live = liveLogHarness({ ...options, channel: 'nest://collaboration-event' });
  const contract = {};
  new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
    '../../src/features/sentinel/timeline/eventContract.ts'), 'utf8')))(contract);
  const calls = [], recoveryCalls = [], closureCalls = [], administrativeCalls = [], events = [], timers = new Map();
  let timerSequence = 0;
  const api = new Proxy({}, { get(_, key) {
    if (key === 'previewWebAdministrativeClosure' && administrative) {
      return async (...args) => { administrativeCalls.push([key,...args]); return administrative.preview(...args); };
    }
    if (key === 'closeWebTaskAdministratively' && administrative) {
      return async (...args) => { administrativeCalls.push([key,...args]); return administrative.close(...args); };
    }
    if (key === 'closeNeverDispatchedWebAttempt' && close) {
      return async (...args) => { closureCalls.push(args); return close(...args); };
    }
    if (key === 'recoverNeverDispatchedWebAttempt' && recover) {
      return async (...args) => { recoveryCalls.push(args); return recover(...args); };
    }
    assert.equal(key, 'getNativeScanStatus', 'status view must never issue recovery/execution commands');
    return async (...args) => { calls.push(args); return getStatus(...args); };
  } });
  const exports = {};
  new Function('require', 'exports', 'setTimeout', 'clearTimeout', transpile(script.content))((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
    if (name === '../reconciliation/api') return { rootCostReconciliationApi: api };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === './nativeRunStatusLabels') return labelsExports;
    if (name === '../../../composables/useCommittedRefresh') return live.committed;
    if (name === '../timeline/eventContract') return contract;
    // NativeRunStatus owns status actions; the on-demand budget panel has its
    // own lifecycle and must not issue IPC while these status tests mount it.
    if (name === './NativeBudgetDiagnostics.vue') return { default: { render: () => null } };
    if (name === './NativeRootReconciliation.vue') return { default: { render: () => null } };
    // The new result-only panel is exercised by its own real SFC suite.
    if (name === './NativeSourcePauseRecovery.vue') return { default: { render: () => null } };
    if (name === './NativeProtectionRecovery.vue') return { default: { render: () => null } };
    throw new Error(`Unexpected import: ${name}`);
  }, exports, (fn) => { timers.set(++timerSequence, fn); return timerSequence; }, (id) => timers.delete(id));
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) { bindings = setup(props, ctx); return () => null; } };
  const props = vue.reactive({ scanId: 'scan-a', attempt: 1, status: 'scanning' });
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...props, onAttemptClosed: (...args) => events.push(args) }) });
  app.mount({});
  await flush();
  return { b: bindings, props, calls, recoveryCalls, closureCalls, administrativeCalls, events, timers, live, unmount: () => app.unmount(),
    render: () => renderToString(vue.createSSRApp({
      render: () => templateExports.render({}, [], {}, vue.proxyRefs(bindings), {}, {}),
    })),
  };
}

const recoverable = () => {
  const value = state('never_claimed');
  value.branches[0].dispatch.manualRecoveryAvailable = true;
  return value;
};
const recovered = (scanId = 'scan-a', attemptNumber = 1) => ({
  scanId, attemptNumber, dispatchState: 'claimed', executionState: 'submitted', automaticReplayAllowed: false,
});

module.exports = { assert, test, flush, state, mount, recoverable, recovered };
