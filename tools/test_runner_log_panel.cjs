const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { liveLogHarness } = require('./live_runner_log_harness.cjs');
const flush = async () => { for (let i = 0; i < 10; i++) await vue.nextTick(); };
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const snapshot = (scanId = 'A', attempt = 1, lines = ['same', 'same']) => ({ scanId, attempt, lines, status: 'ready' });

async function mount(read, options = {}) {
  const live = liveLogHarness(options), calls = [], exports = {};
  const source = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/execution/useRunnerLogPanel.ts'), 'utf8');
  new Function('require', 'exports', ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText)(name => {
    if (name === 'vue') return vue;
    if (name === './useLiveRunnerLog') return live.module;
    if (name === '../api') return { sentinelApi: { readSentinelRunnerLog: async (...args) => {
      calls.push(args); return read ? read(...args) : snapshot(args[0], args[1] || 1);
    } } };
    throw Error(`Unexpected panel import ${name}`);
  }, exports);
  const selection = vue.reactive({ scanId: 'A', attempt: 0, active: true });
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {},
    parentNode: () => null, nextSibling: () => null });
  let state;
  const app = renderer.createApp({ setup() {
    state = exports.useRunnerLogPanel(() => ({ scanId: selection.scanId, attempt: selection.attempt }), () => selection.active);
    return () => null;
  } });
  app.mount({}); await flush();
  return { state, live, calls, selection, unmount: () => app.unmount() };
}
const hint = (attempt = 1, scanId = 'A') => ({ scanId, attempt, line: 'untrusted-event-secret' });

test('global panel follows latest attempts, uses bounded snapshots and preserves repeated lines', async t => {
  let attempt = 1;
  const h = await mount(() => snapshot('A', attempt)); t.after(h.unmount);
  h.live.fire(); await flush();
  assert.deepEqual(h.calls, [['A', undefined, 300]]);
  assert.deepEqual(h.state.snapshot.value.lines, ['same', 'same']);
  attempt = 2;
  for (let i = 0; i < 50; i++) h.live.emit(hint(2));
  assert.equal(h.live.timers, 1); h.live.fire(); await flush();
  assert.equal(h.calls.length, 2); assert.equal(h.state.snapshot.value.attempt, 2);
  assert.ok(!JSON.stringify(h.state.snapshot.value).includes('untrusted'));
  for (const invalid of [null, {}, hint(0), hint(-1), hint('2'), hint(2, 'B')]) h.live.emit(invalid);
  assert.equal(h.live.timers, 0);
});

test('pinned round ignores other attempts and fencing clears old snapshots immediately', async t => {
  const h = await mount(); t.after(h.unmount); h.live.fire(); await flush();
  h.selection.attempt = 3;
  assert.equal(h.state.snapshot.value, undefined);
  await flush(); h.live.fire(); await flush();
  assert.equal(h.state.snapshot.value.attempt, 3);
  h.live.emit(hint(2)); assert.equal(h.live.timers, 0);
  h.live.emit(hint(3)); h.live.fire(); await flush();
  assert.deepEqual(h.calls.at(-1), ['A', 3, 300]);
});

test('events during initial/manual snapshot reads trigger one committed follow-up', async t => {
  const pending = deferred(); let reads = 0;
  const h = await mount(() => ++reads === 1 ? pending.promise : snapshot('A', 2)); t.after(h.unmount);
  const manual = h.state.refresh();
  h.live.emit(hint()); h.live.emit(hint(2)); h.live.fire(); await flush();
  assert.equal(reads, 1);
  pending.resolve(snapshot()); await manual; await flush();
  assert.equal(h.live.timers, 1); h.live.fire(); await flush();
  assert.equal(reads, 2); assert.equal(h.state.snapshot.value.attempt, 2);
});

test('late subscription reconciles logs written before registration', async t => {
  const registration = deferred(); const h = await mount(undefined, { registration: registration.promise });
  t.after(h.unmount); await h.state.refresh();
  assert.equal(h.calls.length, 1); assert.equal(h.live.listening, false);
  registration.resolve(); await flush(); h.live.fire(); await flush();
  assert.equal(h.calls.length, 2); assert.equal(h.live.listening, true);
});

test('failed reads keep the last snapshot and do not expose transport errors', async t => {
  let fail = false;
  const h = await mount(() => { if (fail) throw Error('Bearer private-token'); return snapshot(); });
  t.after(h.unmount); h.live.fire(); await flush(); fail = true;
  h.live.emit(hint()); h.live.fire(); await flush();
  assert.deepEqual(h.state.snapshot.value.lines, ['same', 'same']);
  assert.equal(h.state.readFailed.value, true);
  fail = false; h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.state.readFailed.value, false);
});

test('task round trips and clearing selection fence both success and finally', async t => {
  const old = deferred(), newer = deferred(); let reads = 0;
  const h = await mount(() => ++reads === 1 ? old.promise : newer.promise); t.after(h.unmount);
  const first = h.state.refresh();
  h.selection.scanId = 'B'; h.selection.scanId = 'A';
  const second = h.state.refresh();
  old.resolve(snapshot('A', 99)); await first;
  assert.equal(h.state.snapshot.value, undefined); assert.equal(h.state.loading.value, true);
  h.selection.scanId = ''; newer.resolve(snapshot()); await second;
  assert.equal(h.state.snapshot.value, undefined); assert.equal(h.state.loading.value, false);
});

test('mismatched snapshot is rejected without displaying another task', async t => {
  const h = await mount(() => snapshot('B')); t.after(h.unmount);
  h.live.fire(); await flush();
  assert.equal(h.state.snapshot.value, undefined); assert.equal(h.state.readFailed.value, true);
});

test('hidden and inactive panels pause reads then reconcile on return', async t => {
  const h = await mount(); t.after(h.unmount); h.live.fire(); await flush();
  h.live.visibility(true); h.live.emit(hint()); h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.calls.length, 1);
  h.live.visibility(false); h.live.fire(); await flush(); assert.equal(h.calls.length, 2);
  h.selection.active = false; await flush(); h.live.emit(hint()); h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.calls.length, 2);
  h.selection.active = true; await flush(); h.live.fire(); await flush(); assert.equal(h.calls.length, 3);
});

test('failed subscription recovers and unmount releases listeners and fences late reads', async () => {
  const options = { failure: true }, pending = deferred();
  const h = await mount(() => pending.promise, options);
  assert.equal(h.state.connectionUnavailable.value, true);
  options.failure = false; h.live.reconcile(); await flush();
  assert.equal(h.state.connectionUnavailable.value, false);
  h.live.fire(); await flush(); h.unmount(); pending.resolve(snapshot()); await flush();
  assert.equal(h.state.snapshot.value, undefined);
  assert.equal(h.live.releases, 1); assert.equal(h.live.timers, 0);
  assert.equal(h.live.intervals, 0); assert.equal(h.live.observing, false);
});
