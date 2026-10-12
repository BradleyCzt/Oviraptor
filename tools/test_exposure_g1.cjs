// G1 — frontend honesty tests for the exposure surface: limits name their basis (G1-0),
// clusters name their boundary (G1-1), the unit projection never guesses a province
// (G1-2), excluded procurement/public material never counts as risk (G1-3). Pure logic
// modules are transpiled and run for real; no SFC is mocked into agreement.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');

const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const loadModule = (relative, requireModule) => {
  const exports = {};
  new Function('require', 'exports', transpile(readSource(relative)))(requireModule, exports);
  return exports;
};
const contracts = () => loadModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });

const summary = (over = {}) => ({
  projectId: 7, projectActive: true, eligibleAssetsTotal: 3, eligibleAssetsPlanned: 3, assetsBeyondPlan: 0,
  findingsTotal: 0, findingsNew: 0, findingsReviewing: 0, findingsConfirmed: 0, findingsDismissed: 0, highRiskOpen: 0,
  knownLimits: [{ key: 'passive_assets', label: '每轮工作项上限', cap: 37, effect: '不会' }],
  limitsBasis: 'project_settings_v4', limitsError: null, ...over,
});

// ───────────── G1-0 ─────────────

test('g1-0 limits name their basis and an unreadable configuration shows no numbers', () => {
  const c = contracts();
  const saved = c.readScopeSummary(summary());
  assert.equal(saved.limitsError, '', 'null 表示没有错误');
  assert.match(c.describeLimits(saved), /每轮工作项上限 37（项目配置 v4）/);
  const defaults = c.readScopeSummary(summary({ limitsBasis: 'defaults' }));
  assert.match(c.describeLimits(defaults), /代码默认值/);
  const corrupt = c.readScopeSummary(summary({ limitsBasis: 'settings_corrupt', limitsError: '[exposure_settings_corrupt] v3 无效', knownLimits: [], eligibleAssetsPlanned: 0, assetsBeyondPlan: 3 }));
  assert.match(c.describeLimits(corrupt), /采集上限不可用：\[exposure_settings_corrupt\]/);
  assert.doesNotMatch(c.describeLimits(corrupt), /\d+（/, '损坏时不能显示任何上限数字');
  assert.match(c.describePassiveScope(corrupt), /计划数未核实/);
  const old = c.readScopeSummary(summary({ limitsBasis: undefined, limitsError: undefined }));
  assert.equal(old.limitsBasis, 'unreported', '旧后端没有来源字段时标为未报告，而不是假装项目配置');
  assert.match(c.describeLimits(old), /来源未报告/);
});

test('g1-0 the settings panel states that execution is serial', async (t) => {
  const composable = loadModule('src/features/assets/exposure/useExposureSettings.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts();
    if (name === './api') return {};
    throw Error(`Unexpected import ${name}`);
  });
  const view = effective => ({
    version: 2, storedVersion: 2, saved: true, schemaVersion: 1, exposureSources: ['passive_web'],
    exposureMaxRequests: 2000, exposureMaxResponseBytes: 1000000, exposureMaxTotalBytes: 200000000,
    exposureMaxItems: 500, exposureConcurrency: 6, exposureTimeoutSeconds: 12, exposureCtPageSize: 200,
    exposureArchivePageSize: 200, exposureAllowPrivateDestinations: false, exposureAuthorizationDays: 0,
    bounds: { sources: ['passive_web'] }, effective,
  });
  const mount = async (effective) => {
    let state;
    const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
    const app = renderer.createApp({ setup() { state = composable.useExposureSettings({ projectId: () => 7, api: { getExposureSettings: async () => view(effective) }, notify() {} }); return () => null; } });
    app.mount({});
    t.after(() => app.unmount());
    await state.load();
    return state;
  };
  const serial = await mount({ concurrency: 1, concurrencyImplemented: false, concurrencyNote: '执行器串行' });
  assert.match(serial.concurrencyNotice.value, /已从采集合同移除，执行器串行/);
  assert.equal(serial.draft.value.exposureConcurrency, undefined, '并行度不再进入可编辑草稿');
  const silent = await mount(undefined);
  assert.match(silent.concurrencyNotice.value, /后端未报告/, '旧后端不报告时不能默认说成并行');
  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.match(panel, /并行度已从采集合同移除，执行器串行/);
  assert.doesNotMatch(panel, /key: "exposureConcurrency"/);
  assert.doesNotMatch(panel, /type="password"/);
  assert.match(panel, /不是 token/);
});

// ───────────── G1-1 ─────────────

test('g1-1 a cluster names the rule that drew its boundary, legacy rows included', (t) => {
  const composable = loadModule('src/features/assets/exposure/useExposureClusters.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts();
    if (name === './api') return {};
    throw Error(`Unexpected import ${name}`);
  });
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({ setup() { state = composable.useExposureClusters({ projectId: () => 7, api: {}, notify() {} }); return () => null; } });
  app.mount({});
  t.after(() => app.unmount());
  const row = over => ({ id: 1, clusterKey: 'domain:example.com.cn', kind: 'domain', ...over });
  assert.match(state.boundaryText(row({ boundaryBasis: 'psl', pslVersion: 'psl-2026-10-07' })), /公共后缀注册域（psl-2026-10-07）/);
  assert.match(state.boundaryText(row({ boundaryBasis: 'exact_host', clusterKey: 'host:10.0.0.8' })), /精确主机/);
  assert.match(state.boundaryText(row({ boundaryBasis: '' , clusterKey: 'domain:com.cn' })), /旧版末两段/);
  assert.match(state.boundaryText(row({})), /未报告/, '旧后端不报告边界时不能默认说成公共后缀');
  assert.equal(state.boundaryText(row({ kind: 'content', boundaryBasis: 'content_hash' })), '相同响应内容');
  const panel = readSource('src/features/assets/exposure/ExposureClustersPanel.vue');
  assert.match(panel, /分组边界：\{\{ boundaryText\(row\) \}\}/);
});

// ───────────── G1-2 ─────────────

const projection = (over = {}) => ({
  projectionVersion: 'oviraptor.exposure.units.v1', bindingStatus: 'unit',
  group: { entityId: 1, legalName: '中国移动通信集团有限公司', entityKind: 'group' },
  unit: { entityId: 4, legalName: '咪咕浙江杭州分公司', entityKind: 'city_branch' },
  unitBucket: 'professional_subsidiary', unitProvince: null, directoryVersion: 5, bindingVersion: 3, scopeEntities: 8,
  counts: { members: 2, candidateLinked: 0, confirmedGroup: 2, confirmedUnit: 2, unassigned: 0, pendingAuthorization: 0, eligible: 2, planned: 1, executed: 1, uncovered: 1, latestRunId: 9 },
  perAssetEdge: 'integration_pending', notes: ['上级是专业公司「咪咕文化科技有限公司」，不归入任何省公司'], ...over,
});

test('g1-2 the unit projection never places a professional subsidiary under a province', () => {
  const c = contracts();
  const read = c.readScopeSummary(summary({ unitProjection: projection(), unitProjectionError: null }));
  const lines = c.unitProjectionLines(read.unitProjection, read.unitProjectionError);
  assert.match(lines[0], /咪咕浙江杭州分公司」（专业公司，专业公司不归入任何省公司）/);
  assert.doesNotMatch(lines.join('\n'), /所属省公司「/, '专业公司不能显示所属省');
  assert.match(lines[1], /最近一轮计划 1 \/ 已执行 1 · 未覆盖 1/);
  const city = c.readUnitProjection(projection({ unitBucket: 'city', unitProvince: { entityId: 2, legalName: '中国移动通信集团浙江有限公司', entityKind: 'province_company' }, notes: [] }));
  assert.match(c.unitProjectionLines(city, '')[0], /所属省公司「中国移动通信集团浙江有限公司」/);
  const unbound = c.readUnitProjection(projection({ bindingStatus: 'unbound', group: null, unit: null, unitBucket: 'none', counts: { ...projection().counts, confirmedGroup: null, confirmedUnit: null, unassigned: 2 } }));
  assert.match(c.unitProjectionLines(unbound, '')[0], /未绑定组织目录：确认资产 2 个全部记为未分配单位/);
  const ledger = c.readUnitProjection(projection({ bindingStatus: 'group_ledger', unit: null, unitBucket: 'group', counts: { ...projection().counts, confirmedUnit: null, unassigned: 2 } }));
  assert.match(c.unitProjectionLines(ledger, '')[0], /未分配单位 2（逐资产单位归属：后端未报告）/);
  assert.doesNotMatch(c.unitProjectionLines(ledger, '').join('\n'), /integration_pending/);
});

test('g1-2 a malformed or missing projection is reported, never shown as zeros', () => {
  const c = contracts();
  assert.throws(() => c.readUnitProjection(projection({ bindingStatus: 'province_guess' })), /绑定状态未知/);
  assert.throws(() => c.readUnitProjection(projection({ counts: { members: 1 } })), /单位投影缺少/);
  const old = c.readScopeSummary(summary({}));
  assert.equal(old.unitProjection, null);
  assert.match(c.unitProjectionLines(old.unitProjection, old.unitProjectionError)[0], /后端未报告单位投影/);
  const failed = c.readScopeSummary(summary({ unitProjectionError: '[exposure_unit_projection_failed] 目录表缺失' }));
  assert.match(c.unitProjectionLines(failed.unitProjection, failed.unitProjectionError)[0], /单位投影不可用：\[exposure_unit_projection_failed\]/);
});

// ───────────── G1-3 ─────────────

const flush = async () => { for (let i = 0; i < 30; i++) await vue.nextTick(); };
const findingRow = (id, over = {}) => ({
  id, projectId: 7, category: 'document', title: `线索 ${id}`, severity: 'high', confidence: 60,
  status: 'new', sourceUrl: `https://p.invalid/${id}`, evidenceExcerpt: '', assetLabel: '',
  sourceType: 'passive_web', exposureEligible: true, firstSeenAt: 't', lastSeenAt: 't', ...over,
});
const disposition = (value, over = {}) => ({
  disposition: value, infoCategory: 'internal_business', sourceChannel: 'first_party_web',
  reason: '公开招采公告：标题字样「招标公告」，结构字段「项目编号、采购人」', policyVersion: 'oviraptor.exposure.classify.v1',
  conflict: false, conflictNote: '', ...over,
});
function mount(t, modulePath, factory, deps) {
  const c = contracts();
  const composable = loadModule(modulePath, name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return c;
    if (name === './api') return { exposureApi: deps.api };
    throw Error(`Unexpected import ${name}`);
  });
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({ setup() { state = composable[factory](deps); return () => null; } });
  app.mount({});
  t.after(() => app.unmount());
  return state;
}

test('g1-3 dispositions are read strictly and an excluded row is labelled, not hidden', () => {
  const c = contracts();
  const map = c.readDispositionMap({ 5: disposition('excluded_public_procurement'), 6: disposition('risk_candidate', { infoCategory: 'credential' }) });
  assert.match(c.dispositionBadge(map, 5).label, /已排除：公开招采 · 内部业务资料/);
  assert.equal(c.dispositionBadge(map, 5).tone, 'excluded');
  assert.match(c.dispositionBadge(map, 5).title, /项目编号/);
  assert.equal(c.dispositionBadge(map, 6).tone, 'risk');
  assert.match(c.dispositionBadge(map, 7).label, /旧线索（未分类）/, '没有分类行的旧线索单列，不能显示成风险或排除');
  assert.match(c.dispositionBadge(null, 5).label, /处置未报告/, '旧后端不报告处置时不能显示成风险候选');
  assert.equal(c.readDispositionMap(undefined), null);
  assert.throws(() => c.readDispositionMap({ 5: disposition('auto_ignored') }), /处置记录无效/, '未知处置不能被默认成某一类');
  assert.throws(() => c.readDispositionMap({ abc: disposition('risk_candidate') }), /无效 id/);
  assert.throws(() => c.readDispositionMap([1]), /处置旁表/);
  const conflicted = c.readDispositionMap({ 8: disposition('excluded_public_procurement', { conflict: true, conflictNote: '排除不自动撤销，需人工改判' }) });
  assert.match(c.dispositionBadge(conflicted, 8).label, /有冲突/);
  assert.match(c.dispositionBadge(conflicted, 8).title, /需人工改判/);
});

test('g1-3 the overview states the excluded count next to high risk; old backends say not reported', () => {
  const c = contracts();
  const fresh = c.readScopeSummary(summary({ highRiskOpen: 1, findingsExcluded: 2, findingsClassificationPending: 1, findingsUnclassified: 3, classifyPolicyVersion: 'oviraptor.exposure.classify.v1' }));
  assert.match(c.classificationLine(fresh), /自动排除 2（公开招采\/公开材料，仍可在列表查看，不计入高风险）· 待分类 1 · 旧线索未分类 3/);
  const old = c.readScopeSummary(summary({}));
  assert.equal(old.findingsExcluded, null);
  assert.match(c.classificationLine(old), /分类计数未报告/);
  assert.throws(() => c.readScopeSummary(summary({ findingsExcluded: 'two' })), /findingsExcluded 不是有效计数/);
  assert.throws(() => c.readScopeSummary(summary({ findingsExcluded: -1 })), /不是有效计数/);
});

test('g1-3 the surface sends the disposition filter and keeps the side map across pages', async (t) => {
  const inputs = [];
  const api = {
    exposureScopeSummary: async () => summary({ findingsExcluded: 1, findingsClassificationPending: 0, findingsUnclassified: 0 }),
    pageExposureFindings: async input => {
      inputs.push(input);
      return input.cursor
        ? { items: [findingRow(6)], total: 2, hasMore: false, nextCursor: '', pageSize: 1, dispositions: { 6: disposition('risk_candidate') } }
        : { items: [findingRow(5)], total: 2, hasMore: true, nextCursor: 'c1', pageSize: 1, dispositions: { 5: disposition('excluded_public_procurement') } };
    },
    pageExposureRuns: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 50 }),
    listExposureSourceResultsForProject: async () => [],
    listAssets: async () => ({ items: [], total: 0, page: 1, pageSize: 100, summary: {} }),
  };
  const state = mount(t, 'src/features/assets/exposure/useExposureSurface.ts', 'useExposureSurface', {
    projectId: () => 7, active: () => true, api, notify: () => {},
  });
  state.dispositionFilter.value = 'not_excluded';
  await state.load();
  await flush();
  assert.equal(inputs.at(-1).disposition, 'not_excluded', '筛选必须真的发给后端');
  assert.equal(state.badgeFor(5).tone, 'excluded');
  await state.loadMore();
  await flush();
  assert.equal(inputs.at(-1).disposition, 'not_excluded');
  assert.equal(state.badgeFor(5).tone, 'excluded', '翻页不能丢掉已读到的处置');
  assert.equal(state.badgeFor(6).tone, 'risk');
  assert.match(state.classificationNotice.value, /自动排除 1/);
  const source = readSource('src/components/ExposureSurface.vue');
  assert.match(source, /v-model="dispositionFilter"/);
  assert.match(source, /badgeFor\(item\.id\)\.label/);
  assert.match(source, /classificationNotice/);
});

test('g1-3 an excluded finding offers no remediation and never calls the backend for one', async (t) => {
  const saved = [];
  const evidence = over => ({
    finding: { id: 5, category: 'document', title: '招标公告', confidence: 60, status: 'new' },
    observations: [], versions: [{ revision: 1, runId: 3 }], reviewEvents: [], reviewNeeded: null,
    disposition: disposition('excluded_public_procurement'), classifications: [], classificationError: null, ...over,
  });
  let current = evidence({});
  const api = {
    readExposureEvidence: async () => current,
    reviewExposureFindingScoped: async () => 1,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
    saveExposureRemediation: async (...args) => { saved.push(args); return 9; },
  };
  const notes = [];
  const state = mount(t, 'src/features/assets/exposure/useExposureReview.ts', 'useExposureReview', {
    projectId: () => 7, findingId: () => 5, api, notify: (kind, text) => notes.push([kind, text]),
  });
  await state.load();
  await flush();
  assert.match(state.remediationBlocked.value, /已排除：公开招采/);
  assert.match(state.remediationBlocked.value, /请在下方人工改判/);
  assert.doesNotMatch(state.remediationBlocked.value, /integration_pending/);
  state.actor.value = '张三';
  state.remediationNote.value = '整改';
  assert.equal(await state.recordRemediation(), false);
  assert.equal(saved.length, 0, '排除的线索不能发出整改请求');
  assert.match(notes.at(-1)[1], /不是待整改风险/);
  // 人工确认过的线索：不再拦截。
  current = evidence({ finding: { id: 5, category: 'document', title: '招标公告', confidence: 60, status: 'confirmed' } });
  await state.load();
  await flush();
  assert.equal(state.remediationBlocked.value, '');
  const panel = readSource('src/features/assets/exposure/ExposureReviewPanel.vue');
  assert.match(panel, /v-if="!remediationBlocked"/);
  assert.match(panel, /人工改判/);
  assert.match(panel, /reclassifyExposureFinding/);
  assert.doesNotMatch(panel, /data-state="integration_pending"/);
  assert.doesNotMatch(panel, /fetch\(|mailto:|smtp/i);
  // 证据包读取：未知处置被拒绝。
  const c = contracts();
  assert.throws(() => c.readEvidence({ ...evidence({}), disposition: disposition('maybe') }), /证据包的处置记录无效/);
  assert.equal(c.readEvidence({ ...evidence({}), disposition: null }).disposition, null);
});
