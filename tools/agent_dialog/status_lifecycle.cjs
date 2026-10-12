// Execute the production status composable; only the host document, clock and
// IPC transports are fixtures. Visibility and subscription work are observed.
const { assert, test, deferred, flush, eventRefreshClock, status } = require('../agent_dialog_harness.cjs');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const sourceRoot = path.join(__dirname, '../../src/features/sentinel');
const compile = (file) => ts.transpileModule(fs.readFileSync(path.join(sourceRoot, file), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const projection = {}, cursor = {}, event = {};
new Function('require', 'exports', compile('timeline/projectionContract.ts'))(name =>
  name === './rootDecisionContract' ? require('../result_component_harness.cjs')
    .loadWorkspaceModule('src/features/sentinel/timeline/rootDecisionContract.ts') : require(name), projection);
new Function('require', 'exports', compile('timeline/statusContract.ts'))(
  (name) => name === './projectionContract' ? projection : require(name), cursor);
new Function('exports', compile('timeline/eventContract.ts'))(event);
const compiled = compile('composables/useAgentDialogStatus.ts');

function setup(options = {}) {
  const clock = eventRefreshClock(), handlers = new Set(), reads = [];
  const document = {
    hidden: !!options.hidden,
    addEventListener(name, handler) { assert.equal(name, 'visibilitychange'); handlers.add(handler); },
    removeEventListener(name, handler) { assert.equal(name, 'visibilitychange'); handlers.delete(handler); },
  };
  const api = { getNativeScanStatus: async (...args) => {
    reads.push(args); return options.read ? options.read(...args) : status(args[0]);
  } };
  let subscriptions = 0, released = 0, notify, generation = 0, closed = false;
  const listening = async (name, callback) => {
    assert.equal(name, 'nest://collaboration-event'); subscriptions++; notify = callback;
    return options.listen ? options.listen(subscriptions) : () => { released++; };
  };
  const exports = {};
  new Function('require', 'exports', 'document', compiled)((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
    if (name === '@tauri-apps/api/event') return { listen: listening };
    if (name === '../timeline/projectionContract') return projection;
    if (name === '../timeline/statusContract') return cursor;
    if (name === '../timeline/humanAssessmentContract') return require('../result_component_harness.cjs').loadWorkspaceModule('src/features/sentinel/timeline/humanAssessmentContract.ts');
    if (name === '../timeline/eventContract') return event;
    throw Error(`Unexpected status import: ${name}`);
  }, exports, document);
  const scanId = vue.ref('A'), selected = vue.ref({ id: 'A', status: 'completed' });
  const sync = exports.useAgentDialogStatus({ scanId, selected,
    captureTaskView: () => {
      const current = generation, id = scanId.value;
      return { scanId: id, isCurrent: () => !closed && generation === current && id === scanId.value };
    },
    isDisposed: () => closed, loadScans: async () => {}, tr: (zh) => zh,
    schedule: clock.setTimeout, cancel: clock.clearTimeout,
  });
  return { sync, clock, reads, api, document, handlers,
    get subscriptions() { return subscriptions; }, get released() { return released; },
    send(sequence, attemptNumber = 1) { notify?.({ payload: { scanId: scanId.value, sequence, attemptNumber } }); },
    visibility(hidden) { document.hidden = hidden; for (const handler of handlers) handler(); },
    select(id) { generation++; scanId.value = id; selected.value = { id, status: 'completed' }; sync.resetForTask(); },
    dispose() { closed = true; sync.dispose(); },
  };
}

test('chat retries a failed subscription once and catches up the registration gap from its cursor', async (t) => {
  const h = setup({ listen: async (count) => {
    if (count === 1) throw Error('subscription unavailable'); return () => {};
  }, read: async () => ({ ...status('A'), latestSequence: 4 }) }); t.after(h.dispose);
  await h.sync.loadStatus(); await h.sync.installCollaborationListener(); await flush();
  assert.ok(h.sync.listenerError.value);
  assert.equal(h.clock.timers.size, 1, 'polling and reconnect share one fallback timer');
  h.api.getNativeScanStatus = async (...args) => {
    h.reads.push(args); return { ...status('A'), isIncremental: true, latestSequence: 5 };
  };
  await h.clock.tick(3000); await h.clock.tick(50);
  assert.equal(h.subscriptions, 2, 'the failed registration is retried');
  assert.equal(h.sync.listenerError.value, '');
  assert.deepEqual(h.reads, [['A', undefined, undefined], ['A', 4, 1]]);
  assert.equal(h.sync.latestSequence.value, 5);
  assert.deepEqual([...h.clock.timers.values()].map(row => row.delay), [15000]);
  await h.sync.installCollaborationListener();
  assert.equal(h.subscriptions, 2, 'a connected listener is not duplicated');
});

test('hidden chat cancels queued reads and resumes with one persisted catch-up despite event bursts', async (t) => {
  const h = setup({ read: async () => ({ ...status('A'), latestSequence: 4 }) }); t.after(h.dispose);
  await h.sync.loadStatus(); await h.sync.installCollaborationListener(); await h.clock.tick(50);
  h.reads.length = 0;
  h.send(5);
  h.visibility(true);
  assert.equal(h.clock.timers.size, 0, 'visibility stops both event and fallback timers');
  for (let sequence = 6; sequence <= 100; sequence++) h.send(sequence);
  await h.clock.tick(50); await h.clock.tick(15000); await h.sync.loadStatus();
  assert.deepEqual(h.reads, [], 'hidden views start no background IPC');
  h.api.getNativeScanStatus = async (...args) => {
    h.reads.push(args); return { ...status('A'), isIncremental: true, latestSequence: 100 };
  };
  h.visibility(false); h.visibility(false);
  await h.clock.tick(50);
  assert.deepEqual(h.reads, [['A', 4, 1]]);
  assert.equal(h.sync.latestSequence.value, 100);
  assert.deepEqual([...h.clock.timers.values()].map(row => row.delay), [15000]);
});

test('an initially hidden chat neither subscribes nor reads until it becomes visible', async (t) => {
  const h = setup({ hidden: true }); t.after(h.dispose);
  await h.sync.installCollaborationListener(); await h.sync.loadStatus();
  assert.equal(h.subscriptions, 0);
  assert.deepEqual(h.reads, []);
  assert.equal(h.clock.timers.size, 0);
  h.visibility(false); await flush(); await h.clock.tick(50);
  assert.equal(h.subscriptions, 1);
  assert.deepEqual(h.reads, [['A', undefined, undefined]]);
});

test('explicit task navigation reads one persisted snapshot while native window visibility lags', async (t) => {
  const h = setup({ hidden: true }); t.after(h.dispose);
  await h.sync.loadStatus(undefined, true);
  assert.deepEqual(h.reads, [['A', undefined, undefined]]);
  assert.equal(h.sync.state.value.scanId, 'A');
  assert.equal(h.subscriptions, 0);
  assert.equal(h.clock.timers.size, 0, 'explicit navigation must not start hidden background polling');
  await h.sync.loadStatus();
  assert.equal(h.reads.length, 1);
  h.select('B');
  await h.sync.loadStatus(undefined, true);
  assert.equal(h.sync.state.value.scanId, 'B');
  h.dispose();
  await h.sync.loadStatus(undefined, true);
  assert.equal(h.reads.length, 2, 'user refresh cannot revive a disposed chat');
});

test('visibility recovery waits for an unresolved current read and schedules only one trailing catch-up', async (t) => {
  const h = setup({ read: async () => ({ ...status('A'), latestSequence: 4 }) }); t.after(h.dispose);
  await h.sync.loadStatus(); await h.sync.installCollaborationListener(); await h.clock.tick(50);
  const pending = deferred(); h.reads.length = 0;
  h.api.getNativeScanStatus = (...args) => {
    h.reads.push(args);
    return h.reads.length === 1 ? pending.promise
      : Promise.resolve({ ...status('A'), isIncremental: true, latestSequence: 7 });
  };
  h.send(5); await h.clock.tick(50);
  h.visibility(true); h.visibility(false); h.visibility(false);
  await h.clock.tick(50);
  assert.equal(h.reads.length, 1, 'resume cannot overlap the current IPC');
  pending.resolve({ ...status('A'), isIncremental: true, latestSequence: 5 });
  await flush(); await h.clock.tick(50);
  assert.deepEqual(h.reads, [['A', 4, 1], ['A', 5, 1]]);
  assert.equal(h.sync.latestSequence.value, 7);
  assert.equal(h.clock.timers.size, 1);
});

test('a read covering queued hints retains the single slow fallback without a redundant IPC', async (t) => {
  const h = setup({ read: async () => ({ ...status('A'), latestSequence: 4 }) }); t.after(h.dispose);
  await h.sync.loadStatus(); await h.sync.installCollaborationListener(); await h.clock.tick(50);
  const pending = deferred(); h.reads.length = 0;
  h.api.getNativeScanStatus = (...args) => { h.reads.push(args); return pending.promise; };
  h.send(5); await h.clock.tick(50); h.send(6);
  pending.resolve({ ...status('A'), isIncremental: true, latestSequence: 6 });
  await flush(); await h.clock.tick(50);
  assert.deepEqual(h.reads, [['A', 4, 1]]);
  assert.deepEqual([...h.clock.timers.values()].map(row => row.delay), [15000]);
});

test('listener reconnection is not blocked by an unresolved status read and catch-up still waits for that read', async (t) => {
  const pending = deferred();
  const h = setup({ read: () => pending.promise, listen: async (count) => {
    if (count === 1) throw Error('unavailable'); return () => {};
  } }); t.after(h.dispose);
  const initial = h.sync.loadStatus();
  await h.sync.installCollaborationListener();
  await h.clock.tick(3000);
  assert.equal(h.subscriptions, 2, 'a stalled IPC does not suppress event transport recovery');
  assert.equal(h.reads.length, 1, 'transport recovery cannot start a parallel status IPC');
  h.api.getNativeScanStatus = async (...args) => {
    h.reads.push(args); return { ...status('A'), isIncremental: true, latestSequence: 6 };
  };
  pending.resolve({ ...status('A'), latestSequence: 5 });
  await initial; await h.clock.tick(50);
  assert.deepEqual(h.reads, [['A', undefined, undefined], ['A', 5, 1]]);
  assert.deepEqual([...h.clock.timers.values()].map(row => row.delay), [15000]);
});

test('late retry registration is released after disposal and cannot read, rearm timers or retain visibility handlers', async () => {
  const pending = deferred(); let released = 0;
  const h = setup({ listen: (count) => count === 1 ? Promise.reject(Error('unavailable')) : pending.promise });
  await h.sync.loadStatus(); await h.sync.installCollaborationListener();
  await h.clock.tick(3000);
  assert.equal(h.subscriptions, 2);
  const reads = h.reads.length;
  h.dispose();
  assert.equal(h.handlers.size, 0);
  assert.equal(h.clock.timers.size, 0);
  pending.resolve(() => { released++; }); await flush();
  assert.equal(released, 1);
  assert.equal(h.reads.length, reads);
  assert.equal(h.clock.timers.size, 0);
  h.visibility(false); h.send(100); await h.clock.tick(50);
  assert.equal(h.reads.length, reads);
});

test('hiding a disconnected chat suspends retries; recovery remains single-flight and releases its listener', async () => {
  const pending = deferred(); let released = 0;
  const h = setup({ listen: (count) => count === 1 ? Promise.reject(Error('unavailable')) : pending.promise });
  try {
    await h.sync.loadStatus(); await h.sync.installCollaborationListener();
    h.visibility(true); await h.clock.tick(3000); await h.clock.tick(15000);
    assert.equal(h.subscriptions, 1);
    assert.equal(h.clock.timers.size, 0);
    h.visibility(false); h.visibility(false); void h.sync.installCollaborationListener();
    assert.equal(h.subscriptions, 2, 'one pending registration survives repeated recovery hints');
    pending.resolve(() => { released++; }); await flush(); await h.clock.tick(50);
    assert.equal(h.sync.listenerError.value, '');
    assert.equal(h.clock.timers.size, 1);
  } finally { h.dispose(); }
  assert.equal(released, 1);
  assert.equal(h.handlers.size, 0);
  assert.equal(h.clock.timers.size, 0);
});
