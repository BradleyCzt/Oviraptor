const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { liveLogHarness } = require('./live_runner_log_harness.cjs');
const flush = async () => { for (let i = 0; i < 10; i++) await vue.nextTick(); };
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const row = id => ({ id, runId: 1, level: 'info', stage: 'test', message: 'same', createdAt: 'fixture-time' });
const snapshot = () => [row(2), row(1)];
const hint = (projectId = 10) => ({ logId: 3, runId: 1, projectId, message: 'untrusted-event-secret' });

async function mount(read = snapshot, options = {}) {
  options.channel = 'asset-log-committed';
  const live = liveLogHarness(options), calls = [], exports = {};
  const source = fs.readFileSync(path.join(__dirname, '../src/features/assets/useAssetLogPanel.ts'), 'utf8');
  new Function('require', 'exports', ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText)(name => {
    if (name === 'vue') return vue;
    if (name === '../../composables/useCommittedRefresh') return live.committed;
    if (name === './api') return { assetApi: { listLogs: async (...args) => { calls.push(args); return read(...args); } } };
    throw Error(`Unexpected asset panel import ${name}`);
  }, exports);
  const selection = vue.reactive({ project: 10, active: true });
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {},
    parentNode: () => null, nextSibling: () => null });
  let state;
  const app = renderer.createApp({ setup() {
    state = exports.useAssetLogPanel(() => selection.project, () => selection.active);
    return () => null;
  } });
  app.mount({}); await flush();
  return { state, live, calls, selection, unmount: () => app.unmount() };
}

test('asset panel reads bounded committed rows, coalesces hints and preserves repeated text', async t => {
  const h = await mount(); t.after(h.unmount); h.live.fire(); await flush();
  assert.deepEqual(h.calls, [[undefined, 300, 10]]);
  assert.deepEqual(h.state.logs.value, snapshot());
  for (let i = 0; i < 50; i++) h.live.emit(hint());
  assert.equal(h.live.timers, 1); h.live.fire(); await flush();
  assert.equal(h.calls.length, 2); assert.ok(!JSON.stringify(h.state.logs.value).includes('untrusted'));
  for (const invalid of [null, {}, hint(20), hint(null), hint('10'), { ...hint(), logId: 0 }, { ...hint(), runId: -1 }]) h.live.emit(invalid);
  assert.equal(h.live.timers, 0);
  h.selection.project = undefined; await flush(); h.live.fire(); await flush();
  h.live.emit(hint(null)); h.live.fire(); await flush();
  assert.deepEqual(h.calls.at(-1), [undefined, 300, undefined]);
});

test('asset project roundtrips fence pending responses and immediately clear stale rows', async t => {
  const pending = deferred(); let count = 0;
  const h = await mount(() => ++count === 2 ? pending.promise : snapshot()); t.after(h.unmount);
  h.live.fire(); await flush(); const reading = h.state.refresh();
  h.selection.project = 20; assert.deepEqual(h.state.logs.value, []);
  h.selection.project = 10; pending.resolve([row(99)]); await reading; await flush();
  assert.deepEqual(h.state.logs.value, []);
  h.live.fire(); await flush(); assert.deepEqual(h.state.logs.value, snapshot());
});

test('asset notifications during manual reads cause one follow-up snapshot', async t => {
  const pending = deferred(); let count = 0;
  const h = await mount(() => ++count === 1 ? pending.promise : [row(3)]); t.after(h.unmount);
  const reading = h.state.refresh(); h.live.emit(hint()); h.live.emit(hint()); h.live.fire(); await flush();
  assert.equal(count, 1); pending.resolve(snapshot()); await reading; await flush();
  h.live.fire(); await flush(); assert.equal(count, 2); assert.equal(h.state.logs.value[0].id, 3);
});

test('asset failed reads retain snapshot and recover without exposing raw error text', async t => {
  let fail = false;
  const h = await mount(() => { if (fail) throw Error('Bearer private-token'); return snapshot(); }); t.after(h.unmount);
  h.live.fire(); await flush(); fail = true; await h.state.refresh();
  assert.deepEqual(h.state.logs.value, snapshot()); assert.equal(h.state.readFailed.value, true);
  fail = false; h.live.reconcile(); h.live.fire(); await flush(); assert.equal(h.state.readFailed.value, false);
});

test('asset invalid snapshots are rejected and never replace committed rows', async t => {
  let rows = snapshot(); const h = await mount(() => rows); t.after(h.unmount); h.live.fire(); await flush();
  for (const invalid of [null, [row(1), row(2)], [row(1), row(1)], [row(-1)], [{ ...row(1), message: {} }],
    [{ ...row(1), runId: -1 }], Array.from({ length: 301 }, (_, i) => row(301 - i))]) {
    rows = invalid; await h.state.refresh();
    assert.equal(h.state.readFailed.value, true); assert.deepEqual(h.state.logs.value, snapshot());
  }
});

test('asset hidden and inactive panels pause automatic reads then catch up', async t => {
  const h = await mount(); t.after(h.unmount); h.live.fire(); await flush();
  h.live.visibility(true); h.live.emit(hint()); h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.calls.length, 1);
  h.live.visibility(false); h.live.fire(); await flush(); assert.equal(h.calls.length, 2);
  h.selection.active = false; await flush(); h.live.emit(hint()); h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.calls.length, 2);
  h.selection.active = true; await flush(); h.live.fire(); await flush(); assert.equal(h.calls.length, 3);
});

test('asset late subscription catches up and failed subscription retries', async t => {
  const registration = deferred(), options = { registration: registration.promise, failure: true };
  const h = await mount(undefined, options); t.after(h.unmount); await h.state.refresh();
  registration.resolve(); await flush(); assert.equal(h.state.connectionUnavailable.value, true);
  options.failure = false; h.live.reconcile(); await flush(); h.live.fire(); await flush();
  assert.equal(h.state.connectionUnavailable.value, false); assert.equal(h.calls.length, 2);
});

test('asset unmount releases timers and listeners and fences in-flight responses', async () => {
  const pending = deferred(); const h = await mount(() => pending.promise);
  h.live.fire(); await flush(); h.unmount(); pending.resolve(snapshot()); await flush();
  assert.deepEqual(h.state.logs.value, []); assert.equal(h.live.releases, 1);
  assert.equal(h.live.timers, 0); assert.equal(h.live.intervals, 0); assert.equal(h.live.observing, false);
});

test('asset late listener registration is released after unmount', async () => {
  const registration = deferred(); const h = await mount(undefined, { registration: registration.promise });
  h.unmount(); registration.resolve(); await flush();
  assert.equal(h.live.releases, 1); assert.equal(h.live.listening, false); assert.equal(h.calls.length, 0);
});
