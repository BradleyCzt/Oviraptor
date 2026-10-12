// Exercise the real component setup/render and API; only IPC is substituted.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const { loadSentinelApi } = require('./sentinel_api_harness.cjs');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const transpile = text => ts.transpileModule(text, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const filename = path.join(__dirname, '../src/features/sentinel/components/SourceReviewEvidence.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'source-findings-test' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'source-findings-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const templateExports = {};
new Function('require', 'exports', transpile(template.code))(require, templateExports);
const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {}, parentNode: () => null, nextSibling: () => null });
const flush = async () => { for (let i = 0; i < 12; i++) await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const finding = (id = 'd1', extra = {}) => ({ id, sourceDecisionId: id, scanId: 'A', attemptNumber: 1,
  rootRunId: 'root', reviewerRunId: 'reviewer', materialDigest: 'digest', title: 'Verified source issue', path: 'app.py', line: 3,
  cwe: 'CWE-1', severity: 'high', reviewState: 'confirmed', rationale: 'Independent evidence', confidence: 0.9,
  evidenceRefs: ['receipt:one'], ...extra });
const page = (extra = {}) => ({ schemaVersion: 1, scanId: 'A', attemptNumber: 1, status: 'audited', rootRunId: 'root',
  materialDigest: 'digest', counts: { decisions: 3, confirmed: 1, rejected: 1, insufficient: 1 },
  independentCandidateReviewCompleted: true, independentReviewCompleted: false, offset: 0, nextOffset: null,
  findings: [finding()], ...extra });
const unavailable = status => page({ status, counts: null, materialDigest: null, findings: [], independentCandidateReviewCompleted: false });
const coverage = (extra = {}) => ({ schemaVersion: 1, status: 'prepared_not_reviewed', scanId: 'A', attemptNumber: 1,
  rootRunId: 'root', materialDigest: 'a'.repeat(64), executionEligible: false, independentReviewCompleted: false,
  requestedScope: 'diff', effectiveScope: 'diff', selectedFileCount: 1, changedPathsWithoutContentCount: 2,
  outstandingGaps: ['source_coverage_review', 'dependency_graph_incomplete'], ...extra });
const reviewedCoverage = (sufficient = false) => {
  const gaps = sufficient ? [] : ['dependency_graph_incomplete'];
  return coverage({ status: 'reviewed', independentReviewCompleted: true, coverageSufficient: sufficient,
    outstandingGaps: gaps, sourceCoverageDecision: {
      schemaVersion: 1, id: 'coverage-decision', scanId: 'A', attemptNumber: 1, rootRunId: 'root',
      reviewerRunId: 'coverage-reviewer', assignmentId: 'coverage-assignment', messageId: 'coverage-mail',
      modelRequestHash: 'b'.repeat(64), modelResponseHash: 'c'.repeat(64), materialDigest: 'a'.repeat(64),
      decision: { schemaVersion: 1, subject: 'source_coverage', materialDigest: 'a'.repeat(64),
        coverageSufficient: sufficient, rationale: 'Reviewed actual frozen evidence <script>not executable</script>',
        reasonCodes: ['evidence_checked'], evidenceRefs: ['phase:actual-receipt'], outstandingGaps: gaps },
    } });
};
const reviewedPage = (extra = {}) => page({ independentReviewCompleted: true,
  candidateReviewStatus: 'completed', materialSubject: 'source_candidates', coverage: reviewedCoverage(), ...extra });
async function mount(read, exportReview = async () => '/exports/review.json') {
  const calls = [], exportCalls = [], exports = {};
  new Function('require', 'exports', transpile(script.content))((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: {
      getNativeSourceFindings: (...args) => { calls.push(args); return read(...args); },
      exportNativeSourceFindings: (...args) => { exportCalls.push(args); return exportReview(...args); },
    } };
    throw new Error(`Unexpected import ${name}`);
  }, exports);
  let bindings;
  const component = { ...exports.default, setup(props, context) { bindings = exports.default.setup(props, context); return () => null; } };
  const props = vue.reactive({ scanId: 'A', attemptNumber: 1 });
  const app = renderer.createApp({ setup: () => () => vue.h(component, props) });
  app.mount({}); await flush();
  return { b: bindings, props, calls, exportCalls, unmount: () => app.unmount(),
    render: () => renderToString(vue.createSSRApp({ render: () => templateExports.render({}, [], props, vue.proxyRefs(bindings), {}, {}) })) };
}

test('source findings API preserves exact attempt and paging in IPC', async () => {
  const calls = [];
  const exports = { sentinelApi: loadSentinelApi(async (...args) => { calls.push(args); return page(); }) };
  await exports.sentinelApi.getNativeSourceFindings('A', 3);
  await exports.sentinelApi.getNativeSourceFindings('A', 2, 50, 10);
  await exports.sentinelApi.exportNativeSourceFindings('A', 2);
  await exports.sentinelApi.exportNativeSourceFindings('A', 2, 'sarif');
  await exports.sentinelApi.exportNativeSourceFindings('A', 2, 'bundle');
  assert.deepEqual(calls, [
    ['get_native_source_findings', { scanId: 'A', attemptNumber: 3, offset: 0, limit: 50 }],
    ['get_native_source_findings', { scanId: 'A', attemptNumber: 2, offset: 50, limit: 10 }],
    ['export_native_source_findings', { scanId: 'A', attemptNumber: 2 }],
    ['export_native_source_findings', { scanId: 'A', attemptNumber: 2, format: 'sarif' }],
    ['export_native_source_findings', { scanId: 'A', attemptNumber: 2, format: 'bundle' }],
  ]);
});

test('coverage preparation displays scope and remaining gaps without claiming a review', async (t) => {
  const h = await mount(async () => page({ coverage: coverage() })); t.after(h.unmount);
  const html = await h.render();
  assert.ok(html.includes('覆盖材料已准备，不代表覆盖审查已完成'));
  assert.ok(html.includes('dependency_graph_incomplete'));
  assert.ok(html.includes('所选文件不代表逐文件审查通过'));
});

test('delivered coverage renders separate completed and sufficiency states with actual provenance', async (t) => {
  for (const sufficient of [false, true]) {
    const h = await mount(async () => reviewedPage({ coverage: reviewedCoverage(sufficient) })); t.after(h.unmount);
    assert.equal(h.b.error.value, '');
    const html = await h.render();
    assert.ok(html.includes('总体覆盖独立审查：已完成'));
    assert.ok(html.includes(sufficient ? '仅在本次冻结范围内裁决为覆盖充分' : '覆盖仍不充分'));
    assert.ok(html.includes('不代表 CI 通过'));
    assert.ok(html.includes('覆盖裁决 coverage-decision · Reviewer coverage-reviewer'));
    assert.ok(html.includes('phase:actual-receipt'));
    assert.ok(html.includes('&lt;script&gt;not executable&lt;/script&gt;'));
    assert.ok(!html.includes('总体覆盖独立审查：尚未完成'));
  }
});

test('zero candidates can show real coverage without inventing a candidate review', async (t) => {
  const h = await mount(async () => reviewedPage({ independentCandidateReviewCompleted: false,
    candidateReviewStatus: 'not_applicable_no_candidates', materialSubject: 'source_coverage',
    materialDigest: 'a'.repeat(64), counts: { decisions: 0, confirmed: 0, rejected: 0, insufficient: 0 }, findings: [] }));
  t.after(h.unmount);
  assert.equal(h.b.error.value, '');
  const html = await h.render();
  assert.ok(html.includes('本轮没有待审候选，未创建候选 Reviewer'));
  assert.ok(html.includes('总体覆盖独立审查：已完成'));
  await h.b.exportReview('bundle');
  assert.deepEqual(h.exportCalls, [['A', 1, 'bundle']]);
});

test('coverage completion requires bound decision, truthful gaps and candidate applicability', async (t) => {
  const mutations = [
    p => { delete p.coverage; },
    p => { delete p.coverage.sourceCoverageDecision; },
    p => { p.coverage.sourceCoverageDecision.scanId = 'foreign'; },
    p => { p.coverage.sourceCoverageDecision.attemptNumber = 2; },
    p => { p.coverage.sourceCoverageDecision.rootRunId = 'foreign'; },
    p => { p.coverage.sourceCoverageDecision.reviewerRunId = 'root'; },
    p => { p.coverage.sourceCoverageDecision.assignmentId = ''; },
    p => { p.coverage.sourceCoverageDecision.messageId = ''; },
    p => { p.coverage.sourceCoverageDecision.modelResponseHash = 'bad'; },
    p => { p.coverage.sourceCoverageDecision.materialDigest = 'd'.repeat(64); },
    p => { p.coverage.sourceCoverageDecision.decision.materialDigest = 'd'.repeat(64); },
    p => { p.coverage.sourceCoverageDecision.decision.subject = 'source_candidates'; },
    p => { p.coverage.sourceCoverageDecision.decision.rationale = ''; },
    p => { p.coverage.sourceCoverageDecision.decision.reasonCodes = []; },
    p => { p.coverage.sourceCoverageDecision.decision.outstandingGaps = []; },
    p => { p.coverage.coverageSufficient = true; p.coverage.sourceCoverageDecision.decision.coverageSufficient = true; },
    p => { p.coverage = reviewedCoverage(true); p.coverage.sourceCoverageDecision.decision.evidenceRefs = []; },
    p => { p.independentCandidateReviewCompleted = false; },
    p => { p.candidateReviewStatus = 'not_applicable_no_candidates'; },
  ];
  for (const mutate of mutations) {
    const next = reviewedPage(); mutate(next);
    const h = await mount(async () => next); t.after(h.unmount);
    assert.equal(h.b.page.value, undefined, mutate.toString());
    assert.ok(h.b.error.value);
  }
});

test('coverage corruption on refresh removes completion badges and export access', async (t) => {
  let damaged = false;
  const h = await mount(async () => damaged ? reviewedPage({ coverage: coverage() }) : reviewedPage());
  t.after(h.unmount);
  assert.ok((await h.render()).includes('总体覆盖独立审查：已完成'));
  damaged = true;
  await h.b.load();
  assert.equal(h.b.page.value, undefined);
  assert.ok(!(await h.render()).includes('总体覆盖独立审查：已完成'));
  await h.b.exportReview();
  assert.equal(h.exportCalls.length, 0);
});

test('coverage metadata rejects foreign scope, forged completion and malformed gap lists', async (t) => {
  for (const extra of [
    { scanId: 'B' }, { attemptNumber: 2 }, { rootRunId: 'other-root' }, { materialDigest: '' },
    { status: 'completed' }, { executionEligible: true }, { independentReviewCompleted: true },
    { selectedFileCount: -1 }, { changedPathsWithoutContentCount: 0.5 },
    { requestedScope: 'anything' }, { effectiveScope: 'all_targets' },
    { outstandingGaps: [] }, { outstandingGaps: ['source_coverage_review', 3] },
  ]) {
    const h = await mount(async () => page({ coverage: coverage(extra) })); t.after(h.unmount);
    assert.equal(h.b.page.value, undefined, JSON.stringify(extra));
    assert.ok(h.b.error.value);
  }
  const h = await mount(async () => unavailable('unverified')); t.after(h.unmount);
  assert.ok(!(await h.render()).includes('覆盖材料已准备'));
});

test('coverage material changes invalidate a paged view', async (t) => {
  const h = await mount(async (_scan, _attempt, offset) => offset === 0
    ? page({ coverage: coverage(), counts: { decisions: 2, confirmed: 2, rejected: 0, insufficient: 0 }, nextOffset: 1 })
    : page({ coverage: coverage({ materialDigest: 'b'.repeat(64) }), offset: 1,
      counts: { decisions: 2, confirmed: 2, rejected: 0, insufficient: 0 }, findings: [finding('d2')] }));
  t.after(h.unmount);
  await h.b.load(true);
  assert.equal(h.b.page.value, undefined);
  assert.ok(h.b.error.value);
});

test('bundle export uses the audited attempt and shares the export lock', async (t) => {
  const pending = deferred();
  const h = await mount(async () => page(), () => pending.promise); t.after(h.unmount);
  assert.ok((await h.render()).includes('导出完整审查包'));
  const first = h.b.exportReview('bundle');
  await h.b.exportReview('sarif');
  assert.deepEqual(h.exportCalls, [['A', 1, 'bundle']]);
  pending.resolve('/exports/source-review-bundle.json'); await first;
  assert.ok((await h.render()).includes('/exports/source-review-bundle.json'));
  for (const status of ['not_available', 'unverified']) {
    const invalid = await mount(async () => unavailable(status)); t.after(invalid.unmount);
    assert.ok(!(await invalid.render()).includes('导出完整审查包'));
    await invalid.b.exportReview('bundle'); assert.equal(invalid.exportCalls.length, 0);
  }
});

test('SARIF export uses the audited attempt, shares the export lock and is offered only for audited evidence', async (t) => {
  const pending = deferred();
  const h = await mount(async () => page(), () => pending.promise); t.after(h.unmount);
  assert.ok((await h.render()).includes('导出本轮审查 SARIF'));
  assert.ok((await h.render()).includes('CI 任务的导出同时保留本轮冻结版本、分析器、发布策略和门禁结果'));
  const first = h.b.exportReview('sarif');
  await h.b.exportReview();
  assert.deepEqual(h.exportCalls, [['A', 1, 'sarif']]);
  pending.resolve('/exports/review.sarif'); await first;
  assert.ok((await h.render()).includes('/exports/review.sarif'));
  for (const status of ['not_available', 'unverified']) {
    const invalid = await mount(async () => unavailable(status)); t.after(invalid.unmount);
    assert.ok(!(await invalid.render()).includes('导出本轮审查 SARIF'));
    await invalid.b.exportReview('sarif'); assert.equal(invalid.exportCalls.length, 0);
  }
});

test('source export requests exact scope without sending cached findings and serializes clicks', async (t) => {
  const pending = deferred();
  const h = await mount(async () => page({ nextOffset: 1, counts: { decisions: 2, confirmed: 2, rejected: 0, insufficient: 0 } }), () => pending.promise);
  t.after(h.unmount);
  const first = h.b.exportReview(); await h.b.exportReview();
  assert.deepEqual(h.exportCalls, [['A', 1]], 'export backend must audit all findings, not the displayed first page');
  assert.ok(h.b.exporting.value);
  pending.resolve('/exports/<unsafe>.json'); await first;
  const html = await h.render();
  assert.ok(html.includes('/exports/&lt;unsafe&gt;.json'));
  assert.ok(html.includes('仅在导出时核验'));
  assert.ok(html.includes('已核验 2 条裁决'), 'export message does not suppress evidence');
  await h.b.load(); assert.equal(h.b.exportPath.value, '');
});

test('failed export revokes stale confirmation and unverified reports cannot be exported', async (t) => {
  for (const exported of [async () => { throw new Error('private I/O or audit error'); }, async () => '']) {
    const h = await mount(async () => page(), exported); t.after(h.unmount);
    await h.b.exportReview();
    assert.equal(h.b.page.value, undefined);
    assert.equal(h.b.exportPath.value, '');
    const html = await h.render();
    assert.ok(html.includes('已撤销页面确认展示')); assert.ok(!html.includes('private I/O'));
    assert.ok(!html.includes('独立审查已确认'));
    await h.b.exportReview(); assert.equal(h.exportCalls.length, 1);
    await h.b.load(); assert.equal(h.b.page.value.status, 'audited');
  }
  for (const status of ['not_available', 'unverified']) {
    const h = await mount(async () => unavailable(status)); t.after(h.unmount);
    await h.b.exportReview(); assert.equal(h.exportCalls.length, 0);
  }
});

test('late export success and failure cannot change another task or attempt', async (t) => {
  for (const failure of [false, true]) {
    const pending = deferred();
    const h = await mount(async (scanId, attemptNumber) => page({ scanId, attemptNumber,
      findings: [finding('new', { scanId, attemptNumber })] }), () => pending.promise);
    t.after(h.unmount);
    const first = h.b.exportReview();
    h.props.scanId = 'B'; h.props.attemptNumber = 2; await flush();
    if (failure) pending.reject(new Error('old export failed')); else pending.resolve('/exports/old.json');
    await first;
    assert.equal(h.b.page.value.scanId, 'B'); assert.equal(h.b.page.value.attemptNumber, 2);
    assert.equal(h.b.exportPath.value, ''); assert.equal(h.b.error.value, '');
    assert.equal(h.b.exporting.value, false);
  }
});

test('real source evidence renderer distinguishes candidate review, coverage and escaped evidence', async (t) => {
  const h = await mount(async () => page({ findings: [finding('d1', { title: '<script>unsafe</script>', rationale: '<img src=x onerror=bad()>' })] }));
  t.after(h.unmount);
  const html = await h.render();
  for (const text of ['已核验 3 条裁决', '已确认 1', '已排除 1', '证据不足 1', '总体覆盖独立审查：尚未完成', 'receipt:one', 'Reviewer reviewer']) assert.ok(html.includes(text), text);
  assert.ok(!html.includes('<script>unsafe</script>'));
  assert.ok(html.includes('&lt;script&gt;unsafe&lt;/script&gt;'));
  assert.ok(!html.includes('<img src=x'));
});

test('unavailable, unverified and audited zero have distinct meanings', async (t) => {
  for (const [response, text] of [[unavailable('not_available'), '这不表示没有漏洞'],
    [unavailable('unverified'), '尚无可核验的完整正式裁决'],
    [page({ counts: { decisions: 2, confirmed: 0, rejected: 1, insufficient: 1 }, findings: [] }), '不等于系统安全或覆盖完整']]) {
    const h = await mount(async () => response); t.after(h.unmount);
    const html = await h.render(); assert.ok(html.includes(text)); assert.ok(!html.includes('独立审查已确认'));
  }
});

test('changing tasks or attempts clears old evidence and rejects late responses', async (t) => {
  const first = deferred();
  const h = await mount(async (_, attempt) => attempt === 1 ? first.promise : page({ attemptNumber: 2, findings: [finding('new', { attemptNumber: 2 })] }));
  t.after(h.unmount);
  h.props.attemptNumber = 2; await flush();
  first.resolve(page()); await flush();
  assert.equal(h.b.page.value.attemptNumber, 2);
  assert.equal(h.b.page.value.findings[0].id, 'new');
  h.props.scanId = 'B'; await flush();
  assert.equal(h.b.page.value, undefined, 'response for A cannot populate B');
  assert.ok(h.b.error.value);
});

test('failed refresh removes previously confirmed findings instead of retaining a success badge', async (t) => {
  let fail = false;
  const h = await mount(async () => { if (fail) throw new Error('private backend error'); return page(); });
  t.after(h.unmount); assert.equal(h.b.page.value.status, 'audited');
  fail = true; await h.b.load();
  assert.equal(h.b.page.value, undefined);
  const html = await h.render(); assert.ok(html.includes('未展示任何确认发现')); assert.ok(!html.includes('private backend error'));
});

test('pagination is serial and retains only matching audited material', async (t) => {
  const pending = deferred(), counts = { decisions: 2, confirmed: 2, rejected: 0, insufficient: 0 };
  const h = await mount(async (_, __, offset) => offset ? pending.promise : page({ nextOffset: 1, counts }));
  t.after(h.unmount);
  const request = h.b.load(true); await h.b.load(true);
  assert.deepEqual(h.calls, [['A', 1, 0], ['A', 1, 1]]);
  pending.resolve(page({ offset: 1, findings: [finding('d2')], counts })); await request;
  assert.deepEqual(h.b.page.value.findings.map(row => row.id), ['d1', 'd2']);
});

test('off-page audit failure clears earlier findings; changed material is never merged', async (t) => {
  for (const second of [unavailable('unverified'), page({ materialDigest: 'foreign', findings: [finding('d2')] })]) {
    const h = await mount(async (_, __, offset) => offset ? { ...second, offset } : page({ nextOffset: 1,
      counts: { decisions: 2, confirmed: 2, rejected: 0, insufficient: 0 } }));
    t.after(h.unmount); await h.b.load(true);
    assert.ok(!h.b.page.value || h.b.page.value.status === 'unverified');
    assert.ok(!(await h.render()).includes('独立审查已确认'));
  }
});

test('wrong schema, scope, qualification or item binding cannot display confirmed evidence', async (t) => {
  for (const response of [page({ schemaVersion: 2 }), page({ attemptNumber: 2 }), page({ counts: null }),
    page({ independentReviewCompleted: true }), page({ independentCandidateReviewCompleted: false }),
    page({ findings: [finding('wrong', { scanId: 'B' })] }), page({ nextOffset: 0 }),
    page({ counts: { decisions: 3, confirmed: -1, rejected: 1, insufficient: 3 } }),
    page({ counts: { decisions: 3, confirmed: 2, rejected: 1, insufficient: 1 } }),
    page({ findings: [] }), page({ nextOffset: 2 }),
    page({ findings: [finding('wrong', { materialDigest: 'other-material' })] }),
    page({ findings: [finding('wrong', { reviewerRunId: 'root' })] }),
    page({ findings: [finding('wrong', { evidenceRefs: [] })] }),
    page({ findings: [finding('wrong', { sourceDecisionId: 'another-decision' })] }),
    page({ findings: [finding(), finding()], counts: { decisions: 2, confirmed: 2, rejected: 0, insufficient: 0 } }),
    { ...unavailable('unverified'), findings: [finding()] }]) {
    const h = await mount(async () => response); t.after(h.unmount);
    assert.equal(h.b.page.value, undefined); assert.ok(h.b.error.value);
  }
});
