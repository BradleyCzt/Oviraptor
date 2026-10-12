// §Q0 — the exposure page's read state, driven by the real composable with a mocked IPC.
// These tests are about honesty under races: a late answer from an older project or an
// older run must never overwrite what is on screen, a failed or malformed read keeps the
// last trustworthy snapshot, and nothing may start a collection by itself.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');

const flush = async () => { for (let i = 0; i < 20; i++) await vue.nextTick(); };
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const loadComposable = () => {
  const exports = {};
  const contracts = {};
  new Function('exports', transpile(
    fs.readFileSync(path.join(__dirname, '../src/features/assets/exposure/contracts.ts'), 'utf8'),
  ))(contracts);
  new Function('require', 'exports', transpile(
    fs.readFileSync(path.join(__dirname, '../src/features/assets/exposure/useExposureSurface.ts'), 'utf8'),
  ))(name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts;
    throw Error(`Unexpected exposure import ${name}`);
  }, exports);
  return { composable: exports, contracts };
};

const summary = (projectId, over = {}) => ({
  projectId, projectActive: true, eligibleAssetsTotal: 250, eligibleAssetsPlanned: 200,
  assetsBeyondPlan: 50, findingsTotal: 201, findingsNew: 9, findingsReviewing: 3,
  findingsConfirmed: 2, findingsDismissed: 1, highRiskOpen: 7,
  knownLimits: [{ key: 'passive_assets', label: '被动采集的资产数上限', cap: 200, effect: '超出的准入资产本轮不会被采集' }],
  ...over,
});
const finding = (id, projectId) => ({ id, projectId, title: `线索 ${id}`, severity: 'high', confidence: 80, status: 'new', sourceUrl: `https://p.invalid/${id}`, evidenceExcerpt: '', assetLabel: '', sourceType: 'passive_web', firstSeenAt: 't', lastSeenAt: 't' });
const run = (id, projectId, status = 'completed') => ({ id, projectId, status, eligibleAssets: 2, scannedAssets: 2, fetchedResources: 4, findings: 1, stage: 'completed', currentSource: '', error: '', createdAt: 't', startedAt: 't', completedAt: 't' });
const source = (runId, key = 'passive_web') => ({ runId, sourceKey: key, status: 'completed', itemCount: 3, error: '', completedAt: 't' });
const page = (items, total, { hasMore = false, nextCursor = '', pageSize = items.length || 50 } = {}) => ({ items, total, hasMore, nextCursor, pageSize });

/// A scripted IPC whose responses can be held, delayed and resolved out of order.
function harness(t, read = {}) {
  const calls = [];
  const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
  const api = {
    exposureScopeSummary: projectId => { calls.push(['summary', projectId]); return (read.summary ?? (async () => summary(projectId)))(projectId); },
    pageExposureFindings: input => { calls.push(['findings', input.projectId, input.cursor]); return (read.findings ?? (async () => page([finding(5, input.projectId)], 1)))(input); },
    pageExposureRuns: (projectId, pageSize, cursor) => { calls.push(['runs', projectId]); return (read.runs ?? (async () => page([run(9, projectId, 'running')], 1)))(projectId, pageSize, cursor); },
    listExposureSourceResultsForProject: (runId, expectedProjectId) => {
      calls.push(['sources', runId, expectedProjectId]);
      return (read.sources ?? (async () => [source(runId)]))(runId, expectedProjectId);
    },
    exposureSummary: projectId => { calls.push(['legacy-summary', projectId]); return (read.legacySummary ?? (async () => { throw Error('unknown command: exposure_summary'); }))(projectId); },
    listExposureFindings: (projectId, status, searchTerm) => { calls.push(['legacy-findings', projectId, status]); return (read.legacyFindings ?? (async () => { throw Error('unknown command: list_exposure_findings'); }))(projectId, status, searchTerm); },
    listExposureRuns: projectId => { calls.push(['legacy-runs', projectId]); return (read.legacyRuns ?? (async () => { throw Error('unknown command: list_exposure_runs'); }))(projectId); },
    listExposureSourceResults: (runId, expectedProjectId) => { calls.push(['legacy-sources', runId, expectedProjectId]); return (read.legacySources ?? (async () => { throw Error('unknown command: list_exposure_source_results'); }))(runId); },
    listAssets: query => { calls.push(['assets', query.page, query.pageSize]); return (read.listAssets ?? (async () => ({ items: [], total: 0, page: query.page, pageSize: query.pageSize, summary: {} })))(query); },
    startExposureScan: projectId => { calls.push(['start', projectId]); return (read.startExposureScan ?? (async () => 77))(projectId); },
    cancelExposureScan: (projectId, runId) => { calls.push(['cancel', projectId, runId]); return Promise.resolve(); },
    reviewExposureFinding: (id, status) => { calls.push(['review', id, status]); return Promise.resolve(); },
    saveExposureFinding: input => { calls.push(['save', input.title]); return Promise.resolve(1); },
    exportExposureFindings: projectId => { calls.push(['export', projectId]); return Promise.resolve({ path: '/tmp/x.csv', rows: 3 }); },
  };
  const events = [];
  const selection = vue.reactive({ project: 10, active: true });
  const { composable } = loadComposable();
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureSurface({
        projectId: () => selection.project,
        active: () => selection.active,
        api,
        notify: (kind, text) => events.push([kind, text]),
      });
      return () => null;
    },
  });
  app.mount({});
  let closed = false;
  const unmount = () => { if (closed) return; closed = true; app.unmount(); };
  // A failed assertion must not leave the poll interval alive in the test process.
  if (t && typeof t.after === 'function') t.after(unmount);
  return { state, calls, events, selection, api, deferred, flush, unmount };
}

test('exposure project switch keeps the newest read when responses land in reverse order', async (t) => {
  const first = deferred(), second = deferred();
  let count = 0;
  const h = harness(t, {
    summary: () => {
      count += 1;
      return count === 1 ? first.promise : second.promise;
    },
  });
  await h.flush();
  h.selection.project = 20;
  await h.flush();
  second.resolve(summary(20, { findingsTotal: 5, eligibleAssetsTotal: 5, eligibleAssetsPlanned: 5, assetsBeyondPlan: 0 }));
  await h.flush();
  first.resolve(summary(10, { findingsTotal: 999 }));
  await h.flush();
  assert.equal(h.state.summary.value.projectId, 20, '旧项目的汇总不能覆盖新项目');
  assert.equal(h.state.summary.value.findingsTotal, 5);
  assert.ok(!h.state.stale.value, '晚到的旧响应不该把新项目标成陈旧');
  h.unmount();
});

test('exposure reads are scoped per project for findings, runs and sources', async (t) => {
  const h = harness(t);
  await h.flush();
  const summaryCalls = h.calls.filter(row => row[0] === 'summary').map(row => row[1]);
  assert.deepEqual(summaryCalls, [10]);
  assert.deepEqual(h.calls.find(row => row[0] === 'sources'), ['sources', 9, 10], '数据源必须带 expectedProjectId');
  h.selection.project = 30;
  await h.flush();
  assert.ok(h.calls.some(row => row[0] === 'findings' && row[1] === 30));
  assert.equal(h.state.selectedRunId.value, 9, '新项目里同 id 的任务由后端复验归属');
  h.unmount();
});

test('exposure failed read keeps the last snapshot and says it is stale', async (t) => {
  let fail = false;
  const h = harness(t, {
    summary: async (t) => { if (fail) throw Error('[exposure_query_failed] 临时库忙'); return summary(10); },
    findings: async () => page([finding(1, 10), finding(2, 10)], 2),
  });
  await h.flush();
  assert.equal(h.state.findings.value.length, 2);
  assert.equal(h.state.stale.value, false);
  fail = true;
  await h.state.load();
  assert.equal(h.state.stale.value, true);
  assert.deepEqual(h.state.findings.value.map(row => row.id), [1, 2], '失败不能清空已有结果');
  assert.equal(h.state.summary.value.eligibleAssetsTotal, 250);
  assert.ok(h.events.some(([kind, text]) => kind === 'error' && text.includes('上次可信快照')));
  assert.ok(!h.events.some(([, text]) => text.includes('临时库忙')), '原始错误文本不外泄');
  assert.ok(!h.events.some(([, text]) => text.includes('exposure_query_failed')), '错误码不外泄');
  fail = false;
  await h.state.load();
  assert.equal(h.state.stale.value, false);
  h.unmount();
});

test('exposure malformed pages never replace trustworthy rows', async (t) => {
  const cases = [
    null,
    { items: 'nope', total: 3 },
    { items: [{ id: 1 }], total: 0 },
    { items: [{ id: 1 }, { id: 1 }], total: 2, hasMore: false, nextCursor: '' },
    { items: [{ id: 1 }], total: 1, hasMore: false, nextCursor: 'opaque' },
    { items: [{ id: 1 }], total: 1, hasMore: true, nextCursor: '', pageSize: 0 },
  ];
  for (const broken of cases) {
    const h = harness(t, { findings: async () => broken });
    await h.flush();
    assert.equal(h.state.stale.value, true, `${JSON.stringify(broken)} 必须被拒绝`);
    assert.deepEqual(h.state.findings.value, [], '坏响应不得写入半截数据');
    h.unmount();
  }
  const good = harness(t);
  await good.flush();
  assert.equal(good.state.stale.value, false);
  good.unmount();
});

test('exposure sources follow the selected run and drop superseded answers', async (t) => {
  const slow = deferred();
  let calls = 0;
  const h = harness(t, {
    sources: () => { calls += 1; return calls === 1 ? slow.promise : Promise.resolve([source(8, 'web_archive')]); },
    runs: async () => page([run(9, 10), run(8, 10)], 2),
  });
  await h.flush();
  assert.equal(h.state.selectedRunId.value, 9);
  const switcher = h.state.showSourcesFor(8);
  await h.flush();
  assert.deepEqual(h.state.sources.value.map(row => row.sourceKey), ['web_archive'], '新选择必须立即生效');
  slow.resolve([source(9, 'certificate_transparency')]);
  await switcher;
  await h.flush();
  assert.deepEqual(h.state.sources.value.map(row => row.sourceKey), ['web_archive'], '旧任务的晚到结果不能覆盖所选任务');
  h.unmount();
});

test('exposure bound assets paginate instead of assuming one page is everything', async (t) => {
  const h = harness(t, {
    listAssets: async query => ({
      items: Array.from({ length: query.page === 3 ? 1 : 100 }, (_, index) => ({ id: (query.page - 1) * 100 + index + 1 })),
      total: 201, page: query.page, pageSize: query.pageSize, summary: {},
    }),
  });
  await h.flush();
  const pages = h.calls.filter(row => row[0] === 'assets').map(row => row[1]);
  assert.deepEqual(pages, [1, 2, 3], '必须一页一页读到全量：' + JSON.stringify(pages));
  assert.equal(h.state.boundAssets.value.length, 201);
  assert.equal(h.state.boundAssetsTruncated.value, false);
  h.unmount();
});

test('exposure bound assets report truncation instead of claiming completeness', async (t) => {
  const h = harness(t, {
    listAssets: async query => ({ items: [{ id: query.page }], total: 5_000, page: query.page, pageSize: query.pageSize, summary: {} }),
  });
  await h.flush();
  assert.equal(h.state.boundAssets.value.length, 20, '最多翻 20 页');
  assert.equal(h.state.boundAssetsTruncated.value, true, '读不完必须标记为截断');
  assert.equal(h.state.boundAssetsTotal.value, 5_000);
  h.unmount();
});

test('exposure polling only follows an active run and stops when hidden or unmounted', async (t) => {
  const h = harness(t, { runs: async () => page([run(9, 10, 'running')], 1) });
  await h.flush();
  const before = h.calls.filter(row => row[0] === 'summary').length;
  h.state.tick();
  await h.flush();
  assert.equal(h.calls.filter(row => row[0] === 'summary').length, before + 1, '有活动任务才轮询');
  h.selection.active = false;
  h.state.tick();
  await h.flush();
  assert.equal(h.calls.filter(row => row[0] === 'summary').length, before + 1, '页面不在前台时不轮询');
  h.selection.active = true;
  global.document = { hidden: true };
  h.state.tick();
  await h.flush();
  assert.equal(h.calls.filter(row => row[0] === 'summary').length, before + 1, '标签页隐藏时不轮询');
  delete global.document;
  const overlap = h.deferred();
  const original = h.api.pageExposureRuns;
  h.api.pageExposureRuns = projectId => { h.api.pageExposureRuns = original; return overlap.promise; };
  h.state.tick();
  h.state.tick();
  h.state.tick();
  assert.equal(h.calls.filter(row => row[0] === 'summary').length, before + 2, '重叠轮询必须合并成一次在途读取');
  overlap.resolve(page([run(9, 10, 'running')], 1));
  await h.flush();
  h.unmount();
  const afterUnmount = h.calls.filter(row => row[0] === 'summary').length;
  h.state.tick();
  await h.flush();
  assert.equal(h.calls.filter(row => row[0] === 'summary').length, afterUnmount, '卸载后回调不得再读取');
});

test('exposure page never starts a collection by itself', async (t) => {
  const h = harness(t, { runs: async () => page([run(9, 10)], 1) });
  await h.flush();
  h.state.tick();
  await h.flush();
  await h.state.loadMore();
  await h.state.exportAll();
  await h.state.save({ projectId: 10, category: 'document', title: 'x', sourceType: 'manual', sourceUrl: 'https://p.invalid/x', evidenceExcerpt: '', severity: 'low', confidence: 50 });
  assert.deepEqual(h.calls.filter(row => row[0] === 'start'), [], '读取、轮询、导出与录入都不能启动采集');

  // Two rapid clicks on the explicit button are one request: the second lands while the
  // first is still in flight.
  const pending = deferred();
  h.api.startExposureScan = projectId => {
    h.calls.push(['start', projectId]);
    return pending.promise;
  };
  const first = h.state.startScan();
  const second = h.state.startScan();
  pending.resolve(77);
  await Promise.all([first, second]);
  await h.flush();
  assert.deepEqual(h.calls.filter(row => row[0] === 'start'), [['start', 10]], '连点只能启动一次');

  // A run that is already active also refuses a second collection.
  const busy = harness(t, { runs: async () => page([run(9, 10, 'running')], 1) });
  await busy.flush();
  await busy.state.startScan();
  assert.deepEqual(busy.calls.filter(row => row[0] === 'start'), []);
  busy.unmount();
  h.unmount();
});

test('exposure load more appends the next cursor page and keeps the filtered total', async (t) => {
  let cursor = '';
  const h = harness(t, {
    findings: async input => {
      cursor = input.cursor;
      return cursor === ''
        ? page([finding(1, 10), finding(2, 10)], 3, { hasMore: true, nextCursor: 'C2' })
        : page([finding(3, 10)], 3);
    },
  });
  await h.flush();
  assert.equal(h.state.findings.value.length, 2);
  assert.equal(h.state.hasMore.value, true);
  await h.state.loadMore();
  assert.deepEqual(h.state.findings.value.map(row => row.id), [1, 2, 3]);
  assert.equal(h.state.findingsTotal.value, 3);
  assert.equal(h.state.hasMore.value, false);
  assert.ok(cursor === 'C2');
  await h.state.loadMore();
  assert.equal(h.state.findings.value.length, 3, '没有更多时不再请求');
  h.unmount();
});

test('exposure falls back to the legacy window only for an unregistered command', async (t) => {
  const unregistered = message => async () => { throw Error(message); };
  const h = harness(t, {
    summary: unregistered('command exposure_scope_summary not found'),
    findings: unregistered('unknown command: page_exposure_findings'),
    runs: unregistered('Command page_exposure_runs not found'),
    legacySummary: async () => ({ eligibleAssets: 250, total: 500, newCount: 12, reviewing: 3, confirmed: 2, dismissed: 1, highRisk: 9 }),
    legacyFindings: async () => [finding(1, 10), finding(2, 10)],
    legacyRuns: async () => [run(4, 10), run(3, 10)],
  });
  await h.flush();
  assert.equal(h.state.pagedReadsAvailable.value, false, '未注册的分页命令必须退回旧接口');
  assert.deepEqual(h.state.findings.value.map(row => row.id), [1, 2], '旧接口仍要能给出当前结果');
  // G1-0: the legacy endpoint has no project cap; G0 invented 200 / 50 here.
  assert.equal(h.state.summary.value.eligibleAssetsPlanned, 250);
  assert.equal(h.state.summary.value.assetsBeyondPlan, 0);
  assert.match(h.state.summary.value.limitsError, /未核实/);
  assert.match(h.state.beyondPlanNotice.value, /采集上限不可用/);
  assert.match(h.state.windowNotice.value, /最多前 500/);
  assert.equal(h.state.stale.value, false, '退回旧接口不是失败');
  assert.equal(h.state.hasMore.value, false, '旧接口没有游标，不能假装还能翻页');

  // A real storage failure must not be papered over by the legacy path.
  const busy = harness(t, {
    summary: async () => { throw Error('[exposure_query_failed] 数据库忙'); },
    legacyFindings: async () => { busy.calls.push(['legacy']); return []; },
  });
  await busy.state.load();
  assert.equal(busy.state.pagedReadsAvailable.value, true);
  assert.equal(busy.calls.some(row => row[0] === 'legacy'), false, '真失败不得改走旧接口');
  assert.equal(busy.state.stale.value, true);
  busy.unmount();
  h.unmount();
});

test('exposure scope wording states the planned cap and the uncollected remainder', async (t) => {
  const h = harness(t);
  await h.flush();
  assert.match(h.state.scopeNotice.value, /本轮计划 200 个准入资产/);
  assert.match(h.state.scopeNotice.value, /另有 50 个本轮未采集/);
  assert.match(h.state.beyondPlanNotice.value, /准入资产 250 个/);
  assert.equal(h.state.knownLimits.value[0].label, '被动采集的资产数上限');
  const archived = harness(t, { summary: async () => summary(10, { projectActive: false, eligibleAssetsTotal: 0, eligibleAssetsPlanned: 0, assetsBeyondPlan: 0 }) });
  await archived.flush();
  assert.match(archived.state.scopeNotice.value, /没有采集范围/);
  archived.unmount();
  h.unmount();
});

// G0/P1-4 — the page composable offers no unversioned review write any more: every review
// goes through the scoped panel (projectId + expectedRevision + operator + reason). Even if
// the legacy IPC is still present on the api object, nothing on the surface reaches it.
test('the exposure surface never writes a review through the legacy unversioned entry', async (t) => {
  const h = harness(t, { runs: async () => page([run(9, 10)], 1) });
  await h.flush();
  assert.equal(typeof h.state.review, 'undefined', '页面组合函数不再暴露无版本复核写入');
  h.state.tick();
  await h.flush();
  await h.state.loadMore();
  await h.state.exportAll();
  await h.state.save({ projectId: 10, category: 'document', title: 'x', sourceType: 'manual', sourceUrl: 'https://p.invalid/x', evidenceExcerpt: '', severity: 'low', confidence: 50 });
  assert.deepEqual(h.calls.filter(row => row[0] === 'review'), [], '旧 review_exposure_finding 一次都不能被调用');
});

test('exposure cancel names the project on screen so the backend can refuse another project\'s run', async (t) => {
  const h = harness(t);
  await h.flush();
  await h.state.cancelScan();
  assert.deepEqual(h.calls.find(row => row[0] === 'cancel'), ['cancel', 10, 9], '取消必须带当前项目与任务 id');
  h.unmount();
});

// ───────────── FIX3（2026-10-09 17:29） ─────────────

test('FIX3-R3 the legacy source fallback also carries the current project for backend checking', async (t) => {
  const unregistered = message => async () => { throw Error(message); };
  const h = harness(t, {
    summary: unregistered('command exposure_scope_summary not found'),
    findings: unregistered('unknown command: page_exposure_findings'),
    runs: unregistered('Command page_exposure_runs not found'),
    legacySummary: async () => ({ eligibleAssets: 1, total: 1, newCount: 1, reviewing: 0, confirmed: 0, dismissed: 0, highRisk: 0 }),
    legacyFindings: async () => [finding(1, 10)],
    legacyRuns: async () => [run(4, 10)],
    legacySources: async runId => [source(runId)],
  });
  await h.flush();
  assert.equal(h.state.pagedReadsAvailable.value, false);
  await h.state.showSourcesFor(4);
  await h.flush();
  const legacy = h.calls.filter(row => row[0] === 'legacy-sources');
  assert.ok(legacy.length >= 1, JSON.stringify(h.calls));
  assert.ok(legacy.every(row => row[2] === 10), `旧来源接口必须带 expectedProjectId：${JSON.stringify(legacy)}`);
  h.unmount();
});
