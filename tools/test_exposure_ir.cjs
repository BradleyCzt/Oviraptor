// P0 IR-15 / IR-01 / IR-14 — registration, retired unversioned review, import and grade builders.
// Builders refuse incomplete payloads before any invoke. Source scans lock the registrations.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

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

const file = (name, format = 'text') => ({ name, format, contentBase64: 'YQ==' });
const importForm = (over = {}) => ({
  channel: 'document_import',
  operator: 'alice',
  sourceDeclaration: '用户手工提供的导出',
  files: [file('a.txt')],
  ...over,
});

test('ir14 a complete import request is built and a declared url is only a field', () => {
  const c = contracts();
  const built = c.buildImportRequest(importForm({ declaredUrl: 'https://example.test/a.txt', batchLabel: 'batch-1' }), 'op-12345678');
  assert.equal(built.ok, true);
  assert.equal(built.payload.operationId, 'op-12345678');
  assert.equal(built.payload.channel, 'document_import');
  assert.equal(built.payload.declaredUrl, 'https://example.test/a.txt');
  assert.equal(built.payload.files[0].name, 'a.txt');
  assert.equal(built.payload.publicMaterial, undefined);
});

test('ir14 import refuses a missing declaration, a public claim without a note, a path name, a bad operation id, and text on the intel channel', () => {
  const c = contracts();
  assert.equal(c.buildImportRequest(importForm({ sourceDeclaration: '  ' }), 'op-12345678').ok, false);
  assert.match(c.buildImportRequest(importForm({ sourceDeclaration: '  ' }), 'op-12345678').error, /来源说明/);
  const pub = c.buildImportRequest(importForm({ publicMaterial: true, publicMaterialNote: '' }), 'op-12345678');
  assert.equal(pub.ok, false);
  assert.match(pub.error, /公开材料必须写明理由/);
  const noted = c.buildImportRequest(importForm({ publicMaterial: true, publicMaterialNote: '公开标准' }), 'op-12345678');
  assert.equal(noted.ok, true);
  assert.equal(noted.payload.publicMaterialNote, '公开标准');
  const slash = c.buildImportRequest(importForm({ files: [file('dir/a.txt')] }), 'op-12345678');
  assert.equal(slash.ok, false);
  assert.match(slash.error, /不能是路径/);
  assert.equal(c.buildImportRequest(importForm(), 'short').ok, false);
  const intel = c.buildImportRequest(importForm({ channel: 'public_intel_import', files: [file('r.txt', 'text')] }), 'op-12345678');
  assert.equal(intel.ok, false);
  assert.match(intel.error, /不在本渠道允许范围内/);
  const intelOk = c.buildImportRequest(importForm({ channel: 'public_intel_import', files: [file('r.csv', 'csv')] }), 'op-12345678');
  assert.equal(intelOk.ok, true);
  assert.equal(c.buildImportRequest(importForm({ files: [] }), 'op-12345678').ok, false);
});

test('ir14 grade requires a reason, and human_verified or sample_supported need a real anchor', () => {
  const c = contracts();
  const base = { findingId: 7, expectedGrade: 'claim_only', grade: 'human_verified', actor: 'alice', reason: '核对过样本' };
  const bare = c.buildGradeRequest(base, 'op-12345678');
  assert.equal(bare.ok, false);
  assert.match(bare.error, /样本观察|单位/);
  const zero = c.buildGradeRequest({ ...base, entityId: 0, sampleObservationId: 0 }, 'op-12345678');
  assert.equal(zero.ok, false);
  const sample = c.buildGradeRequest({ ...base, sampleObservationId: 4 }, 'op-12345678');
  assert.equal(sample.ok, true);
  assert.equal(sample.payload.sampleObservationId, 4);
  assert.equal(sample.payload.expectedGrade, 'claim_only');
  const entity = c.buildGradeRequest({ ...base, entityId: 9 }, 'op-12345678');
  assert.equal(entity.ok, true);
  const short = c.buildGradeRequest({ ...base, grade: 'claim_only', reason: '短' }, 'op-12345678');
  assert.equal(short.ok, false);
  assert.match(short.error, /理由/);
  const supported = c.buildGradeRequest({ ...base, grade: 'sample_supported' }, 'op-12345678');
  assert.equal(supported.ok, false);
  const supportedOk = c.buildGradeRequest({ ...base, grade: 'sample_supported', sampleObservationId: 4 }, 'op-12345678');
  assert.equal(supportedOk.ok, true);
  assert.equal(c.buildGradeRequest({ ...base, grade: 'nope', sampleObservationId: 4 }, 'op-12345678').ok, false);
});

test('ir15 ir01 ir14 the handler, the shared export, and the exposure api match the landed set', () => {
  const lib = readSource('src-tauri/src/lib.rs');
  for (const name of [
    'page_exposure_incidents',
    'read_exposure_incident',
    'page_exposure_incident_locations',
    'act_on_exposure_incident',
    'propose_exposure_incidents',
    'import_exposure_documents',
    'grade_exposure_intel',
  ]) {
    assert.match(lib, new RegExp(`commands::exposure::${name},`), name);
  }
  assert.match(lib, /commands::exposure::review_exposure_finding_scoped,/);
  assert.doesNotMatch(lib, /commands::(?:exposure::)?review_exposure_finding,/);
  const shared = readSource('src/features/assets/api.ts');
  assert.doesNotMatch(shared, /invoke<void>\("review_exposure_finding"/);
  assert.doesNotMatch(shared, /reviewExposureFinding:/);
  const api = readSource('src/features/assets/exposure/api.ts');
  assert.match(api, /import_exposure_documents/);
  assert.match(api, /grade_exposure_intel/);
  assert.match(api, /propose_exposure_incidents/);
  assert.match(api, /IR-15/);
  const rust = readSource('src-tauri/src/commands/exposure.rs');
  assert.doesNotMatch(rust, /pub fn review_exposure_finding\(/);
  assert.match(rust, /pub fn review_exposure_finding_scoped/);
  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.match(panel, /写入导入/);
  assert.doesNotMatch(panel, /不提供导入按钮/);
  assert.doesNotMatch(panel, /fetch\(|mailto:|smtp/i);
  const surface = readSource('src/features/assets/exposure/ExposureSurface.vue');
  assert.match(surface, /写入等级/);
  assert.match(surface, /gradeExposureIntel/);
  assert.doesNotMatch(surface, /fetch\(|mailto:|smtp/i);
  const incidents = readSource('src/features/assets/exposure/ExposureIncidentsPanel.vue');
  assert.match(incidents, /为历史轮次补建议/);
  assert.doesNotMatch(incidents, /fetch\(|mailto:|smtp/i);
});

test('ir07 reclassify refuses a missing reason, a zero classification id, and a short operation id', () => {
  const c = contracts();
  const form = {
    findingId: 5,
    expectedClassificationId: 9,
    disposition: 'risk_candidate',
    infoCategory: 'credential',
    actor: 'alice',
    reason: '这是人工改判理由',
  };
  const ok = c.buildReclassifyRequest(form, 'op-12345678');
  assert.equal(ok.ok, true);
  assert.equal(ok.payload.expectedClassificationId, 9);
  assert.equal(ok.payload.reason, '这是人工改判理由');
  assert.equal(c.buildReclassifyRequest({ ...form, reason: '短' }, 'op-12345678').ok, false);
  assert.equal(c.buildReclassifyRequest({ ...form, expectedClassificationId: 0 }, 'op-12345678').ok, false);
  assert.equal(c.buildReclassifyRequest(form, 'short').ok, false);
  assert.equal(c.buildReclassifyRequest({ ...form, disposition: 'nope' }, 'op-12345678').ok, false);
});

test('ir10 and ir01 and ir08 and ir04 are locked in source', () => {
  const pkg = readSource('package.json');
  assert.match(pkg, /"test:exposure-g1"/);
  assert.match(pkg, /"test:exposure-ir"/);
  assert.match(pkg, /"test:exposure"/);
  const models = readSource('src-tauri/src/assets/model.rs');
  assert.doesNotMatch(models, /ExposureFindingReviewInput/);
  const lib = readSource('src-tauri/src/lib.rs');
  assert.match(lib, /reclassify_exposure_finding/);
  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.doesNotMatch(panel, /key: "exposureConcurrency"/);
  assert.match(panel, /钥匙串账户名，不是 token/);
  assert.doesNotMatch(panel, /type="password"/);
  const review = readSource('src/features/assets/exposure/ExposureReviewPanel.vue');
  assert.match(review, /buildReclassifyRequest/);
  assert.match(review, /reclassifyExposureFinding/);
  assert.doesNotMatch(review, /from "\.\/incidents"/);
  assert.doesNotMatch(review, /fetch\(|mailto:|smtp/i);
  const github = readSource('src-tauri/src/commands/exposure/adapters/github.rs');
  assert.match(github, /find-generic-password/);
  assert.doesNotMatch(github, /ghp_/);
});

test('ir02 the group ledger shows real unit counts, a subsidiary stays apart, and old backends say not reported', () => {
  const c = contracts();
  const base = {
    projectionVersion: 'v', bindingStatus: 'group_ledger', group: { entityId: 1, legalName: '集团', entityKind: 'group' }, unit: null,
    unitBucket: 'group', unitProvince: null, directoryVersion: 5, bindingVersion: 3, scopeEntities: 8,
    counts: { members: 4, candidateLinked: 0, confirmedGroup: 4, confirmedUnit: 3, unassigned: 1, pendingAuthorization: 0, eligible: 4, planned: 0, executed: 0, uncovered: 4, latestRunId: null },
    perAssetEdge: 'ownership_projection',
    perUnit: [
      { unit: { entityId: 2, legalName: '浙江公司', entityKind: 'province_company' }, bucket: 'province', province: { entityId: 2, legalName: '浙江公司', entityKind: 'province_company' }, assets: 2 },
      { unit: { entityId: 4, legalName: '苏州软件', entityKind: 'professional_subsidiary' }, bucket: 'professional_subsidiary', province: null, assets: 1 },
      { unit: { entityId: 9 }, bucket: 'province', province: null, assets: -1 },
    ],
    unassignedReasons: { no_unit_decision: 1, bogus: 'x' },
    notes: [],
  };
  const projection = c.readUnitProjection(base);
  assert.equal(projection.perUnit.length, 2, '无效行被丢弃');
  assert.deepEqual(projection.unassignedReasons, { no_unit_decision: 1 });
  const lines = c.unitProjectionLines(projection, '');
  assert.match(lines[0], /其中有单位 3、未分配单位 1/);
  assert.ok(lines.some((line) => /苏州软件（专业公司，专业公司单列）：资产 1/.test(line)));
  assert.ok(lines.some((line) => /浙江公司（省公司，属「浙江公司」）：资产 2/.test(line)));
  assert.doesNotMatch(lines.join('\n'), /integration_pending/);
  const old = c.readUnitProjection({ ...base, perAssetEdge: undefined, perUnit: undefined, unassignedReasons: undefined, counts: { ...base.counts, confirmedUnit: null } });
  assert.equal(old.perAssetEdge, 'not_reported');
  assert.match(c.unitProjectionLines(old, '')[0], /后端未报告/);
  assert.equal(c.UNASSIGNED_REASON_LABELS.no_unit_decision, '只做了单独确认，没有指明单位');
  const panel = readSource('src/features/assets/exposure/ExposureIncidentsPanel.vue');
  assert.doesNotMatch(panel, /IR-02\/IR-11/);
  const units = readSource('src-tauri/src/commands/exposure/units.rs');
  assert.match(units, /ownership_asset_unit_projection/);
  assert.doesNotMatch(units, /per_asset_edge: "integration_pending"/);
  const own = readSource('src-tauri/src/commands/ownership_unit_projection.rs').split('#[cfg(test)]')[0];
  assert.doesNotMatch(own, /INSERT|UPDATE |DELETE|ALTER TABLE|CREATE TABLE/, '归属投影只读');
  assert.match(readSource('src-tauri/src/commands.rs'), /pub\(crate\) mod ownership;/);
  assert.match(readSource('src-tauri/src/commands/ownership.rs'), /include!\("ownership_unit_projection.rs"\);/);
});
