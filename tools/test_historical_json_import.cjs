// Actual parent + child setup/render/lifecycle. Only native picker, IPC and icons
// are substituted. No browser or real Tauri window is claimed by this harness.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const { loadSentinelApi } = require('./sentinel_api_harness.cjs');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const transpile = text => ts.transpileModule(text, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
function component(relative, resolve, capture = () => {}) {
  const filename = path.join(__dirname, '../src/features/sentinel/components', relative);
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id: relative });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: relative,
    compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, []);
  const result = {}, render = {};
  new Function('require', 'exports', transpile(script.content))(resolve, result);
  new Function('require', 'exports', transpile(template.code))(require, render);
  const original = result.default.setup;
  return { ...result.default, render: render.render, setup(props, context) {
    const bindings = original(props, context); capture(bindings); return bindings;
  } };
}
const node = (type, text = '') => ({ type, text, props: {}, children: [], parent: null });
function remove(child) {
  if (child.parent) { const rows = child.parent.children, index = rows.indexOf(child); if (index >= 0) rows.splice(index, 1); }
  child.parent = null;
}
const renderer = vue.createRenderer({
  createElement: type => Object.assign(node(type), { addEventListener() {}, removeEventListener() {}, setAttribute() {}, removeAttribute() {} }),
  createText: text => node('#text', text), createComment: text => node('#comment', text),
  insert(child, parent, anchor) { remove(child); const index = anchor ? parent.children.indexOf(anchor) : -1;
    parent.children.splice(index < 0 ? parent.children.length : index, 0, child); child.parent = parent; },
  remove, setText: (n, text) => { n.text = text; }, setElementText(n, text) { n.children = []; n.text = text; },
  patchProp(n, key, _, value) { n.props[key] = value; }, parentNode: n => n.parent,
  nextSibling: n => n.parent?.children[n.parent.children.indexOf(n) + 1] ?? null,
});
const text = n => (n.type === '#comment' ? '' : n.text) + n.children.map(text).join(' ');
const find = (n, predicate) => predicate(n) ? n : n.children.map(child => find(child, predicate)).find(Boolean);
const flush = async () => { for (let i = 0; i < 18; i++) await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const presentation = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../src/features/sentinel/presentation.ts'), 'utf8')))(require, presentation);
const historicalContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/results/historicalPreviewContract.ts'), 'utf8')))(historicalContract);
const history = (id) => ({ rowId: id, bundleId: `sha256:${'a'.repeat(64)}`, scanId: `history-${id}`, attemptNumber: 1,
  title: `Imported ${id}`, status: 'imported', findingCandidates: 1, coverageRecords: 0, importedAt: '2026-09-27' });
const previewRow = (id, extra = {}) => ({ membershipId: id, scanId: 'history-1', attemptNumber: 1,
  kind: 'finding_candidate', title: `Preview ${id}`, severity: '', target: '', producer: 'historical fixture',
  reviewState: 'unreviewed', readOnly: true, executionEligible: false, ...extra });
async function mount({ picker = async () => '/chosen/history.json', importFile = async () => 3,
  list = async () => [], preview = async () => [] } = {}) {
  const calls = [], pickerCalls = []; let parentBindings, childBindings;
  const api = { importSentinelProject: async path => { calls.push(['import', path]); return importFile(path); },
    listHistoricalImportRuns: async cursor => { calls.push(['list', cursor]); return list(cursor); },
    listHistoricalBundlePreviews: async (...args) => { calls.push(['preview', ...args]); return preview(...args); } };
  const child = component('HistoricalJsonImport.vue', name => {
    if (name === 'vue') return vue;
    if (name === '@tauri-apps/plugin-dialog') return { open: options => { pickerCalls.push(options); return picker(options); } };
    if (name === '../api') return { sentinelApi: api };
    throw new Error(`Unexpected child import ${name}`);
  }, bindings => { childBindings = bindings; });
  const parent = component('SentinelTaskCenter.vue', name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
    if (name === '../results/taskPageContract') {
      const contract = {};
      new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
        '../src/features/sentinel/results/taskPageContract.ts'), 'utf8')))(contract);
      return contract;
    }
    if (['../execution/useTaskExecutionDetails', '../results/useHistoricalImportLedger'].includes(name)) {
      const details = {};
      const source = fs.readFileSync(path.join(__dirname, '../src/features/sentinel', `${name.slice(3)}.ts`), 'utf8');
      new Function('require', 'exports', transpile(source))((dependency) => {
        if (dependency === 'vue') return vue;
        if (dependency === '../api') return { sentinelApi: api };
        if (dependency === './useLiveRunnerLog') return { useLiveRunnerLog() {} };
        if (['../results/historicalPreviewContract', './historicalPreviewContract'].includes(dependency)) return historicalContract;
        throw new Error(`Unexpected business module import ${dependency}`);
      }, details);
      return details;
    }
    if (name === '../presentation') return presentation;
    if (name === '../../../i18n') return { useI18n: () => ({ tr: zh => zh }) };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name === './HistoricalJsonImport.vue') return { default: child };
    if (name.endsWith('.vue')) return { default: () => null };
    throw new Error(`Unexpected parent import ${name}`);
  }, bindings => { parentBindings = bindings; });
  const props = { scans: [], hasMore: false, loadingMore: false, previewTargets: [], attentionCount: 0,
    totalTokens: 0, totalRequests: 0, tokenScopeLabel: '全部部署', zeroYieldCount: 0, zeroYieldTokens: 0, cacheHitRate: 0, controlBusy: '', highValueCount: () => 0 };
  // v-model on the parent's real search input checks focus during render updates.
  const previousDocument = global.document; global.document = { activeElement: null };
  const root = node('root'), app = renderer.createApp(parent, props); app.mount(root);
  parentBindings.view.value = 'history'; await flush();
  return { calls, pickerCalls, p: parentBindings, get c() { return childBindings; }, root,
    button: () => find(root, n => n.type === 'button' && text(n).includes('导入历史 JSON')),
    renderText: () => text(root), unmount: () => { app.unmount(); global.document = previousDocument; } };
}

test('historical import IPC accepts only the chosen path, not project or task authority', async () => {
  const calls = [];
  const exports = { sentinelApi: loadSentinelApi(async (...args) => { calls.push(args); return 2; }) };
  await exports.sentinelApi.importSentinelProject('/chosen/archive.json');
  await exports.sentinelApi.importSentinelResults('{"format":"oviraptor-sentinel-v1"}');
  assert.deepEqual(calls, [['import_sentinel_project', { path: '/chosen/archive.json' }],
    ['import_sentinel_results', { content: '{"format":"oviraptor-sentinel-v1"}' }]]);
});

test('real task center button imports once and refreshes only the historical list', async t => {
  const pending = deferred(); let lists = 0;
  const h = await mount({ importFile: () => pending.promise, list: async () => ++lists === 1 ? [] : [history(2)] });
  t.after(h.unmount); const click = h.button().props.onClick(); await flush();
  assert.equal(h.button().props.disabled, true);
  await h.c.importJson(); assert.equal(h.calls.filter(c => c[0] === 'import').length, 1);
  pending.resolve(3); await click; await flush();
  assert.deepEqual(h.calls, [['list', undefined], ['import', '/chosen/history.json'], ['list', undefined]]);
  assert.equal(h.pickerCalls[0].multiple, false);
  assert.match(h.renderText(), /Imported 2/); assert.match(h.renderText(), /没有创建或恢复扫描任务/);
  assert.match(h.renderText(), /未审核/); assert.equal(h.button().props.disabled, false);
});

test('cancelled picker never imports, and failures never display raw paths or secrets', async t => {
  for (const picker of [async () => null, async () => { throw Error('Bearer picker-secret'); }]) {
    const h = await mount({ picker }); await h.button().props.onClick(); await flush();
    assert.equal(h.calls.some(c => c[0] === 'import'), false); assert.doesNotMatch(h.renderText(), /picker-secret/);
    assert.equal(h.c.busy.value, false); h.unmount();
  }
  const h = await mount({ importFile: async () => { throw Error('/private/token-secret.json'); } });t.after(h.unmount);
  await h.button().props.onClick(); await flush();
  assert.match(h.renderText(), /导入未完成/); assert.doesNotMatch(h.renderText(), /token-secret/);
  assert.equal(h.calls.filter(c => c[0] === 'list').length, 1);
});

test('import completion supersedes in-flight list and preview responses', async t => {
  const oldList = deferred(), oldPreview = deferred(); let lists = 0;
  const h = await mount({ list: () => ++lists === 1 ? oldList.promise : Promise.resolve([history(3)]), preview: () => oldPreview.promise });
  t.after(h.unmount); const viewing = h.p.selectImportedRun(history(1));
  await h.button().props.onClick(); await flush();
  oldList.resolve([history(1)]); oldPreview.resolve([{ title: 'obsolete', membershipId: 1 }]); await viewing; await flush();
  assert.deepEqual(h.p.importedRuns.value.map(r => r.rowId), [3]);
  assert.equal(h.p.importedSelected.value, undefined); assert.deepEqual(h.p.importedPreviews.value, []);
  assert.doesNotMatch(h.renderText(), /obsolete|Imported 1/);
});

test('unmount while choosing a file prevents import; late commit does not refresh a destroyed view', async () => {
  const picker = deferred(); const h = await mount({ picker: () => picker.promise });
  const click = h.button().props.onClick(); h.unmount(); picker.resolve('/chosen/late.json'); await click;
  assert.equal(h.calls.some(c => c[0] === 'import'), false);
  const pending = deferred(); const second = await mount({ importFile: () => pending.promise });
  const importing = second.button().props.onClick(); await flush(); second.unmount(); pending.resolve(3); await importing;
  assert.equal(second.calls.filter(c => c[0] === 'list').length, 1);
});

test('invalid import receipts fail closed without refreshing or claiming success', async t => {
  const h = await mount({ importFile: async () => -1 });t.after(h.unmount);
  await h.button().props.onClick(); await flush(); assert.match(h.renderText(), /导入未完成/);
  assert.equal(h.calls.filter(c => c[0] === 'list').length, 1);
});

test('closing a pending historical preview clears loading and fences its response', async t => {
  const pending = deferred();
  const h = await mount({ list: async () => [history(1)], preview: () => pending.promise }); t.after(h.unmount);
  const reading = h.p.selectImportedRun(history(1));
  assert.equal(h.p.importedPreviewLoading.value, true);
  await h.p.selectImportedRun(history(1));
  assert.equal(h.p.importedSelected.value, undefined);
  assert.equal(h.p.importedPreviewLoading.value, false);
  pending.resolve([previewRow(1)]); await reading; await flush();
  assert.deepEqual(h.p.importedPreviews.value, []); assert.doesNotMatch(h.renderText(), /Preview 1/);
});

test('historical ledger read failures never expose raw database paths or credentials', async t => {
  const failure = () => Promise.reject(Error('/private/history.db Bearer db-secret'));
  const listing = await mount({ list: failure });
  try {
    assert.match(listing.renderText(), /历史导入记录暂时不可读取/);
    assert.doesNotMatch(listing.renderText(), /private|db-secret/);
  } finally { listing.unmount(); }
  const h = await mount({ list: async () => [history(1)], preview: failure }); t.after(h.unmount);
  await h.p.selectImportedRun(history(1)); await flush();
  assert.match(h.renderText(), /历史产物预览暂时不可读取/);
  assert.doesNotMatch(h.renderText(), /private|db-secret/);
});

test('historical preview rejects malformed or contradictory authority instead of relabelling it read-only', async () => {
  const invalidPages = [{ reviewState: 'confirmed' }, { readOnly: false }, { executionEligible: true }]
    .map(invalid => [previewRow(1, invalid)]);
  invalidPages.push(null, {}, [null], Array.from({ length: 301 }, (_, i) => previewRow(i + 1)));
  for (const invalidPage of invalidPages) {
    const h = await mount({ list: async () => [history(1)], preview: async () => invalidPage });
    try {
      await h.p.selectImportedRun(history(1)); await flush();
      assert.deepEqual(h.p.importedPreviews.value, []);
      assert.match(h.renderText(), /历史产物预览暂时不可读取/);
      assert.doesNotMatch(h.renderText(), /Preview 1/);
    } finally { h.unmount(); }
  }
});

test('unmount isolates late historical ledger failures and prevents further reads', async () => {
  const list = deferred(), preview = deferred();
  const h = await mount({ list: () => list.promise, preview: () => preview.promise });
  const reading = h.p.selectImportedRun(history(1)); h.unmount();
  list.reject(Error('old list error')); preview.reject(Error('old preview error'));
  await reading; await flush();
  assert.deepEqual(h.p.importedRuns.value, []); assert.deepEqual(h.p.importedPreviews.value, []);
  assert.equal(h.p.importedError.value, '');
  await h.p.loadImportedRuns(false, true); await h.p.selectImportedRun(history(2));
  assert.deepEqual(h.calls, [['list', undefined], ['preview', history(1).bundleId, 1]]);
});

test('historical preview binds bundle and projection IDs and keeps multiple attempts read-only', async t => {
  const first = deferred(); const secondRun = { ...history(2), bundleId: `sha256:${'b'.repeat(64)}`, attemptNumber: 2 };
  const rows = [previewRow(3, { scanId: secondRun.scanId }), previewRow(4, { scanId: secondRun.scanId, attemptNumber: 2 })];
  const h = await mount({ list: async () => [history(1), secondRun],
    preview: (_, projectionId) => projectionId === 1 ? first.promise : Promise.resolve(rows) }); t.after(h.unmount);
  const old = h.p.selectImportedRun(history(1)); await h.p.selectImportedRun(secondRun); await flush();
  first.resolve([previewRow(1)]); await old; await flush();
  assert.deepEqual(h.calls.filter(call => call[0] === 'preview'), [
    ['preview', history(1).bundleId, 1], ['preview', secondRun.bundleId, 2],
  ]);
  assert.deepEqual(h.p.importedPreviews.value.map(row => row.membershipId), [3, 4]);
  assert.match(h.renderText(), /Preview 3/); assert.match(h.renderText(), /Preview 4/);
  assert.doesNotMatch(h.renderText(), /Preview 1/);
  assert.match(h.renderText(), /未审核 \/ 只读/);
});

test('historical ledger pagination uses the last visible projection and preserves rows on failure', async t => {
  let requests = 0;
  const rows = Array.from({ length: 51 }, (_, i) => history(101 - i));
  const h = await mount({ list: async () => {
    if (++requests === 1) return rows;
    if (requests === 2) throw Error('temporary failure');
    return [rows[50]];
  } }); t.after(h.unmount);
  assert.equal(h.p.importedRuns.value.length, 50); assert.equal(h.p.importedHasMore.value, true);
  await h.p.loadImportedRuns(true);
  assert.equal(h.p.importedRuns.value.length, 50); assert.equal(h.p.importedHasMore.value, true);
  await h.p.loadImportedRuns(true);
  assert.deepEqual(h.calls, [['list', undefined], ['list', 52], ['list', 52]]);
  assert.equal(h.p.importedRuns.value.length, 51); assert.equal(h.p.importedHasMore.value, false);
  assert.equal(h.p.importedError.value, '');
});

test('historical ledger rejects a preview returned for a different scan scope', async t => {
  const h = await mount({ list: async () => [history(1)],
    preview: async () => [previewRow(1, { scanId: 'history-2' })] }); t.after(h.unmount);
  await h.p.selectImportedRun(history(1)); await flush();
  assert.deepEqual(h.p.importedPreviews.value, []);
  assert.match(h.renderText(), /历史产物预览暂时不可读取/);
  assert.doesNotMatch(h.renderText(), /Preview 1/);
});
