// G3 — frontend honesty tests for documents and intel: each document-read refusal keeps its own
// label (status-suffixed codes included), import/intel capabilities read "已实现 · 入口待注册
// （integration_pending）" rather than available, unsupported directions read "未实现", the
// document URL editor names invalid lines instead of dropping them and refuses to save them,
// and public-material declarations require a reason. Pure modules are transpiled and run.

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

test('g3 each document answer keeps its own label and none reads as success', () => {
  const c = contracts();
  const rows = [
    ['partial', 'document_redirect_not_followed:返回 302，Location=http://x', /跳转，未跟随/],
    ['auth_failed', 'document_access_denied_401:401', /401\/403 无权访问（401）/],
    ['auth_failed', 'document_access_denied_403:403', /无权访问（403）/],
    ['failed', 'document_not_found_404:404', /文档不存在（404）/],
    ['rate_limited', 'document_rate_limited_429:429', /请求过多（429）/],
    ['failed', 'document_http_500:500', /其它 HTTP 状态（500）/],
    ['partial', 'document_unsupported_type:pdf', /只记元数据/],
    ['parse_error', 'document_charset_unsupported:gbk', /非 UTF-8/],
    ['parse_error', 'document_not_utf8:x', /不是合法 UTF-8/],
    ['partial', 'document_truncated:x', /只分析了前段/],
    ['blocked', 'document_private_denied:x', /内网/],
    ['blocked', 'document_port_denied:x', /端口/],
    ['partial', 'document_budget_exhausted:x', /额度用尽/],
    ['unconfigured', 'document_link_unconfigured:未指定文档 URL', /未指定文档 URL/],
    ['parse_error', 'parse_error:坏.json', /无法按声明格式解析/],
    ['unsupported', 'unsupported_format:pdf', /格式不支持/],
    ['unsupported', 'import_not_resumable:x', /需重新导入/],
  ];
  const seen = new Set();
  for (const [status, error, reason] of rows) {
    const view = c.sourceStatusView({ sourceKey: 'document_link', status, error });
    assert.match(view.reason, reason, error);
    assert.notEqual(view.tone, 'ok', error);
    seen.add(view.reason);
  }
  assert.equal(seen.size, rows.length, '每种回答各有各的标签');
  assert.equal(c.sourceReasonCode('document_http_99:x'), '', '不是三位状态码就不认');
  assert.equal(c.sourceReasonCode('document_http_500x:x'), '');
  assert.equal(c.sourceReasonCode('github_link_rejected:x'), 'github_link_rejected', 'G2 标签仍可用');
  const empty = c.sourceStatusView({ sourceKey: 'document_link', status: 'completed_no_matches', error: '' });
  assert.equal(empty.tone, 'empty');
  assert.match(empty.status, /不代表没有泄漏/);
  assert.equal(c.sourceLabel('document_link'), '指定公开文档 URL');
  assert.equal(c.sourceLabel('document_import'), '文件导入（用户提供）');
  assert.equal(c.sourceLabel('public_intel_import'), '公开情报导入（用户提供）');
});

test('g3 import and intel read as integration_pending, unsupported directions as 未实现', () => {
  const c = contracts();
  const caps = c.readSourceCapabilities([
    { sourceKey: 'document_link', capability: 'explicit_document_url', credential: 'none', state: 'configured', reason: '已指定 2 个地址' },
    { sourceKey: 'document_link', capability: 'explicit_document_url', credential: 'none', state: 'unconfigured', reason: '未指定文档 URL' },
    { sourceKey: 'document_import', capability: 'file_import', credential: 'none', state: 'integration_pending', integration: 'integration_pending', reason: 'IR-14' },
    { sourceKey: 'public_intel_import', capability: 'public_intel_import', credential: 'none', state: 'integration_pending', integration: 'integration_pending', reason: 'IR-14' },
    { sourceKey: 'document_import', capability: 'pdf_docx_ocr', credential: 'none', state: 'unsupported', reason: '没有解析器' },
    { sourceKey: 'netdisk', capability: 'netdisk_keyword_search', credential: 'none', state: 'unsupported', reason: '不在本期' },
    { sourceKey: 'underground_intel', capability: 'underground_feed', credential: 'none', state: 'unsupported', reason: '不在本期' },
  ]);
  const lines = caps.map(c.capabilityLine);
  const tones = caps.map(c.capabilityTone);
  assert.match(lines[0], /^指定公开文档 URL 只读 · 无需凭据 · 可用/);
  assert.match(lines[1], /未配置：未指定文档 URL/);
  assert.match(lines[2], /文件导入.*已实现 · 入口待注册（integration_pending）：IR-14/);
  assert.match(lines[3], /公开情报导入与分级 .*integration_pending/);
  for (const line of lines.slice(4)) assert.match(line, / · 未实现：/);
  assert.deepEqual(tones, ['ok', 'gap', 'pending', 'pending', 'unsupported', 'unsupported', 'unsupported']);
  for (const line of lines.slice(2)) assert.doesNotMatch(line, /可用/, line);
  // G2 behaviour is unchanged: a credential-less code search stays "未配置（integration_pending）".
  const github = c.capabilityLine({ sourceKey: 'github_code', capability: 'default_branch_code_search', credential: 'required', state: 'unconfigured', integration: 'integration_pending', reason: '' });
  assert.match(github, /未配置（integration_pending）/);
  assert.equal(c.intelGradeLabel('claim_only'), '仅声称（无样本支撑）');
  assert.match(c.intelGradeLabel('sample_supported'), /样本支撑/);
  assert.match(c.intelGradeLabel('human_verified'), /人工核实/);
  assert.match(c.intelGradeLabel('confirmed'), /未知等级/);
  assert.equal(c.sourceLabel('netdisk'), '未知来源 netdisk', '未实现方向不冒充已知来源');
});

test('g3 the document URL list is canonical, names invalid lines and requires a reason for public material', () => {
  const c = contracts();
  const parsed = c.parseDocumentUrls([
    'HTTPS://Docs.Example.TEST/a?v=1#frag',
    'https://docs.example.test/a?v=1',
    'http://example.test/b | 国家标准公开文本',
    'ftp://example.test/c',
    'http://user:pw@example.test/d',
    'not a url',
    'https://example.test/e |   ',
    `https://example.test/f | ${'长'.repeat(201)}`,
    '',
  ].join('\n'));
  assert.deepEqual(parsed.entries, [
    { url: 'http://example.test/b', publicMaterial: true, note: '国家标准公开文本' },
    { url: 'https://docs.example.test/a?v=1', publicMaterial: false, note: '' },
  ]);
  assert.equal(parsed.invalid.length, 5);
  assert.ok(parsed.invalid.includes('ftp://example.test/c'));
  assert.ok(parsed.invalid.includes('https://example.test/e |'));
  const many = Array.from({ length: 21 }, (_, i) => `https://example.test/${i}`).join('\n');
  assert.match(c.parseDocumentUrls(many).invalid.at(-1), /最多 20 个/);
  assert.deepEqual(c.parseDocumentUrls('').entries, []);
  const round = c.formatDocumentUrls(parsed.entries);
  assert.deepEqual(c.parseDocumentUrls(round), { entries: parsed.entries, invalid: [] });
  assert.deepEqual(c.readDocumentUrls(['https://a.test/', { url: 'https://b.test/', publicMaterial: true, note: '公开' }, 7, null]), [
    { url: 'https://a.test/', publicMaterial: false, note: '' },
    { url: 'https://b.test/', publicMaterial: true, note: '公开' },
  ]);
  assert.deepEqual(c.readDocumentUrls(undefined), []);
});

test('g3 the settings editor carries document URLs and refuses to save invalid ones', async (t) => {
  const c = contracts();
  const composable = loadModule('src/features/assets/exposure/useExposureSettings.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return c;
    if (name === './api') return {};
    throw Error(`Unexpected import ${name}`);
  });
  const view = extra => ({
    version: 3, storedVersion: 3, saved: true, schemaVersion: 1, exposureSources: ['passive_web', 'document_link'],
    exposureMaxRequests: 2000, exposureMaxResponseBytes: 1000000, exposureMaxTotalBytes: 200000000,
    exposureMaxItems: 500, exposureConcurrency: 2, exposureTimeoutSeconds: 12, exposureCtPageSize: 200,
    exposureArchivePageSize: 200, exposureAllowPrivateDestinations: false, exposureAuthorizationDays: 0,
    bounds: { sources: ['passive_web', 'certificate_transparency', 'web_archive', 'github_repositories', 'github_code', 'document_link'] }, ...extra,
  });
  const mount = async (extra, saved) => {
    let state;
    const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
    const api = {
      getExposureSettings: async () => view(extra),
      saveExposureSettings: async (_p, _v, settings) => { saved.push(settings); return 4; },
    };
    const app = renderer.createApp({ setup() { state = composable.useExposureSettings({ projectId: () => 7, api, notify() {} }); return () => null; } });
    app.mount({});
    t.after(() => app.unmount());
    await state.load();
    return state;
  };
  const saved = [];
  const state = await mount({ exposureDocumentUrls: [{ url: 'https://docs.example.test/a', publicMaterial: true, note: '公开手册' }, 'https://docs.example.test/b'] }, saved);
  assert.equal(state.documentText.value, 'https://docs.example.test/a | 公开手册\nhttps://docs.example.test/b');
  state.setDocumentUrls('https://docs.example.test/a\nftp://bad');
  assert.deepEqual(state.documentInvalid.value, ['ftp://bad']);
  assert.equal(await state.save(), false, '不合规的地址不能被悄悄丢掉后保存');
  assert.equal(saved.length, 0);
  assert.match(state.error.value, /ftp:\/\/bad/);
  state.setDocumentUrls('https://docs.example.test/z\nhttps://docs.example.test/a | 公开标准');
  assert.equal(await state.save(), true);
  assert.deepEqual(saved[0].exposureDocumentUrls, [
    { url: 'https://docs.example.test/a', publicMaterial: true, note: '公开标准' },
    { url: 'https://docs.example.test/z', publicMaterial: false, note: '' },
  ]);
  const old = await mount({}, []);
  assert.equal(old.documentText.value, '');
  assert.deepEqual(old.draft.value.exposureDocumentUrls, []);
  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.match(panel, /setDocumentUrls/);
  assert.match(panel, /写入导入/);
  assert.doesNotMatch(panel, /不提供导入按钮/);
  assert.match(panel, /`\$\{cap\.sourceKey\}:\$\{cap\.capability\}`/, '同名能力按来源区分');
  assert.match(panel, /importExposureDocuments/, '导入面板调用已注册的命令');
  assert.doesNotMatch(panel, /fetch\(|mailto:|smtp/i);
  const api = readSource('src/features/assets/exposure/api.ts');
  assert.match(api, /import_exposure_documents/);
  assert.match(api, /grade_exposure_intel/);
});

test('g3 an intel evidence package names its grade and a grade read failure stays visible', () => {
  const c = contracts();
  assert.match(c.intelGradeLine({ intelGrade: 'claim_only' }), /情报等级：仅声称.*未经应用验证/);
  assert.match(c.intelGradeLine({ intelGrade: 'human_verified' }), /人工核实/);
  assert.match(c.intelGradeLine({ intelGrade: 'claim_only', intelGradeError: 'bad json' }), /读取失败：bad json/);
  assert.equal(c.intelGradeLine({ intelGrade: null }), '');
  assert.equal(c.intelGradeLine(null), '');
  assert.equal(c.intelGradeLine({ finding: {} }), '', '非情报线索不显示等级');
  const base = { finding: { id: 3, category: 'intel_claim', title: 't', confidence: 40, status: 'new' }, observations: [], versions: [] };
  const read = c.readEvidence({ ...base, intelGrade: 'sample_supported', intelGradeError: null });
  assert.equal(read.intelGrade, 'sample_supported', '解析证据包不能丢掉等级');
  assert.match(c.intelGradeLine(read), /样本支撑/);
  assert.equal(c.readEvidence(base).intelGrade, null);
  assert.equal(c.intelGradeLine(c.readEvidence(base)), '');
  const surface = readSource('src/features/assets/exposure/ExposureSurface.vue');
  assert.match(surface, /intelGradeLine\(evidencePackage\)/);
});
