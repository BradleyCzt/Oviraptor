// Render the extracted business panels as real Vue SFCs; the parent harness stubs them.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const transpile = (source) => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
function loadPanel(name) {
  const filename = path.join(__dirname, '../src/features/sentinel/workbench', name);
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id: name });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: name,
    compilerOptions: { bindingMetadata: script.bindings },
  });
  assert.deepEqual(template.errors, []);
  const imports = (name) => {
    if (name === 'vue') return vue;
    if (name === '../presentation') {
      const labels = {};
      new Function('exports', transpile(fs.readFileSync(path.join(__dirname, '../src/features/sentinel/presentation.ts'), 'utf8')))(labels);
      return labels;
    }
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '../../../components/InlineConfirm.vue') return { default: () => null };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    throw new Error(`Unexpected panel import: ${name}`);
  };
  const scriptExports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(imports, scriptExports);
  new Function('require', 'exports', transpile(template.code))(imports, templateExports);
  return { ...scriptExports.default, render: templateExports.render };
}
const render = (component, props) => renderToString(vue.createSSRApp(component, props));

test('USD budget field explains the source gate and Web billing estimate', async () => {
  const panel = loadPanel('WorkbenchBudgetField.vue');
  const tr = (zh) => zh;
  const source = await render(panel, { mode: 'code', modelValue: 5, tr });
  assert.match(source, /value="5"/);
  assert.match(source, /主动清空/);
  assert.match(source, /不保证实际账单金额上限/);
  const web = await render(panel, { mode: 'web', modelValue: 5, tr });
  assert.match(web, /value="5"/);
  assert.match(web, /费用预算参考/);
  assert.match(web, /尚不能限制供应商的实际美元账单/);
  assert.doesNotMatch(web, /主动清空/);
});

test('grey-box fallback renders secret input as password and keeps explicit auth modes', async () => {
  const panel = loadPanel('WorkbenchGreyboxSettings.vue');
  const form = vue.reactive({ environment: 'staging', authProfileName: 'test', authType: 'header',
    authHeaderName: 'X-Test', authValue: 'secret' });
  const html = await render(panel, { form });
  assert.match(html, /type="password"/);
  assert.match(html, /X-Test/);
  assert.match(html, /自定义 Header/);
});

test('CI/CD panel retains independent thresholds and release switch', async () => {
  const panel = loadPanel('WorkbenchCicdSettings.vue');
  const html = await render(panel, { form: vue.reactive({ ciProvider: 'gitlab', repositoryUrl: 'https://example.test/repo',
    branch: 'main', commitSha: 'abcdef', buildId: 'b1', environment: 'test',
    maxCritical: 0, maxHigh: 5, blockRelease: true }) });
  assert.match(html, /GitLab CI/);
  assert.match(html, /超出阈值时阻断发布/);
  assert.match(html, /type="checkbox" checked/);
});

test('overview renders scoped recent tasks and aggregate usage', async () => {
  const panel = loadPanel('WorkbenchOverview.vue');
  const scans = [
    { id: 'a', scanType: 'code', taskName: 'Code only', status: 'completed_with_gaps', inputTokens: 3, outputTokens: 5, totalTokens: 8 },
    { id: 'b', scanType: 'web', taskName: 'Web only', status: 'scanning', inputTokens: 10, outputTokens: 20, totalTokens: 30 },
  ];
  const html = await render(panel, { scans, mode: 'code', modeLabel: (mode) => mode });
  assert.match(html, /Code only/);
  assert.doesNotMatch(html, /Web only/);
  assert.match(html, /30/);
  assert.match(html, /当前已加载任务的累计 Token/);
  assert.match(html, /已完成·存在覆盖缺口/);
  assert.doesNotMatch(html, /扫描中/);
  assert.match(html, /type="button"/);
  const fallback = await render(panel, { scans: [{ ...scans[0], taskName: '', projectName: '', status: '<unknown>' }], mode: 'code', modeLabel: (mode) => mode });
  assert.match(fallback, /未命名任务/);
  assert.match(fallback, /&lt;unknown&gt;/);
  assert.doesNotMatch(fallback, /已完成|<unknown>/);
});

test('Skills catalog renders escaped content and built-in clone action', async () => {
  const panel = loadPanel('WorkbenchSkillsCatalog.vue');
  const catalog = {
    skills: vue.ref([{ id: 1, name: '<unsafe>', description: 'Base', instructions: '<script>alert(1)</script>',
      builtin: true, enabled: true }]),
    skillBusy: vue.ref(false), refiningSkillId: vue.ref(), deleteSkill: vue.ref(),
    editingSkill: vue.ref(), showSkillEditor: vue.ref(false),
    skillForm: vue.reactive({ name: '', description: '', instructions: '', enabled: true }),
    skillPreview: vue.ref([]),
    refineSkill() {}, exportSkills() {}, importSkills() {}, importInternalSecSkills() {},
    editSkill() {}, cloneBuiltinSkill() {}, insertSkillTemplate() {}, saveSkill() {}, removeSkill() {},
  };
  const html = await render(panel, { catalog });
  assert.match(html, /复制后编辑/);
  assert.match(html, /&lt;unsafe&gt;/);
  assert.doesNotMatch(html, /<script>/);
});

test('skill selection hides built-ins for Web and preserves manual instructions for code review', async () => {
  const panel = loadPanel('WorkbenchSkillSelection.vue');
  const skills = [
    { id: 1, name: 'Default', enabled: true, builtin: true },
    { id: 2, name: 'Custom', enabled: true, builtin: false },
  ];
  const form = vue.reactive({ skillIds: [2], instruction: 'Focus on auth' });
  const collapsed = await render(panel, { mode: 'web', expanded: false, skills, form });
  assert.doesNotMatch(collapsed, /Custom|Focus on auth/);
  const expanded = await render(panel, { mode: 'web', expanded: true, skills, form });
  assert.match(expanded, /Custom/);
  assert.doesNotMatch(expanded, /Default/);
  const code = await render(panel, { mode: 'code', expanded: false, skills, form });
  assert.match(code, /Default/);
  assert.match(code, /Focus on auth/);
});

test('browser identities keep anonymous default and isolated session actions', async () => {
  const panel = loadPanel('WorkbenchBrowserAuth.vue');
  const form = vue.reactive({ projectId: 3, authSessionIds: [], authLoginUrl: '', authSessionName: '' });
  const controls = {
    authBusy: vue.ref(''), authSessions: vue.ref([]), showAuthSessionPicker: vue.ref(false),
    firstUrl: () => 'https://example.test/login', authStatusLabel: () => '有效', shortTime: () => '今天',
    onAuthSessionToggle() {}, openLogin() {}, finishLogin() {}, validateLogin() {}, removeLogin() {},
  };
  const anonymous = await render(panel, { form, controls });
  assert.match(anonymous, /默认按匿名身份扫描/);
  assert.doesNotMatch(anonymous, /auth-session-create/);
  controls.authSessions.value = [
    { id: 'valid', name: '<Admin>', status: 'valid', cookieCount: 1, headerCount: 0,
      storageCount: 0, capturedRequestCount: 1, scopeHosts: ['example.test'] },
    { id: 'capturing', name: 'Second', status: 'capturing', cookieCount: 0, headerCount: 0,
      storageCount: 0, capturedRequestCount: 0, scopeHosts: [] },
  ];
  controls.showAuthSessionPicker.value = true;
  const expanded = await render(panel, { form, controls });
  assert.match(expanded, /1 个可用/);
  assert.match(expanded, /https:\/\/example\.test\/login/);
  assert.match(expanded, /&lt;Admin&gt;/);
  assert.match(expanded, /我已登录，完成捕获/);
  assert.match(expanded, /disabled[^>]*type="checkbox"|type="checkbox"[^>]*disabled/);
  assert.doesNotMatch(expanded, /<Admin>/);
});

test('handoff and evidence follow-up show frozen submissions without granting execution', async () => {
  const panel = loadPanel('WorkbenchTaskNotices.vue');
  const handoffControls = { handoffConfirmed: vue.ref(false), pendingHandoff: vue.ref({
    taskName: 'Frozen task', maxBudgetUsd: 5, authSessionIds: [], urls: ['https://example.test'],
  }) };
  const followupControls = {
    pendingFollowupInput: vue.ref({ taskName: 'Frozen follow-up', scanMode: 'standard',
      maxBudgetUsd: 2, authSessionIds: [] }),
    followupSubmission: vue.ref(), followupRecoveryBusy: vue.ref(false),
    followupRecoveryError: vue.ref(''), showReleaseFollowup: vue.ref(false),
    restoreFollowupSubmission() {}, releaseFollowupSubmission() {},
  };
  const handoff = await render(panel, { handoff: { sourceScanId: 'old', closureId: 'receipt' },
    busy: false, handoffControls, followupControls });
  assert.match(handoff, /仅保存草稿/);
  assert.match(handoff, /Frozen task/);
  assert.match(handoff, /不复制原登录身份/);
  const followup = await render(panel, { followup: { sourceScanId: 'old', candidateId: 'candidate',
    candidateRevision: 2, missingEvidence: ['control'], proposal: { prerequisites: ['fresh auth'] } },
    busy: false, handoffControls, followupControls });
  assert.match(followup, /尚未授权执行/);
  assert.match(followup, /Frozen follow-up/);
  assert.match(followup, /fresh auth/);
  assert.doesNotMatch(followup, /Frozen task/);
});

test('Web policy card displays the selected plan and custom skills', async () => {
  const panel = loadPanel('WorkbenchWebPolicy.vue');
  const html = await render(panel, { scanMode: 'deep', summary: {
    contracts: 24, discovery: 3, verifiers: 3, chain: true,
  }, skillNames: ['系统默认', '<Custom>'] });
  assert.match(html, /DEEP/);
  assert.match(html, /≤ 24/);
  assert.match(html, /&lt;Custom&gt;/);
});
