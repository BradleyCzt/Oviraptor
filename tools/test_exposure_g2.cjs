// G2 — frontend honesty tests for GitHub: every provider refusal keeps its own label, an empty
// bounded result is not shown as success or as failure, code search without a credential is
// "未配置/integration_pending" rather than available, and the official-owner list rejects
// malformed logins instead of dropping them. Pure modules are transpiled and run for real.

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

test('g2 each GitHub refusal has its own label and none reads as an empty result', () => {
  const c = contracts();
  const rows = [
    ['auth_failed', 'github_auth_failed_401:GitHub 401：身份无效', '401 身份无效'],
    ['auth_failed', 'github_forbidden_403:GitHub 403：无权访问该资源', '403 无权访问'],
    ['rate_limited', 'github_rate_limited_403:GitHub 403：额度已用尽', '403 额度用尽'],
    ['rate_limited', 'github_rate_limited_429:GitHub 429：请求过多', '429 请求过多'],
    ['failed', 'github_query_rejected_422:GitHub 422：查询被拒绝', '422 查询被拒绝'],
    ['failed', 'github_unavailable_503:GitHub 503：服务暂不可用', '503 服务暂不可用'],
    ['failed', 'github_redirect_not_followed:GitHub 返回 302', '接口跳转未跟随（凭据不随跳转）'],
  ];
  const seen = new Set();
  for (const [status, error, reason] of rows) {
    const view = c.sourceStatusView({ sourceKey: 'github_repositories', status, error });
    assert.equal(view.reason, reason, error);
    assert.equal(view.tone, 'gap', error);
    assert.doesNotMatch(view.status, /未命中|已完成/);
    seen.add(view.reason);
  }
  assert.equal(seen.size, rows.length, '七种拒绝各有各的标签');
  assert.equal(c.sourceStatusView({ sourceKey: 'github_repositories', status: 'rate_limited', error: '' }).status, '额度受限');
  assert.equal(c.sourceStatusView({ sourceKey: 'github_repositories', status: 'auth_failed', error: '' }).status, '身份被拒');
});

test('g2 an empty bounded result is its own state, neither success nor failure', () => {
  const c = contracts();
  const empty = c.sourceStatusView({ sourceKey: 'github_repositories', status: 'completed_no_matches', error: '' });
  assert.equal(empty.tone, 'empty');
  assert.match(empty.status, /不代表没有泄漏/);
  const done = c.sourceStatusView({ sourceKey: 'github_repositories', status: 'completed', error: '' });
  assert.equal(done.tone, 'ok');
  assert.match(done.status, /有界/);
  const unknown = c.sourceStatusView({ sourceKey: 'mystery', status: 'weird', error: '' });
  assert.equal(unknown.tone, 'gap');
  assert.match(unknown.label, /未知来源 mystery/);
  assert.match(unknown.status, /未知状态 weird/);
});

test('g2 code search without a credential is unconfigured with integration_pending, not available', () => {
  const c = contracts();
  const source = c.sourceStatusView({
    sourceKey: 'github_code',
    status: 'unconfigured',
    error: 'integration_pending:凭据引用端口尚未装配（IR-04 待批准），GitHub 代码搜索未配置；未发送任何请求',
  });
  assert.equal(source.tone, 'pending');
  assert.equal(source.label, 'GitHub 代码搜索（需凭据）');
  assert.match(source.reason, /IR-04/);
  const caps = c.readSourceCapabilities([
    { sourceKey: 'github_repositories', capability: 'repository_metadata', credential: 'none', state: 'configured', reason: '匿名公开元数据' },
    { sourceKey: 'github_code', capability: 'default_branch_code_search', credential: 'required', state: 'unconfigured', integration: 'integration_pending', reason: '凭据引用端口尚未装配（IR-04 待批准）' },
    { sourceKey: 'github_code', capability: 'controlled_blob_read', credential: 'required', state: 'unconfigured', integration: 'integration_pending', reason: '' },
  ]);
  assert.equal(caps.length, 3);
  assert.match(c.capabilityLine(caps[0]), /仓库元数据搜索 · 无需凭据 · 可用/);
  assert.match(c.capabilityLine(caps[1]), /默认分支代码搜索 · 需要凭据 · 未配置（integration_pending）：凭据引用端口/);
  assert.doesNotMatch(c.capabilityLine(caps[1]), /可用/);
  assert.match(c.capabilityLine(caps[2]), /命中文件受控读取 · 需要凭据 · 未配置/);
  assert.equal(c.readSourceCapabilities(undefined), null, '旧后端不报能力时是 null，页面显示 integration_pending');
});

test('g2 the official owner list rejects malformed logins instead of dropping them', () => {
  const c = contracts();
  const parsed = c.parseOwnerList('Acme-Official\nacme-official, acme-labs\nRob--W --- a_b  有空格');
  assert.deepEqual(parsed.owners, ['acme-labs', 'acme-official', 'rob--w'], '旧式登录名（双连字符）是真实账号');
  assert.deepEqual(parsed.invalid, ['---', 'a_b', '有空格']);
  assert.deepEqual(c.parseOwnerList('').owners, []);
  assert.deepEqual(c.parseOwnerList('x'.repeat(40)).invalid, ['x'.repeat(40)]);
  assert.deepEqual(c.parseOwnerList('x'.repeat(39)).owners, ['x'.repeat(39)]);
});

test('g2 source labels name GitHub explicitly and keep the three G0 sources', () => {
  const c = contracts();
  assert.equal(c.sourceLabel('passive_web'), '被动 Web 采集');
  assert.equal(c.sourceLabel('certificate_transparency'), '证书透明度');
  assert.equal(c.sourceLabel('web_archive'), '历史归档');
  assert.equal(c.sourceLabel('github_repositories'), 'GitHub 仓库元数据（匿名）');
  assert.equal(c.sourceReasonCode('github_link_rejected:Link 下一页指向 http://x'), 'github_link_rejected');
  assert.equal(c.sourceReasonCode('some other error'), '');
});

test('g2 the settings editor carries the owner list, refuses to save malformed owners and admits a missing capability list', async (t) => {
  const c = contracts();
  const composable = loadModule('src/features/assets/exposure/useExposureSettings.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return c;
    if (name === './api') return {};
    throw Error(`Unexpected import ${name}`);
  });
  const view = extra => ({
    version: 3, storedVersion: 3, saved: true, schemaVersion: 1, exposureSources: ['passive_web', 'github_repositories'],
    exposureMaxRequests: 2000, exposureMaxResponseBytes: 1000000, exposureMaxTotalBytes: 200000000,
    exposureMaxItems: 500, exposureConcurrency: 2, exposureTimeoutSeconds: 12, exposureCtPageSize: 200,
    exposureArchivePageSize: 200, exposureAllowPrivateDestinations: false, exposureAuthorizationDays: 0,
    bounds: { sources: ['passive_web', 'certificate_transparency', 'web_archive', 'github_repositories', 'github_code'] }, ...extra,
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
  const state = await mount({
    exposureGithubOfficialOwners: ['acme-official'],
    sourceCapabilities: [{ sourceKey: 'github_code', capability: 'default_branch_code_search', credential: 'required', state: 'unconfigured', integration: 'integration_pending', reason: 'IR-04' }],
  }, saved);
  assert.equal(state.ownerText.value, 'acme-official');
  assert.equal(state.capabilities.value.length, 1);
  assert.equal(state.capabilities.value[0].state, 'unconfigured');
  state.setOfficialOwners('acme-official\nbad_name');
  assert.deepEqual(state.ownerInvalid.value, ['bad_name']);
  assert.equal(await state.save(), false, '不合规的账号不能被悄悄丢掉后保存');
  assert.equal(saved.length, 0);
  assert.match(state.error.value, /bad_name/);
  state.setOfficialOwners('Acme-Labs\nacme-official');
  assert.equal(await state.save(), true);
  assert.deepEqual(saved[0].exposureGithubOfficialOwners, ['acme-labs', 'acme-official']);
  const old = await mount({}, []);
  assert.equal(old.capabilities.value, null, '旧后端不报能力：null，面板显示 integration_pending');
  assert.deepEqual(old.draft.value.exposureGithubOfficialOwners, []);
  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.match(panel, /integration_pending：后端未报告 GitHub 能力/);
  assert.match(panel, /没有凭据时代码搜索不发任何请求/);
  const surface = readSource('src/components/ExposureSurface.vue');
  assert.match(surface, /sourceStatusView\(source\)\.reason/);
});
