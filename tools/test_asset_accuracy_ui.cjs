// Real asset SFC setup + templates with synthetic local records. These checks
// exercise presentation contracts, not live collection, SQLite or desktop IPC.
const test = require('node:test');
const { assert, flush, textOf, deferred, mountAssetAccuracy } = require('./asset_accuracy_ui_harness.cjs');

const SUMMARY = 'src/features/assets/components/CollectionRunSummary.vue';
const OBSERVATION = 'src/features/assets/components/AssetObservation.vue';
const QUERY = 'src/features/assets/components/QueryPanel.vue';
const WORKSPACE = 'src/features/assets/workspace/AssetsPluginWorkspace.vue';
const ASSETS = 'src/features/assets/components/AssetWorkspace.vue';
const summary = (overrides = {}) => ({
  schemaVersion: 1, executionComplete: true, coverageComplete: true,
  queries: 3, completedQueries: 3, failedQueries: 0, truncatedQueries: 0,
  partialQueries: 0, invalidRecords: 0, candidates: 37,
  importedCandidates: 35, probedAssets: 31, reusedPages: 0,
  errorCategories: [], reasons: [], ...overrides,
});
const retention = (overrides = {}) => ({
  scope: 'owned_collection_files_and_control_metadata_only', runBytes: 12, projectBytes: 34, globalBytes: 56,
  expiredFiles: 2, expiredBytes: 8, unmanagedRuns: 3, unknownFiles: 4, historyTruncated: true,
  maxRunBytes: 100, maxProjectBytes: 200, maxGlobalBytes: 300, maxPages: 20, maxAgeSeconds: 3600, ...overrides,
});
const run = (overrides = {}) => ({
  id: 41, projectId: 91, profileId: 7, projectName: 'Synthetic workspace',
  name: 'Synthetic collection', pipeline: 'full', status: 'completed', stage: 'completed',
  progress: 100, processed: 37, total: 37, outputDir: '', error: '',
  createdAt: '2026-10-15T08:00:00Z', collectionSummary: summary(), ...overrides,
});
const asset = (extra = {}, overrides = {}) => ({
  id: 11, projectId: 91, host: 'fixture.example.test', probeOutcome: 'accessible',
  projectLastSeen: '2026-10-15T11:00:00Z', lastSeen: '2026-10-15T11:00:00Z',
  extra: { ...extra }, ...overrides,
});
async function mount(t, filename, props, options) {
  const h = await mountAssetAccuracy(filename, props, options);
  t.after(() => { h.unmount(); assert.deepEqual(h.warnings, [], 'The real compiled component must render without Vue warnings'); });
  return h;
}
function one(h, attribute, value) {
  const nodes = h.matching(attribute, value);
  assert.equal(nodes.length, 1, `Expected one rendered ${attribute}=${value}`);
  return nodes[0];
}
const metric = (h, name) => textOf(one(h, 'data-collection-metric', name));
const time = (h, name) => textOf(one(h, 'data-observation-time', name));
const coverage = h => one(h, 'data-collection-coverage').props['data-collection-coverage'];
const execution = h => one(h, 'data-collection-execution').props['data-collection-execution'];
const state = h => one(h, 'data-observation-state').props['data-observation-state'];

test('completed collection renders typed coverage, execution and distinct retained/imported/probed counts', async t => {
  const h = await mount(t, SUMMARY, { run: run() });
  assert.equal(coverage(h), 'complete'); assert.equal(execution(h), 'complete');
  assert.equal(one(h, 'data-job-status').props['data-job-status'], 'completed');
  assert.equal(metric(h, 'queries'), 'Queries 3 / 3');
  assert.equal(metric(h, 'candidates'), 'Retained candidates 37');
  assert.equal(metric(h, 'importedCandidates'), 'Imported 35');
  assert.equal(metric(h, 'probedAssets'), 'Probed 31');
  assert.match(h.text(), /Collection coverage complete/);
});

test('completed 100 percent job with page-limited evidence stays coverage-incomplete', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({
    coverageComplete: false, truncatedQueries: 2, reasons: ['page_limit'],
  }) }) });
  assert.equal(coverage(h), 'incomplete'); assert.equal(execution(h), 'complete');
  assert.match(h.text(), /Collection coverage incomplete/);
  assert.doesNotMatch(h.text(), /Collection coverage complete(?:\s|$)/);
  assert.equal(metric(h, 'truncatedQueries'), 'Truncated queries 2');
  assert.match(h.text(), /Page limit reached/);
});

test('collection query completion and coverage remain independent of a later failed job', async t => {
  const h = await mount(t, SUMMARY, { run: run({ status: 'failed', stage: 'probe', error: 'synthetic probe failure' }) });
  assert.equal(one(h, 'data-job-status').props['data-job-status'], 'failed');
  assert.equal(execution(h), 'complete'); assert.equal(coverage(h), 'complete');
  assert.match(h.text(), /Collection queries completed/);
});

test('failed collection reports retained candidates separately from zero imports and unexecuted probing', async t => {
  const h = await mount(t, SUMMARY, { run: run({ status: 'failed', collectionSummary: summary({
    executionComplete: false, coverageComplete: false, completedQueries: 1, failedQueries: 2,
    candidates: 17, importedCandidates: 0, probedAssets: null,
    reasons: ['query_failure'], errorCategories: ['provider_quota'],
  }) }) });
  assert.equal(execution(h), 'incomplete'); assert.equal(coverage(h), 'incomplete');
  assert.equal(metric(h, 'candidates'), 'Retained candidates 17');
  assert.equal(metric(h, 'importedCandidates'), 'Imported 0');
  assert.equal(metric(h, 'probedAssets'), 'Probed Unknown');
  assert.equal(metric(h, 'failedQueries'), 'Failed queries 2');
  assert.match(h.text(), /Provider quota or rate limit/);
});

test('unknown import and probe counts are not displayed as zero', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({ importedCandidates: null, probedAssets: null }) }) });
  for (const name of ['importedCandidates', 'probedAssets']) {
    assert.match(metric(h, name), /Unknown/); assert.doesNotMatch(metric(h, name), /\b0\b/);
  }
});

test('a real zero probe count stays different from an unknown probe count after reactive prop replacement', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({ probedAssets: null }) }) });
  assert.equal(metric(h, 'probedAssets'), 'Probed Unknown');
  h.props.run = run({ collectionSummary: summary({ probedAssets: 0 }) }); await flush();
  assert.equal(metric(h, 'probedAssets'), 'Probed 0');
});

test('legacy absent or null summaries remain unknown regardless of completed job progress', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: undefined }) });
  assert.equal(coverage(h), 'unknown'); assert.equal(execution(h), 'unknown');
  assert.equal(h.matching('data-collection-metric').length, 0);
  assert.match(h.text(), /A completed job does not prove complete collection/);
  h.props.run = run({ collectionSummary: null }); await flush();
  assert.equal(coverage(h), 'unknown'); assert.equal(execution(h), 'unknown');
});

test('legacy error strings cannot supply coverage evidence or classify unknown records', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: null, error: 'coverage_complete=true; completed 100%; provider_quota' }) });
  assert.equal(coverage(h), 'unknown'); assert.equal(execution(h), 'unknown');
  assert.doesNotMatch(h.text(), /Provider quota or rate limit/);
  h.props.run = run({ collectionSummary: null, status: 'failed', error: 'page_limit coverage_complete=false' }); await flush();
  assert.equal(coverage(h), 'unknown');
  assert.doesNotMatch(h.text(), /Page limit reached/);
});

test('typed evidence takes precedence over contradictory legacy error prose', async t => {
  const h = await mount(t, SUMMARY, { run: run({ error: 'coverage incomplete, page_limit, provider_quota' }) });
  assert.equal(coverage(h), 'complete'); assert.equal(execution(h), 'complete');
  assert.doesNotMatch(h.text(), /Page limit reached|Provider quota or rate limit/);
});

test('malformed or future evidence is unknown rather than a confident partial summary', async t => {
  const h = await mount(t, SUMMARY, { run: run() });
  for (const invalid of [
    summary({ schemaVersion: 2 }), summary({ candidates: -1 }), summary({ queries: '3' }),
    summary({ completedQueries: 1.5 }), summary({ probedAssets: Number.MAX_SAFE_INTEGER + 1 }),
    summary({ coverageComplete: 'false' }), summary({ reasons: [17] }),
  ]) {
    h.props.run = run({ collectionSummary: invalid }); await flush();
    assert.equal(coverage(h), 'unknown'); assert.equal(execution(h), 'unknown');
    assert.equal(h.matching('data-collection-metric').length, 0);
  }
});

for (const [name, override] of Object.entries({
  unfinished: { executionComplete: false },
  failed: { failedQueries: 1 },
  truncated: { truncatedQueries: 1 },
  partial: { partialQueries: 1 },
  invalid: { invalidRecords: 1 },
  missing: { completedQueries: 2 },
  excess: { completedQueries: 4 },
})) test(`contradictory ${name} collection evidence cannot render green coverage`, async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary(override) }) });
  assert.equal(coverage(h), 'unknown'); assert.equal(execution(h), 'unknown');
  assert.equal(h.matching('data-collection-metric').length, 0);
  assert.doesNotMatch(h.text(), /Collection coverage complete(?:\s|$)/);
});

test('excess completed query count remains unknown even when coverage is already marked incomplete', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({ coverageComplete: false, completedQueries: 4 }) }) });
  assert.equal(coverage(h), 'unknown'); assert.equal(execution(h), 'unknown');
});

test('coverage counters and explanations are inside a closed details element while primary counts stay visible', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({
    coverageComplete: false, truncatedQueries: 1, reasons: ['page_limit'],
  }) }) });
  const details = h.nodes().find(node => node.type === 'details');
  assert.ok(details); assert.ok(!details.props.open);
  assert.equal(one(h, 'data-collection-metric', 'truncatedQueries').parent.parent, details);
  assert.notEqual(one(h, 'data-collection-metric', 'candidates').parent.parent, details);
  assert.match(textOf(details), /Coverage details.*Truncated queries 1.*Page limit reached/);
});

test('stable collector precheck and pending reasons are translated from evidence codes', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({
    executionComplete: false, coverageComplete: false, completedQueries: 0,
    reasons: ['coverage_unproven', 'collection_failed', 'provider_precheck_failed', 'collection_pending'],
  }) }) });
  assert.match(h.text(), /Complete coverage has not been proven/);
  assert.match(h.text(), /Collection queries did not finish/);
  assert.match(h.text(), /Provider precheck failed; collection did not start/);
  assert.match(h.text(), /Collection has not finished/);
  assert.doesNotMatch(h.text(), /provider_precheck_failed|collection_pending/);
});

const collectorLabels = [
  ['resource_limited', '采集存储达到资源上限', 'Collection storage resource limit reached'],
  ['retention_limit', '达到页面或存储保留上限', 'Page or storage retention limit reached'],
  ['redacted_locator', '目标地址被脱敏的记录已拒收', 'Records with redacted target addresses were rejected'],
  ['duplicate_record', '来源返回重复记录', 'Source returned duplicate records'],
  ['duplicate_page', '来源返回重复页', 'Source returned a repeated page'],
  ['field_downgrade', '来源回退字段，证据不完整', 'Source fields were reduced; evidence is incomplete'],
  ['total_changed', '来源总数在分页间变化', 'Source total changed between pages'],
  ['total_mismatch', '返回记录数超过来源声明总数', 'Returned records exceed the reported total'],
  ['checkpoint_io', '采集断点读写失败', 'Collection checkpoint read/write failed'],
  ['transport_config', '采集连接配置无效', 'Collection transport configuration is invalid'],
  ['collection_storage_unavailable', '采集存储不可用', 'Collection storage is unavailable'],
];
for (const locale of ['zh', 'en']) test(`collector resource, rejection and receipt errors have specific ${locale} labels in the real component`, async t => {
  const codes = collectorLabels.map(([code]) => code);
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({
    executionComplete: false, coverageComplete: false, completedQueries: 1,
    reasons: codes, errorCategories: codes,
  }) }) }, { locale });
  assert.equal(coverage(h), 'incomplete'); assert.equal(execution(h), 'incomplete');
  const notes = h.nodes().find(node => node.props.class === 'collection-notes'); assert.ok(notes);
  for (const [code, zh, en] of collectorLabels) {
    const label = locale === 'zh' ? zh : en;
    assert.equal(textOf(notes).split(label).length - 1, 1, `${code} must have one specific, deduplicated label`);
    assert.ok(!textOf(notes).includes(code));
  }
  assert.doesNotMatch(h.text(), /其他覆盖缺口|其他查询错误|Other coverage gap|Other query error/);
});

test('registered collection retention budgets and reclaimed files stay inside expandable scoped details', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({ retention: retention() }) }) });
  assert.equal(coverage(h), 'complete');
  const details = h.nodes().find(n => n.type === 'details'); assert.ok(details); assert.ok(!details.props.open);
  assert.equal(textOf(one(h, 'data-retention-metric', 'runBytes')), 'Run bytes 12 / 100');
  assert.equal(textOf(one(h, 'data-retention-metric', 'projectBytes')), 'Project bytes 34 / 200');
  assert.equal(textOf(one(h, 'data-retention-metric', 'globalBytes')), 'Registered global bytes 56 / 300');
  assert.equal(textOf(one(h, 'data-retention-metric', 'expiredFiles')), 'Reclaimed files 2 · 8 bytes');
  assert.equal(textOf(one(h, 'data-retention-metric', 'unmanagedRuns')), 'Unmanaged runs 3 · Unknown files 4');
  assert.equal(textOf(one(h, 'data-retention-metric', 'limits')), 'Pages per run limit 20 · Retention seconds limit 3,600');
  assert.match(h.text(), /other jobs and unregistered directories are outside this budget/);
  assert.match(textOf(one(h, 'data-retention-history-truncated')), /not a complete history/);
});

test('legacy absent retention stays absent without inventing whole-application budget evidence', async t => {
  const h = await mount(t, SUMMARY, { run: run() });
  assert.equal(h.matching('data-retention-metric').length, 0);
  h.props.run = run({ collectionSummary: summary({ retention: null }) }); await flush();
  assert.equal(h.matching('data-retention-metric').length, 0); assert.equal(coverage(h), 'complete');
});

for (const [name, invalid] of Object.entries({
  scope: { scope: 'whole_application' }, negative: { runBytes: -1 },
  unsafe: { globalBytes: Number.MAX_SAFE_INTEGER + 1 }, history: { historyTruncated: 'false' },
})) test(`invalid ${name} retention evidence is hidden without changing independent coverage`, async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({ retention: retention(invalid) }) }) });
  assert.equal(coverage(h), 'complete'); assert.equal(h.matching('data-retention-metric').length, 0);
  assert.match(h.text(), /Retention budget evidence could not be verified/); assert.doesNotMatch(h.text(), /whole_application/);
});

test('partial results, invalid records and reused pages render independent typed counters', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({
    coverageComplete: false, partialQueries: 2, invalidRecords: 4, reusedPages: 7,
    reasons: ['unknown_total', 'invalid_records', 'mixed_observation_times'],
  }) }) });
  assert.equal(metric(h, 'partialQueries'), 'Partial queries 2');
  assert.equal(metric(h, 'invalidRecords'), 'Invalid records 4');
  assert.equal(metric(h, 'reusedPages'), 'Reused pages 7');
  assert.match(h.text(), /Source total is unknown/); assert.match(h.text(), /Invalid source records/);
  assert.match(h.text(), /Pages from different times were reused/);
});

test('unknown reason codes render safe generic labels without raw provider text', async t => {
  const h = await mount(t, SUMMARY, { run: run({ collectionSummary: summary({
    coverageComplete: false, reasons: ['synthetic-unrecognized-reason'], errorCategories: ['synthetic-unrecognized-error'],
  }) }) });
  assert.match(h.text(), /Other coverage gap/); assert.match(h.text(), /Other query error/);
  assert.doesNotMatch(h.text(), /synthetic-unrecognized/);
});

test('compact summaries retain the same incomplete coverage and unknown-count contract', async t => {
  const h = await mount(t, SUMMARY, { compact: true, run: run({ collectionSummary: summary({ coverageComplete: false, probedAssets: null }) }) });
  assert.equal(coverage(h), 'incomplete'); assert.equal(metric(h, 'probedAssets'), 'Probed Unknown');
  assert.equal(one(h, 'data-collection-coverage').props.class.includes('compact'), true);
});

test('observation rendering keeps source, original page receipt and probe timestamps distinct', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_provider_observed_at: '2026-10-12T01:02:03Z', lastupdatetime: '2026-10-01 04:05:06',
    collection_received_at: '2026-10-15T08:00:00Z', probe_checked_at: '2026-10-15T09:00:00Z',
  }) });
  assert.equal(state(h), 'recorded');
  assert.equal(time(h, 'source'), '2026-10-12T01:02:03Z');
  assert.equal(time(h, 'received'), '2026-10-15T08:00:00Z');
  assert.equal(time(h, 'probe'), '2026-10-15T09:00:00Z');
  assert.doesNotMatch(h.text(), /2026-10-15T11:00:00Z|2026-10-01 04:05:06/);
  assert.match(h.text(), /not current availability/); assert.match(h.text(), /not ownership or permission/);
});

test('a later collection receipt cannot turn a historical accessible result into current verification', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-14T09:00:00Z',
  }) });
  assert.equal(state(h), 'unverified');
  assert.match(h.text(), /Historical probe · collection not yet verified/);
  assert.equal(time(h, 'probe'), '2026-10-14T09:00:00Z');
});

test('timestamp ordering compares instants rather than timezone-offset strings', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15T10:00:00+08:00', probe_checked_at: '2026-10-15T03:00:00Z',
  }) });
  assert.equal(state(h), 'recorded');
  h.props.asset = asset({ collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-15T11:00:00+08:00' }); await flush();
  assert.equal(state(h), 'unverified');
});

test('an asset without a probe outcome stays a candidate even with a timestamp or fresh import fields', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-15T11:00:00Z',
  }, { probeOutcome: '' }) });
  assert.equal(state(h), 'candidate'); assert.match(h.text(), /Candidate · no probe result/);
});

test('a skipped probe attempt with a timestamp does not become a recorded verification', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-15T11:00:00Z',
  }, { probeOutcome: 'skipped' }) });
  assert.equal(state(h), 'skipped'); assert.match(h.text(), /Candidate · probe skipped/);
  assert.match(h.text(), /Last probe attempt/); assert.doesNotMatch(h.text(), /Last recorded probe/);
  assert.equal(time(h, 'probe'), '2026-10-15T11:00:00Z');
});

test('old probe outcomes with missing timing remain historical without borrowing last-seen time', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({ collection_received_at: '2026-10-15T10:00:00Z' }) });
  assert.equal(state(h), 'historical'); assert.equal(time(h, 'probe'), 'Unknown');
  assert.doesNotMatch(h.text(), /2026-10-15T11:00:00Z/);
});

test('legacy unzoned dates are displayed but cannot prove collection/probe ordering', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15 10:00:00', probe_checked_at: '2026-10-15T11:00:00Z',
    lastupdatetime: '2026-10-12 05:06:07',
  }) });
  assert.equal(state(h), 'unknown'); assert.equal(time(h, 'source'), '2026-10-12 05:06:07');
  assert.equal(time(h, 'received'), '2026-10-15 10:00:00');
  h.props.asset = asset({ collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-15 11:00:00' }); await flush();
  assert.equal(state(h), 'unknown');
});

for (const badTime of ['2026-10-15 11:00:00', 'not-an-instant', '2026-02-30T11:00:00Z', '2026-10-15T24:00:00Z']) {
  test(`unverifiable probe time ${badTime} without a receipt cannot become a recorded verification`, async t => {
    const h = await mount(t, OBSERVATION, { asset: asset({ probe_checked_at: badTime }) });
    assert.equal(state(h), 'unknown'); assert.equal(time(h, 'probe'), badTime);
    assert.match(h.text(), /Probe timing unknown/); assert.doesNotMatch(h.text(), /Last recorded probe/);
  });
}

for (const [reason, expectedState, explanation] of [
  ['endpoint_changed', 'unverified', /source endpoint changed/],
  ['collection_after_probe', 'unverified', /source page was received after the probe/],
  ['unknown_probe_time', 'historical', /old probe time is missing or not comparable/],
]) test(`explicit ${reason} history takes priority over apparently newer probe clocks`, async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15T08:00:00Z', probe_checked_at: '2026-10-15T09:00:00Z',
    probe_history_reason: reason, probe_observed_ip: '192.0.2.4', probe_observed_endpoint: 'https://old.example.test/',
  }, { ip: '192.0.2.5' }) });
  assert.equal(state(h), expectedState); assert.match(h.text(), explanation);
  assert.equal(time(h, 'observedIp'), '192.0.2.4');
  assert.equal(time(h, 'observedEndpoint'), 'https://old.example.test/');
  assert.doesNotMatch(h.text(), /192\.0\.2\.5/);
  assert.match(h.text(), /not a verified remote peer address/);
});

test('missing historical probe IP and endpoint never borrow current source values', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({ probe_history_reason: 'endpoint_changed' }, {
    ip: '192.0.2.5', link: 'https://current.example.test/',
  }) });
  assert.equal(state(h), 'unverified');
  assert.equal(time(h, 'observedIp'), 'Unknown'); assert.equal(time(h, 'observedEndpoint'), 'Unknown');
  assert.doesNotMatch(h.text(), /192\.0\.2\.5|https:\/\/current\.example\.test/);
});

test('legacy candidate endpoint basis cannot be described as an observed probe input', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({ probe_history_reason: 'unknown_probe_time',
    probe_observed_endpoint: 'https://legacy.example.test/', probe_observed_endpoint_basis: 'legacy_candidate_endpoint',
  }) });
  assert.match(h.text(), /Probe endpoint record/); assert.doesNotMatch(h.text(), /Probe observed endpoint/);
  assert.match(textOf(one(h, 'data-observation-endpoint-basis')), /original probe input is unavailable/);
  h.props.asset = asset({ probe_checked_at: '2026-10-15T11:00:00Z', probe_observed_endpoint: 'https://new.example.test/', probe_observed_endpoint_basis: 'probe_input_url' }); await flush();
  assert.equal(h.matching('data-observation-endpoint-basis').length, 0);
});

test('a new negative probe response replaces historical UI without retaining the old endpoint or reason', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    probe_history_reason: 'endpoint_changed', probe_checked_at: '2026-10-14T09:00:00Z',
    probe_observed_ip: '192.0.2.4', probe_observed_endpoint: 'https://old.example.test/',
  }) });
  assert.equal(state(h), 'unverified');
  h.props.asset = asset({
    collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-15T11:00:00Z',
    probe_observed_ip: '192.0.2.5', probe_observed_endpoint: 'https://current.example.test/',
  }, { probeOutcome: 'unreachable' }); await flush();
  assert.equal(state(h), 'recorded'); assert.equal(time(h, 'observedIp'), '192.0.2.5');
  assert.equal(time(h, 'observedEndpoint'), 'https://current.example.test/');
  assert.equal(h.matching('data-observation-history').length, 0);
  assert.doesNotMatch(h.text(), /192\.0\.2\.4|https:\/\/old\.example\.test|source endpoint changed/);
  assert.match(h.text(), /not current availability/);
});

test('missing observation evidence stays unknown across rerenders instead of using the clock or import time', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({}, { probeOutcome: '' }) });
  assert.equal(state(h), 'candidate');
  for (const name of ['source', 'received', 'probe']) assert.equal(time(h, name), 'Unknown');
  h.props.asset = asset({}, { probeOutcome: '', lastSeen: '2099-12-31T23:59:59Z', projectLastSeen: '2099-12-31T23:59:59Z' }); await flush();
  for (const name of ['source', 'received', 'probe']) assert.equal(time(h, name), 'Unknown');
  assert.doesNotMatch(h.text(), /2099-12-31/);
});

test('blank normalized source evidence falls back to the untouched legacy provider timestamp', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({ collection_provider_observed_at: '  ', lastupdatetime: '2026-10-10 01:02:03' }) });
  assert.equal(time(h, 'source'), '2026-10-10 01:02:03');
});

test('reactive asset navigation replaces old observation state and all evidence values', async t => {
  const h = await mount(t, OBSERVATION, { asset: asset({
    collection_received_at: '2026-10-15T10:00:00Z', probe_checked_at: '2026-10-14T09:00:00Z',
  }) });
  assert.equal(state(h), 'unverified');
  h.props.asset = asset({ collection_provider_observed_at: '2026-10-13T01:00:00Z' }, { id: 12, probeOutcome: '' }); await flush();
  assert.equal(state(h), 'candidate'); assert.equal(time(h, 'source'), '2026-10-13T01:00:00Z');
  assert.equal(time(h, 'received'), 'Unknown'); assert.equal(time(h, 'probe'), 'Unknown');
  assert.doesNotMatch(h.text(), /2026-10-14T09:00:00Z|2026-10-15T10:00:00Z/);
});

const profile = (id, settings, isDefault = false) => ({ id, name: `Synthetic profile ${id}`, settings, isDefault });
const queryProps = profiles => ({ projects: [{ id: 91, name: 'Synthetic workspace', status: 'active' }], profiles, selectedProjectId: 91 });
const queryOptions = { commands: { list_targets: async () => [] } };
const plan = h => textOf(one(h, 'data-probe-plan'));

test('a selected no-probe profile explicitly describes imported candidates without promising accessibility', async t => {
  const h = await mount(t, QUERY, queryProps([profile(7, { runProbe: false, runRefine: true }, true)]), queryOptions);
  assert.equal(one(h, 'data-probe-plan').props['data-probe-plan'], 'disabled');
  assert.equal(one(h, 'data-query-profile').props['data-query-profile'], 7);
  assert.match(plan(h), /Collect → Import candidates → Refine/);
  assert.match(plan(h), /This run does not probe availability/);
  assert.match(plan(h), /Imported source candidates do not prove current accessibility/);
  assert.doesNotMatch(plan(h), /→ Probe/);
});

test('disabling both refine and probe shows only the stages that will actually execute', async t => {
  const h = await mount(t, QUERY, queryProps([profile(7, { runProbe: false, runRefine: false })]), queryOptions);
  assert.match(plan(h), /Collect → Import candidates/); assert.doesNotMatch(plan(h), /→ Refine|→ Probe/);
});

test('enabled probing still advertises the backend-required refine stage even when runRefine is false', async t => {
  const h = await mount(t, QUERY, queryProps([profile(7, { runProbe: true, runRefine: false })]), queryOptions);
  assert.equal(one(h, 'data-probe-plan').props['data-probe-plan'], 'enabled');
  assert.match(plan(h), /Collect → Import candidates → Refine → Probe/);
  assert.doesNotMatch(plan(h), /does not probe availability/);
});

test('legacy absent or non-boolean runProbe settings preserve the backend enabled default', async t => {
  const h = await mount(t, QUERY, queryProps([profile(7, {})]), queryOptions);
  assert.equal(one(h, 'data-probe-plan').props['data-probe-plan'], 'enabled');
  h.props.profiles = [profile(7, { runProbe: 'false', runRefine: false })]; await flush();
  assert.equal(one(h, 'data-probe-plan').props['data-probe-plan'], 'enabled');
  assert.match(plan(h), /Refine → Probe/);
});

test('the rendered profile selector updates the execution plan before the selected profile is started', async t => {
  const h = await mount(t, QUERY, queryProps([
    profile(7, { runProbe: false }, true), profile(8, { runProbe: true, runRefine: false }),
  ]), { commands: {
    list_targets: async () => [{ id: 1, projectId: 91, targetType: 'domain', value: 'fixture.example.test', enabled: true }],
    start_job: async () => 55,
  } });
  assert.equal(one(h, 'data-probe-plan').props['data-probe-plan'], 'disabled');
  const selector = h.nodes().find(node => node.type === 'select' && node.children.some(child => child.type === 'option' && child.props.value === 8));
  assert.ok(selector, 'The actual profile select must be rendered');
  assert.equal(typeof selector.props['onUpdate:modelValue'], 'function');
  selector.props['onUpdate:modelValue'](8); await flush();
  assert.equal(one(h, 'data-query-profile').props['data-query-profile'], 8);
  assert.equal(one(h, 'data-probe-plan').props['data-probe-plan'], 'enabled');
  assert.match(plan(h), /Refine → Probe/);
  const button = h.nodes().find(node => node.type === 'button' && textOf(node).includes('Start job'));
  assert.ok(button); assert.ok(!button.props.disabled); await button.props.onClick(); await flush();
  const starts = h.calls.filter(([command]) => command === 'start_job');
  assert.equal(starts.length, 1); assert.equal(starts[0][1].input.profileId, 8);
  assert.equal(starts[0][1].input.projectId, 91);
  assert.deepEqual(h.events.filter(([name]) => name === 'started'), [['started', 55]]);
});

const workspaceProps = profiles => ({ ...queryProps(profiles), projectId: 91, view: 'dashboard', refreshToken: 0 });
const workspaceCommands = (overrides = {}) => ({
  dashboard_stats: async () => ({ projectCount: 1, assetCount: 37, aliveCount: 31, pendingCount: 2, blockedCount: 0, runningJobs: 0 }),
  list_runs: async () => [], list_events: async () => [], start_job: async () => 56, ...overrides,
});

test('dashboard collection uses the displayed default profile and does not promise disabled probing', async t => {
  const h = await mount(t, WORKSPACE, workspaceProps([
    profile(7, { runProbe: true }), profile(8, { runProbe: false, runRefine: false }, true),
  ]), { commands: workspaceCommands() });
  assert.equal(one(h, 'data-quick-profile').props['data-quick-profile'], 8);
  assert.equal(one(h, 'data-quick-probe-plan').props['data-quick-probe-plan'], 'disabled');
  assert.equal(textOf(one(h, 'data-asset-action', 'collect')), ' Collect candidates');
  assert.match(h.text(), /Synthetic profile 8 · Collect → Import candidates/);
  assert.match(h.text(), /This run does not probe availability/);
  assert.doesNotMatch(h.text(), /Collect and probe|→ Probe|→ Refine/);
  await h.click('data-asset-action', 'collect');
  const starts = h.calls.filter(([command]) => command === 'start_job');
  assert.equal(starts.length, 1); assert.equal(starts[0][1].input.profileId, 8);
  assert.equal(starts[0][1].input.projectId, 91); assert.equal(starts[0][1].input.pipeline, 'full');
});

test('dashboard profile updates refresh its real plan and keep the same start contract', async t => {
  const h = await mount(t, WORKSPACE, workspaceProps([profile(7, { runProbe: false, runRefine: false })]), { commands: workspaceCommands() });
  assert.equal(one(h, 'data-quick-probe-plan').props['data-quick-probe-plan'], 'disabled');
  h.props.profiles = [profile(9, { runProbe: true, runRefine: false }, true)]; await flush();
  assert.equal(one(h, 'data-quick-profile').props['data-quick-profile'], 9);
  assert.equal(one(h, 'data-quick-probe-plan').props['data-quick-probe-plan'], 'enabled');
  assert.match(h.text(), /Synthetic profile 9 · Collect → Import candidates → Refine → Probe/);
  assert.match(textOf(one(h, 'data-asset-action', 'collect')), /Collect and probe/);
  assert.doesNotMatch(h.text(), /This run does not probe availability/);
  await h.click('data-asset-action', 'collect');
  assert.equal(h.calls.find(([command]) => command === 'start_job')[1].input.profileId, 9);
});

test('a dashboard without profiles stays unknown and cannot issue a start', async t => {
  const h = await mount(t, WORKSPACE, workspaceProps([]), { commands: workspaceCommands() });
  assert.equal(one(h, 'data-quick-probe-plan').props['data-quick-probe-plan'], 'unknown');
  assert.match(h.text(), /Create an execution profile first/);
  assert.doesNotMatch(h.text(), /Collect and probe|→ Probe/);
  await h.click('data-asset-action', 'collect');
  assert.equal(h.calls.filter(([command]) => command === 'start_job').length, 0);
  assert.ok(h.events.some(event => event[0] === 'notify' && event[1] === 'error'));
});

test('terminal job events reread typed coverage instead of interpreting event progress or message text', async t => {
  let rows = [run({ status: 'running', stage: 'collect', progress: 50, collectionSummary: null })];
  const h = await mount(t, WORKSPACE, workspaceProps([profile(7, {})]), { commands: workspaceCommands({ list_runs: async () => rows }) });
  assert.equal(coverage(h), 'unknown');
  rows = [run({ collectionSummary: summary({ coverageComplete: false, truncatedQueries: 1, reasons: ['page_limit'] }) })];
  h.emitNative('job-progress', { runId: 41, status: 'completed', stage: 'completed', progress: 100, message: 'all covered' });
  await flush();
  assert.equal(h.calls.filter(([command]) => command === 'list_runs').length, 2);
  assert.equal(one(h, 'data-job-status').props['data-job-status'], 'completed');
  assert.equal(coverage(h), 'incomplete'); assert.equal(metric(h, 'truncatedQueries'), 'Truncated queries 1');
});

test('pure reprobe task cards do not invent collection coverage', async t => {
  const h = await mount(t, WORKSPACE, { ...workspaceProps([profile(7, {})]), view: 'tasks' }, {
    commands: workspaceCommands({ list_runs: async () => [run({ pipeline: 'reprobe', collectionSummary: null })] }),
  });
  assert.equal(h.matching('data-collection-coverage').length, 0);
  assert.match(h.text(), /Execution · Completed/);
});

test('asset filter labels describe historical observations while their rendered controls keep existing query keys', async t => {
  const reads = [];
  const h = await mount(t, ASSETS, { projects: queryProps([]).projects, selectedProjectId: 91 }, { commands: {
    list_assets: async ({ query }) => { reads.push(query); return { items: [], total: 0, page: query.page, pageSize: query.pageSize }; },
  } });
  const probe = h.nodes().find(node => node.type === 'select' && node.children.some(child => child.type === 'option' && child.props.value === 'browser_accessible'));
  assert.ok(probe); assert.match(textOf(probe), /Last probe accessible \(incl. legacy\)/);
  probe.props['onUpdate:modelValue']('browser_accessible'); await probe.props.onChange(); await flush();
  assert.equal(reads.at(-1).probeView, 'browser_accessible');
  const sort = h.nodes().find(node => node.type === 'select' && node.children.some(child => child.type === 'option' && child.props.value === 'priority'));
  assert.ok(sort); assert.match(textOf(sort), /Project last imported/); assert.match(textOf(sort), /Discovery label/);
  sort.props['onUpdate:modelValue']('projectLastSeen'); await sort.props.onChange(); await flush();
  assert.equal(reads.at(-1).sortBy, 'projectLastSeen');
  sort.props['onUpdate:modelValue']('company'); await sort.props.onChange(); await flush();
  assert.equal(reads.at(-1).sortBy, 'company');
  assert.match(h.text(), /Validity review does not confirm ownership or grant execution permission/);
  assert.match(h.text(), /they do not prove this run re-probed the asset/);
});

const queueSummary = (overrides = {}) => ({ all: 0, pending: 0, uncertain: 0, confirmed: 0, rejected: 0, notApplicable: 0, sentToAgent: 0, ...overrides });
const assetPage = (query, overrides = {}) => ({ items: [], total: 0, memberTotal: 0, page: query.page, pageSize: query.pageSize, summary: queueSummary(), ...overrides });
const snapshotState = h => one(h, 'data-asset-snapshot-state').props['data-asset-snapshot-state'];
const summaryCount = (h, name) => textOf(one(h, 'data-asset-summary', name));
const assetCount = (h, name) => textOf(one(h, 'data-asset-count', name));
const assetProps = { projects: queryProps([]).projects, selectedProjectId: 91 };
async function refreshAssets(h) {
  const button = h.nodes().find(n => n.type === 'button' && textOf(n).trim() === 'Refresh');
  assert.ok(button); assert.equal(typeof button.props.onClick, 'function'); await button.props.onClick(); await flush();
}
async function searchAssets(h, text) {
  const input = h.nodes().find(n => n.type === 'input' && n.props.placeholder === 'Search project, company, domain, IP, title or asset key');
  assert.ok(input); input.props['onUpdate:modelValue'](text); input.props.onKeyup({ key: 'Enter' }); await flush();
}

test('first asset read loading and failure never masquerade as a real zero, while an explicit retry can prove zero', async t => {
  const first = deferred(); let reads = 0;
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => ++reads === 1 ? first.promise : assetPage(query),
  } });
  assert.equal(snapshotState(h), 'loading'); assert.equal(summaryCount(h, 'all'), 'Loading…');
  assert.match(assetCount(h, 'total'), /Asset counts loading/); assert.doesNotMatch(h.text(), /No matching assets/);
  first.reject(new Error('Synthetic initial read failure')); await flush();
  assert.equal(snapshotState(h), 'unknown'); assert.equal(summaryCount(h, 'all'), 'Unknown');
  assert.match(assetCount(h, 'total'), /Asset counts unknown/); assert.doesNotMatch(h.text(), /No matching assets|0 display groups/);
  await refreshAssets(h);
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '0');
  assert.match(assetCount(h, 'total'), /0 display groups/); assert.match(assetCount(h, 'members'), /0 raw URLs/);
  assert.match(h.text(), /No matching assets/); assert.equal(reads, 2);
});

test('new-filter read failures retain rows as stale evidence but never attach old counts to the current filter', async t => {
  const second = deferred(); let reads = 0;
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => ++reads === 1 ? assetPage(query, { items: [asset({}, { title: 'Synthetic previous snapshot' })], total: 7, memberTotal: 12, summary: queueSummary({ all: 90, pending: 52, confirmed: 30, rejected: 5, notApplicable: 3 }) })
      : reads === 2 ? second.promise : assetPage(query),
  } });
  assert.equal(summaryCount(h, 'all'), '90');
  await searchAssets(h, 'new-filter');
  assert.equal(snapshotState(h), 'stale'); assert.equal(summaryCount(h, 'all'), 'Loading…');
  assert.match(h.text(), /Synthetic previous snapshot/); assert.match(h.text(), /last successful snapshot/);
  second.reject(new Error('Synthetic new filter failed')); await flush();
  assert.equal(snapshotState(h), 'stale'); assert.equal(summaryCount(h, 'all'), 'Unknown');
  assert.match(assetCount(h, 'total'), /Asset counts unknown/); assert.doesNotMatch(assetCount(h, 'total'), /7 display groups|12 raw URLs/);
  assert.match(h.text(), /Synthetic previous snapshot/); assert.doesNotMatch(h.text(), /No matching assets/);
  await refreshAssets(h); assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '0');
  assert.doesNotMatch(h.text(), /Synthetic previous snapshot/); assert.match(h.text(), /No matching assets/);
});

test('same-query refresh explicitly labels the old successful snapshot until the new authoritative count arrives', async t => {
  const second = deferred(); let reads = 0;
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => ++reads === 1 ? assetPage(query, { total: 7, memberTotal: 7, summary: queueSummary({ all: 7, pending: 7 }) }) : second.promise,
  } });
  const refreshing = refreshAssets(h); await flush();
  assert.equal(snapshotState(h), 'refreshing'); assert.equal(summaryCount(h, 'all'), '7');
  assert.match(h.text(), /Refreshing; showing the last successful snapshot/);
  second.resolve(assetPage({ page: 1, pageSize: 50 }, { total: 9, memberTotal: 9, summary: queueSummary({ all: 9, pending: 9 }) })); await refreshing;
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '9');
});

test('a late old-filter count cannot overwrite the committed newer-filter count', async t => {
  const pending = [];
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => { if (!query.search) return assetPage(query); const wait = deferred(); pending.push({ query, wait }); return wait.promise; },
  } });
  await searchAssets(h, 'older'); await searchAssets(h, 'newer'); assert.equal(pending.length, 2);
  pending[1].wait.resolve(assetPage(pending[1].query, { total: 8, memberTotal: 8, summary: queueSummary({ all: 8, pending: 8 }) })); await flush();
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '8');
  pending[0].wait.resolve(assetPage(pending[0].query, { total: 999, memberTotal: 999, summary: queueSummary({ all: 999, pending: 999 }) })); await flush();
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '8'); assert.doesNotMatch(assetCount(h, 'total'), /999/);
});

test('the same query keeps display groups, raw filtered URLs and pre-decision summary denominators distinct', async t => {
  const reads = [];
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => { reads.push(query); return assetPage(query, {
      items: [asset({}, { id: 1 }), asset({}, { id: 2 })], total: query.collapseIdenticalResponses ? 2 : 52,
      memberTotal: 52, summary: queueSummary({ all: 90, pending: 52, confirmed: 30, rejected: 5, notApplicable: 3 }),
    }); },
  } });
  assert.equal(reads[0].probeView, 'browser_review'); assert.equal(reads[0].decisionView, 'review');
  assert.equal(reads[0].deletedView, 'active'); assert.equal(reads[0].collapseIdenticalResponses, true);
  assert.match(assetCount(h, 'total'), /2 display groups/); assert.match(assetCount(h, 'members'), /52 raw URLs/);
  assert.equal(summaryCount(h, 'all'), '90'); assert.match(h.text(), /Raw URLs · before decision filter/);
  const label = h.nodes().find(n => n.type === 'label' && textOf(n).includes('Show one identical response'));
  const checkbox = label.children.find(n => n.type === 'input'); assert.ok(checkbox);
  checkbox.props['onUpdate:modelValue'](false); checkbox.props.onChange(); await flush();
  assert.equal(reads.at(-1).collapseIdenticalResponses, false); assert.equal(reads.at(-1).decisionView, 'review');
  assert.match(assetCount(h, 'total'), /52 URLs/); assert.equal(h.matching('data-asset-count', 'members').length, 0);
  assert.equal(summaryCount(h, 'all'), '90');
});

test('legacy missing queue summaries or raw member counts are unknown rather than synthesized from folded totals', async t => {
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => ({ items: [asset()], total: 2, page: query.page, pageSize: query.pageSize }),
  } });
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), 'Unknown');
  assert.match(assetCount(h, 'total'), /2 display groups/); assert.match(assetCount(h, 'members'), /raw URL count unknown/);
});

test('invalid negative or contradictory asset counts do not become available or render zero', async t => {
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: () => ({ items: [], total: -1, memberTotal: 0, summary: queueSummary() }),
  } });
  assert.equal(snapshotState(h), 'unknown'); assert.equal(summaryCount(h, 'all'), 'Unknown');
  assert.match(assetCount(h, 'total'), /Asset counts unknown/); assert.doesNotMatch(h.text(), /No matching assets/);
});

test('unknown manual-decision records remain a separate business bucket, not a read failure or pending count', async t => {
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => assetPage(query, { total: 5, memberTotal: 5,
      summary: queueSummary({ all: 5, pending: 1, confirmed: 1, rejected: 1, unknown: 2 }) }),
  } });
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '5'); assert.equal(summaryCount(h, 'pending'), '1');
  assert.match(textOf(one(h, 'data-asset-unknown-decisions')), /2 manual decisions are unknown or unclassified/);
  assert.equal(h.nodes().filter(n => n.type === 'button' && textOf(n).includes('unknown or unclassified')).length, 0);
});

test('opening an unfolded unseen asset rereads rows and every same-filter count after the persisted seen mark', async t => {
  const opened = [], reads = [], reread = deferred(); let marked = false;
  const url = 'https://fixture.example.test/';
  const h = await mount(t, ASSETS, assetProps, { openUrl: async value => opened.push(value), commands: {
    list_assets: ({ query }) => {
      reads.push(query);
      if (marked) return reread.promise;
      return assetPage(query, { items: [asset({}, { host: url })], total: 1, memberTotal: 1,
        summary: queueSummary({ all: 1, pending: 1 }) });
    },
    mark_asset_entry_opened: args => {
      assert.deepEqual(args, { projectId: 91, assetId: 11 }); marked = true; return '2026-10-15T13:00:00Z';
    },
  } });
  const foldLabel = h.nodes().find(n => n.type === 'label' && textOf(n).includes('Show one identical response'));
  const fold = foldLabel.children.find(n => n.type === 'input');
  fold.props['onUpdate:modelValue'](false); fold.props.onChange(); await flush();
  const seen = h.nodes().find(n => n.type === 'select' && n.children.some(child => child.type === 'option' && child.props.value === 'unseen'));
  assert.ok(seen); seen.props['onUpdate:modelValue']('unseen'); seen.props.onChange(); await flush();
  const before = reads.length, sameQuery = reads.at(-1);
  assert.equal(sameQuery.collapseIdenticalResponses, false); assert.equal(sameQuery.entrySeenView, 'unseen');
  assert.equal(summaryCount(h, 'all'), '1'); assert.match(assetCount(h, 'total'), /1 URLs/);
  const host = h.nodes().find(n => n.type === 'button' && n.props.class === 'asset-link' && textOf(n) === url);
  assert.ok(host); const click = host.props.onClick(); await flush();
  assert.deepEqual(opened, [url]); assert.equal(h.calls.filter(([name]) => name === 'mark_asset_entry_opened').length, 1);
  assert.equal(reads.length, before + 1); assert.deepEqual(reads.at(-1), sameQuery);
  assert.equal(snapshotState(h), 'refreshing'); assert.equal(summaryCount(h, 'all'), '1');
  assert.match(h.text(), /Refreshing; showing the last successful snapshot/);
  reread.resolve(assetPage(sameQuery)); await click; await flush();
  assert.equal(snapshotState(h), 'ready'); assert.equal(summaryCount(h, 'all'), '0');
  assert.equal(summaryCount(h, 'pending'), '0'); assert.match(assetCount(h, 'total'), /0 URLs/);
  assert.match(h.text(), /No matching assets/);
  assert.equal(h.nodes().filter(n => n.type === 'button' && n.props.class === 'asset-link' && textOf(n) === url).length, 0);
});

test('empty probe outcomes stay explicitly unknown even when a historical HTTP status is 200', async t => {
  const h = await mount(t, ASSETS, assetProps, { commands: {
    list_assets: ({ query }) => assetPage(query, { items: [asset({}, { probeOutcome: '', statusCode: '200' })], total: 1, memberTotal: 1, summary: queueSummary({ all: 1, pending: 1 }) }),
  } });
  const cell = h.nodes().find(n => n.type === 'td' && n.props.class === 'col-probeOutcome'); assert.ok(cell);
  assert.equal(textOf(cell), 'Unknown · no probe result'); assert.doesNotMatch(textOf(cell), /Web accessible/);
});

for (const locale of ['zh', 'en']) test(`the saved exact asset-statistics activity uses its ${locale} label without parsing sample counts`, async t => {
  const summary = 'Exact inventory 77; returned sample 20. Unknown states remain separate.';
  const h = await mount(t, 'src/features/conversations/ConversationToolActivity.vue', { activity: [{ id: 'stats-1', name: 'assets_get_stats', status: 'completed', summary, round: 1 }] }, { locale });
  assert.match(h.text(), locale === 'zh' ? /统计项目资产/ : /Count project assets/);
  assert.ok(h.text().includes(summary)); assert.doesNotMatch(h.text(), /assets_get_stats/);
});
