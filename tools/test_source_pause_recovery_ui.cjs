// Production SFC, template and API adapter; IPC is local and scripted.
const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const { loadSentinelApi } = require('./sentinel_api_harness.cjs');
const vue = require('vue');
const { parse, compileScript, compileTemplate, compileStyle } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const filename = path.join(__dirname, '../src/features/sentinel/components/NativeSourcePauseRecovery.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'source-result-recovery-test' });
const transpile = (source) => ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const template = compileTemplate({ source: descriptor.template.content, filename,
  id: 'source-result-recovery-test', compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const style = compileStyle({ source: descriptor.styles[0].content, filename,
  id: 'data-v-source-result-recovery-test', scoped: true });
assert.deepEqual(style.errors, []);
const templateExports = {};
new Function('require', 'exports', transpile(template.code))(require, templateExports);
const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}),
  createComment: () => ({}), insert() {}, remove() {}, setText() {}, setElementText() {},
  patchProp() {}, parentNode: () => null, nextSibling: () => null });
const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };
const receipt = (changes = {}) => ({ schemaVersion: 1, scanId: 'scan-a', attemptNumber: 1,
  rootRunId: 'original-root', scanStatus: 'paused', branchStatus: 'partial',
  changed: true, executionReplayed: false, ...changes });
function mount(recover = async () => receipt(), initial = {}) {
  const calls = [], events = [], exports = {};
  new Function('require', 'exports', transpile(script.content))((name) => {
    if (name === 'vue') return vue;
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '../api') return { sentinelApi: new Proxy({}, { get(_, key) {
      assert.equal(key, 'recoverNativeSourcePauseResult');
      return async (...args) => { calls.push(args); return recover(...args); };
    } }) };
    throw new Error(`Unexpected import: ${name}`);
  }, exports);
  let b;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) {
    b = setup(props, ctx);
    return () => templateExports.render({}, [], props, vue.proxyRefs(b), {}, {});
  } };
  const props = vue.reactive({ scanId: 'scan-a', attempt: 1, status: 'paused', eligible: true, ...initial });
  const app = renderer.createApp({ setup: () => () => vue.h(component,
    { ...props, onRefresh: () => events.push('refresh') }) });
  app.mount({});
  return { b, props, calls, events, unmount: () => app.unmount(),
    html: () => renderToString(vue.createSSRApp({ render: () =>
      templateExports.render({}, [], props, vue.proxyRefs(b), {}, {}) })) };
}

test('explicit click consumes original result and refreshes without executing or changing attempt', async () => {
  const h = mount(); await flush();
  assert.deepEqual(h.calls, []);
  assert.match(await h.html(), /恢复原结果/);
  await h.b.recoverResult(); await flush();
  assert.deepEqual(h.calls, [['scan-a', 1]]);
  assert.deepEqual(h.events, ['refresh']);
  assert.match(await h.html(), /费用保留，任务继续暂停/);
  assert.equal(h.props.status, 'paused'); assert.equal(h.props.attempt, 1);
  h.props.eligible = false; await flush();
  const html = await h.html(); assert.doesNotMatch(html, /<button/);
  assert.match(html, /原结果已恢复/); h.unmount();
});

test('duplicate clicks are single flight and original replay remains paused', async () => {
  let complete;
  const h = mount(() => new Promise(resolve => { complete = resolve; }));
  const action = h.b.recoverResult(); await flush();
  await h.b.recoverResult();
  assert.equal(h.calls.length, 1); assert.equal(h.b.busy.value, true);
  assert.match(await h.html(), /disabled/);
  complete(receipt({ changed: false })); await action;
  assert.equal(h.b.busy.value, false); assert.match(await h.html(), /原结果已核对/);
  assert.deepEqual(h.events, ['refresh']); h.unmount();
});

test('noneligible, active, disabled and invalid attempt views never issue recovery IPC', async () => {
  for (const initial of [{ eligible: false }, { status: 'scanning' }, { disabled: true },
    { scanId: '' }, { attempt: 0 }, { attempt: 1.5 }, { attempt: Number.MAX_SAFE_INTEGER + 1 }]) {
    const h = mount(undefined, initial); await flush(); await h.b.recoverResult();
    assert.deepEqual(h.calls, []); assert.deepEqual(h.events, []); h.unmount();
  }
});

test('late receipts cannot alter another scan, attempt, state transition or disposed view', async () => {
  for (const change of [h => { h.props.scanId = 'scan-b'; }, h => { h.props.attempt = 2; },
    async h => { h.props.status = 'scanning'; await flush(); h.props.status = 'paused'; },
    h => h.unmount()]) {
    let complete;
    const h = mount(() => new Promise(resolve => { complete = resolve; }));
    const action = h.b.recoverResult(); await change(h); await flush();
    complete(receipt()); await action; await flush();
    assert.equal(h.b.message.value, ''); assert.equal(h.b.error.value, '');
    assert.deepEqual(h.events, []); h.unmount();
  }
});

test('mismatched or execution-replay receipts cannot display recovery success', async () => {
  for (const value of [null, receipt({ schemaVersion: 2 }), receipt({ scanId: 'scan-b' }),
    receipt({ attemptNumber: 2 }), receipt({ scanStatus: 'scanning' }), receipt({ branchStatus: 'completed' }),
    receipt({ executionReplayed: true }), receipt({ rootRunId: '' }), receipt({ changed: 'true' })]) {
    const h = mount(async () => value); await h.b.recoverResult();
    assert.equal(h.b.message.value, ''); assert.match(await h.html(), /恢复未确认/);
    assert.equal(h.b.busy.value, false); assert.deepEqual(h.events, ['refresh']); h.unmount();
  }
});

test('busy, unsettled costs and commit errors retain obligations without exposing raw secrets', async () => {
  for (const [reason, label] of [['original_sdk_not_idle', '原执行或清理尚未确认结束'],
    ['model_usage_uncertain', '费用尚未确认'], ['commit_unconfirmed', '恢复未确认']]) {
    const h = mount(async () => { throw new Error(`${reason}: /private/db SELECT secret=sk-sensitive`); });
    await h.b.recoverResult();
    const html = await h.html(); assert.match(html, new RegExp(label));
    assert.doesNotMatch(html, /sk-sensitive|private\/db|SELECT/);
    assert.equal(h.b.message.value, ''); assert.equal(h.props.status, 'paused');
    assert.deepEqual(h.calls, [['scan-a', 1]]); h.unmount();
  }
});

test('production API adapter invokes the registered command with exact original scope', async () => {
  const calls = [];
  const exports = { sentinelApi: loadSentinelApi(async (...args) => { calls.push(args); return receipt(); }) };
  assert.deepEqual(await exports.sentinelApi.recoverNativeSourcePauseResult('scan-a', 1), receipt());
  assert.deepEqual(calls, [['recover_native_source_pause_result', { scanId: 'scan-a', attemptNumber: 1 }]]);
});
