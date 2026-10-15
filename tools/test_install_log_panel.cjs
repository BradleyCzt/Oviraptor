const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const flush = async () => { for (let i = 0; i < 12; i++) await vue.nextTick(); };
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const row = id => ({ id, stage: 'packages', stream: 'stdout', message: `line ${id}`, time: 'now' });

async function mount(options = {}) {
  let listener, timer, state, releases = 0, subscriptions = 0, changes = 0, reads = [];
  const rows = options.rows || [];
  const exports = {};
  const source = fs.readFileSync(path.join(__dirname, '../src/features/environment/useInstallLogPanel.ts'), 'utf8');
  const listing = async (after, limit) => {
    reads.push(after);
    assert.equal(limit, 300);
    if (options.load) return options.load(after);
    if (options.readFailure) throw Error('private-db-error');
    const ordered = after === undefined ? rows.slice(-limit) : rows.filter(item => item.id > after).slice(0, limit);
    return { rows: structuredClone(ordered), more: after !== undefined && rows.some(item => item.id > (ordered.at(-1)?.id ?? after)),
      olderRows: after === undefined ? rows.length - ordered.length : 0, earliestId: rows[0]?.id ?? 0 };
  };
  new Function('require', 'exports', 'window', ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
  } }).outputText)(name => {
    if (name === 'vue') return vue;
    if (name === './api') return { environmentApi: { listEnvironmentInstallLogs: listing } };
    if (name === '@tauri-apps/api/event') return { listen: async (channel, callback) => {
      assert.equal(channel, 'environment-install-log'); subscriptions++;
      if (options.registration) await options.registration;
      if (options.listenerFailure) throw Error('private-listener-error');
      listener = callback; return () => { releases++; listener = undefined; };
    } };
    throw Error(`Unexpected import ${name}`);
  }, exports, { setInterval(callback, ms) { assert.equal(ms, 15000); timer = callback; return 17; },
    clearInterval(id) { assert.equal(id, 17); timer = undefined; } });
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {},
    parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({ setup() {
    state = exports.useInstallLogPanel(() => { changes++; }); return () => null;
  } });
  app.mount({}); await flush();
  return { state, rows, emit: payload => listener?.({ payload }), retry: () => timer?.(), unmount: () => app.unmount(),
    get releases() { return releases; }, get subscriptions() { return subscriptions; },
    get reads() { return reads; }, get changes() { return changes; }, get timer() { return !!timer; } };
}

test('install journal snapshot, event wakeups and a 300-row display window', async t => {
  const h = await mount({ rows: Array.from({ length: 305 }, (_, n) => row(n + 1)) }); t.after(h.unmount);
  assert.deepEqual(h.state.logs.value.map(item => item.id), Array.from({ length: 300 }, (_, n) => n + 6));
  assert.equal(h.state.evicted.value, 5);
  h.rows.push(row(306), row(307)); h.emit({ id: 307, message: 'must not display event data' }); await flush();
  assert.deepEqual(h.state.logs.value.slice(-2).map(item => item.id), [306, 307]);
  assert.equal(h.state.evicted.value, 7);
  assert.equal(h.state.gapPossible.value, false);
});

test('listener failure retries without claiming a gap, polling recovers committed rows', async t => {
  const options = { rows: [row(1)], listenerFailure: true }, h = await mount(options); t.after(h.unmount);
  assert.equal(h.state.connectionUnavailable.value, true);
  assert.equal(h.state.gapPossible.value, false);
  h.rows.push(row(2)); options.listenerFailure = false; h.retry(); await flush();
  assert.equal(h.state.connectionUnavailable.value, false);
  assert.deepEqual(h.state.logs.value.map(item => item.id), [1, 2]);
  assert.equal(h.subscriptions, 2);
  h.emit({ id: 2 }); await flush();
  assert.deepEqual(h.state.logs.value.map(item => item.id), [1, 2]);
});

test('large reconnect drains pages, deduplicates wakeups and reports actual retention loss', async t => {
  const h = await mount({ rows: [row(1)] }); t.after(h.unmount);
  h.rows.push(...Array.from({ length: 605 }, (_, n) => row(n + 2)));
  h.emit({ id: 606 }); await flush();
  assert.equal(h.state.logs.value.length, 300);
  assert.equal(h.state.logs.value.at(-1).id, 606);
  assert.equal(h.state.evicted.value, 306);
  assert.equal(h.state.gapPossible.value, false);
  h.rows.splice(0, 605); h.rows.push(row(607)); h.retry(); await flush();
  assert.equal(h.state.gapPossible.value, false); // cursor 606 still retained in this view
  h.rows.splice(0); h.rows.push(row(700)); h.retry(); await flush();
  assert.equal(h.state.gapPossible.value, true);
  assert.equal(h.state.logs.value.at(-1).id, 700);
});

test('clear takes a durable high-water mark; a stale read cannot republish old rows', async t => {
  const oldRead = deferred();
  let held = false, snapshots = 0;
  const h = await mount({ rows: [row(1)], load: after => {
    if (after === 1 && !held) { held = true; return oldRead.promise; }
    const rows = after === undefined ? (++snapshots === 1 ? [row(1)] : [row(1), row(2)])
      : [row(2), row(3)].filter(item => item.id > after);
    return { rows, more: false, olderRows: 0, earliestId: 1 };
  } }); t.after(h.unmount);
  h.emit({ id: 2 }); await flush();
  await h.state.clear();
  oldRead.resolve({ rows: [row(2)], more: false, olderRows: 0, earliestId: 1 }); await flush();
  assert.deepEqual(h.state.logs.value.map(item => item.id), [3]);
  assert.equal(h.state.evicted.value, 0);
});

test('read failures and invalid pages never partially publish or leak raw errors', async t => {
  const options = { rows: [row(1)] }, h = await mount(options); t.after(h.unmount);
  options.load = () => ({ rows: [row(2), { ...row(3), stream: 'private' }], more: false, olderRows: 0, earliestId: 1 });
  h.emit({ id: 3 }); await flush();
  assert.deepEqual(h.state.logs.value.map(item => item.id), [1]);
  assert.equal(h.state.connectionUnavailable.value, true);
  assert.equal(h.state.gapPossible.value, false);
  await assert.rejects(h.state.clear(), /无法确认安装日志起点/);
  assert.deepEqual(h.state.logs.value.map(item => item.id), [1]);
  options.load = () => ({ rows: [row(2)], more: false, olderRows: 0, earliestId: 1 });
  h.retry(); await flush();
  assert.deepEqual(h.state.logs.value.map(item => item.id), [1, 2]);
  assert.equal(h.state.connectionUnavailable.value, false);
});

test('slow registration and reads release after unmount without late publication', async () => {
  const registration = deferred(), load = deferred();
  const h = await mount({ registration: registration.promise, load: () => load.promise });
  assert.equal(h.timer, true);
  h.retry(); assert.equal(h.subscriptions, 1);
  h.unmount(); registration.resolve(); load.resolve({ rows: [row(1)], more: false, olderRows: 0, earliestId: 1 });
  await flush();
  assert.equal(h.releases, 1);
  assert.equal(h.timer, false);
  assert.equal(h.state.logs.value.length, 0);
});
