// §E2 — the settings editor's own honesty tests: defaults are labelled as defaults, a
// refused save stays unsaved, a version clash is not reported as applied, and every field the
// page can edit is a field the Rust contract actually accepts.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');

const flush = async () => { for (let i = 0; i < 20; i++) await vue.nextTick(); };
const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const loadModule = (relative, requireModule) => {
  const exports = {};
  new Function('require', 'exports', transpile(readSource(relative)))(requireModule, exports);
  return exports;
};

const view = (over = {}) => ({
  version: 3, storedVersion: 3, saved: true, schemaVersion: 1,
  exposureSources: ['passive_web', 'certificate_transparency'],
  exposureMaxRequests: 2000, exposureMaxResponseBytes: 1000000, exposureMaxTotalBytes: 200000000,
  exposureMaxItems: 500, exposureConcurrency: 2, exposureTimeoutSeconds: 12,
  exposureCtPageSize: 200, exposureArchivePageSize: 200,
  exposureAllowPrivateDestinations: false, exposureAuthorizationDays: 0,
  bounds: { maxRequests: 200000, maxResponseBytes: 67108864, maxTotalBytes: 8589934592, maxItems: 200000, concurrency: 8, timeoutSeconds: 120, pageSize: 1000, authorizationDays: 365, sources: ['passive_web', 'certificate_transparency', 'web_archive'], units: {} },
  ...over,
});

function mountSettings(t, api, projectId = () => 7) {
  const composable = loadModule('src/features/assets/exposure/useExposureSettings.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return loadModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });
    if (name === './api') return {};
    throw Error(`Unexpected settings import ${name}`);
  });
  const events = [];
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureSettings({ projectId, api, notify: (kind, text) => events.push([kind, text]) });
      return () => null;
    },
  });
  app.mount({});
  if (t && typeof t.after === 'function') t.after(() => app.unmount());
  return { state, events };
}

test('settings payload is validated instead of being shown as blanks', async (t) => {
  const h = mountSettings(t, { getExposureSettings: async () => view({ exposureSources: undefined }) });
  await h.state.load();
  await flush();
  assert.equal(h.state.current.value, null, '缺来源字段的响应不能被当成可用配置');
  assert.match(h.state.error.value, /配置缺少/);

  const h2 = mountSettings(t, { getExposureSettings: async () => ({ ...view(), version: undefined }) });
  await h2.state.load();
  await flush();
  assert.match(h2.state.error.value, /配置缺少版本号/);
});

test('an unregistered settings command is reported as missing, never as defaults in effect', async (t) => {
  const h = mountSettings(t, {});
  await h.state.load();
  await flush();
  assert.equal(h.state.available.value, false);
  assert.match(h.state.notice.value, /尚未注册/);
  assert.equal(h.state.current.value, null);
});

test('unsaved defaults are labelled as defaults and the draft mirrors them', async (t) => {
  const h = mountSettings(t, { getExposureSettings: async () => view({ saved: false, version: 1, storedVersion: 0 }) });
  await h.state.load();
  await flush();
  assert.equal(h.state.isDefault.value, true);
  assert.match(h.state.versionLabel.value, /默认配置（未保存，服务器版本 v0）/);
  assert.match(h.state.notice.value, /当前是代码默认值/);
  assert.equal(h.state.draft.value.exposureMaxItems, 500);
  assert.deepEqual(h.state.sources.value, ['passive_web', 'certificate_transparency']);
});

test('saving uses the version that was read, not whatever the editor holds', async (t) => {
  const sent = [];
  const h = mountSettings(t, {
    getExposureSettings: async () => view(),
    saveExposureSettings: (projectId, expectedVersion, settings) => {
      sent.push([projectId, expectedVersion, settings]);
      return Promise.resolve(4);
    },
  });
  await h.state.load();
  await flush();
  h.state.setNumber('exposureMaxItems', 120);
  h.state.toggleSource('web_archive');
  h.state.setPrivateEgress(true);
  assert.equal(h.state.dirty.value, true);
  assert.equal(await h.state.save(), true);
  assert.deepEqual(sent[0][0], 7);
  assert.equal(sent[0][1], 3, 'expectedVersion 必须是读到的 storedVersion');
  assert.equal(sent[0][2].exposureMaxItems, 120);
  assert.equal(sent[0][2].exposureAllowPrivateDestinations, true);
  assert.deepEqual(sent[0][2].exposureSources, ['passive_web', 'certificate_transparency', 'web_archive']);
  assert.deepEqual(h.events.filter(row => row[0] === 'success').map(row => row[1]), ['采集配置已保存为 v4，只对之后新建的轮次生效']);
  // 保存成功后重新读取，页面显示的是服务器版本而不是本地草稿。
  assert.equal(h.state.dirty.value, false);
});

test('a version clash keeps the draft and says nothing was saved', async (t) => {
  const h = mountSettings(t, {
    getExposureSettings: async () => view(),
    saveExposureSettings: () => Promise.reject(Error('[exposure_setting_conflict] 当前版本 v5，编辑器读到 v3，已拒绝覆盖')),
  });
  await h.state.load();
  await flush();
  h.state.setNumber('exposureMaxRequests', 9000);
  assert.equal(await h.state.save(), false);
  assert.equal(h.state.conflict.value, true);
  assert.equal(h.state.draft.value.exposureMaxRequests, 9000, '冲突时用户的编辑内容不能被丢掉');
  assert.match(h.state.error.value, /本次未保存/);
  assert.deepEqual(h.events.map(row => row[0]), ['error'], '冲突不能被报成成功');
  assert.doesNotMatch(h.events[0][1], /已保存/);
});

test('an invalid field keeps its backend reason and stays unsaved', async (t) => {
  const h = mountSettings(t, {
    getExposureSettings: async () => view(),
    saveExposureSettings: () => Promise.reject(Error('[exposure_setting_invalid] exposure_setting_invalid：exposureMaxRequests 必须在 1~200000 之间，收到 0')),
  });
  await h.state.load();
  await flush();
  h.state.setNumber('exposureMaxRequests', 0);
  assert.equal(await h.state.save(), false);
  assert.equal(h.state.conflict.value, false);
  assert.match(h.state.error.value, /exposureMaxRequests/);
  assert.match(h.state.error.value, /1~200000/);
  assert.equal(h.state.current.value.exposureMaxRequests, 2000, '服务器值未被改动');
});

test('every editable field is one the Rust contract accepts', () => {
  // 后端 from_json 的白名单；页面能改的字段必须在其中，否则会出现"改了但被拒绝"的假可用。
  const rust = readSource('src-tauri/src/commands/exposure/settings.rs');
  const declared = new Set([...rust.matchAll(/"([A-Za-z]+)",\n/g)].map(m => m[1]));
  const known = ['exposureSources', 'exposureMaxRequests', 'exposureMaxResponseBytes', 'exposureMaxTotalBytes',
    'exposureMaxItems', 'exposureConcurrency', 'exposureTimeoutSeconds', 'exposureCtPageSize',
    'exposureArchivePageSize', 'exposureAllowPrivateDestinations', 'exposureAuthorizationDays', 'version'];
  for (const key of known) assert.ok(rust.includes(`"${key}"`), `Rust 合同里必须声明 ${key}`);
  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  const edited = [...panel.matchAll(/key: "(exposure[A-Za-z]+)"/g)].map(m => m[1]);
  assert.ok(edited.length >= 8, `页面必须能编辑主要额度字段，实际 ${edited.length}`);
  for (const key of edited) {
    assert.ok(known.includes(key), `页面编辑了后端不认识的字段 ${key}`);
  }
  assert.ok(declared.size > 0);
});

// G0/P1-2/P1-3 — the history window says when it is not the whole history, a corrupt history
// row is kept as "unreadable" instead of crashing or becoming blanks, and a corrupt current
// configuration is an error on the page, never the defaults shown as if they were in effect.
test('setting history keeps the truncation fact and corrupt rows; corrupt settings are not defaults', async (t) => {
  const page = {
    items: [
      { version: 250, settings: null, corrupt: true, createdAt: '2026-10-08 22:00:00' },
      { version: 249, settings: { exposureMaxItems: 5, exposureMaxRequests: 9, exposureSources: ['passive_web'] }, corrupt: false, createdAt: '2026-10-08 21:59:00' },
    ],
    limit: 2,
    truncated: true,
  };
  const h = mountSettings(t, { getExposureSettings: async () => view(), listExposureSettingVersions: async () => page });
  await h.state.loadHistory();
  await flush();
  assert.equal(h.state.history.value.truncated, true, '截断事实必须原样保留');
  assert.equal(h.state.history.value.items[0].corrupt, true);
  assert.equal(h.state.history.value.items[0].settings, null);
  assert.equal(h.state.error.value, '');

  const broken = mountSettings(t, {
    getExposureSettings: async () => { throw Error('[exposure_settings_corrupt] 已保存的暴露面配置 v3 不是有效 JSON，拒绝按默认值继续'); },
  });
  await broken.state.load();
  await flush();
  assert.equal(broken.state.current.value, null, '损坏配置不能显示为默认值');
  assert.match(broken.state.error.value, /exposure_settings_corrupt|拒绝按默认值/);

  const panel = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.match(panel, /history\.truncated/, '面板必须显示历史被截断');
  assert.match(panel, /row\.corrupt \|\| !row\.settings/, '面板不能对无法解析的版本取字段');
});

// ───────────── FIX4 P1／R4（2026-10-09） ─────────────

function switchableSettings(t, failB = false) {
  const sent = [];
  const gates = [];
  let pid = 7;
  const api = {
    getExposureSettings: (projectId) => {
      if (projectId === 7) return Promise.resolve(view({ version: 3, storedVersion: 3 }));
      if (failB) return Promise.reject(Error('读取失败'));
      return new Promise(resolve => gates.push(() => resolve(view({ version: 9, storedVersion: 9, exposureAllowPrivateDestinations: false, exposureDocumentUrls: [] }))));
    },
    saveExposureSettings: (projectId, expectedVersion, settings) => { sent.push([projectId, expectedVersion, settings]); return Promise.resolve(expectedVersion + 1); },
  };
  const h = mountSettings(t, api, () => pid);
  return { ...h, sent, gates, switchTo: (next) => { pid = next; } };
}

test('FIX4-R4 while project B is still loading, project A\'s draft (private egress, document URLs) is not submitted to B', async (t) => {
  const h = switchableSettings(t);
  await h.state.load();
  await flush();
  h.state.setPrivateEgress(true);
  h.state.setDocumentUrls('https://a.example.test/public.pdf');
  h.switchTo(8);
  const loadingB = h.state.load();
  await flush();
  const saving = h.state.save(); // 修前会提交后等待 B 的读取，不 await
  await flush();
  assert.deepEqual(h.sent.map(row => [row[0], row[1], row[2].exposureAllowPrivateDestinations, row[2].exposureDocumentUrls]), [], '加载中不得提交');
  assert.equal(await saving, false);
  // B 读完后保存的是 B 自己的版本与配置。
  h.gates[0]();
  await loadingB;
  await flush();
  const second = h.state.save();
  await flush();
  h.gates[1](); // 保存成功后的重新读取
  assert.equal(await second, true);
  assert.deepEqual([h.sent[0][0], h.sent[0][1], h.sent[0][2].exposureAllowPrivateDestinations], [8, 9, false]);
});

test('FIX4-R4 after switching project without a successful read of B, the old draft cannot be saved onto B', async (t) => {
  const h = switchableSettings(t, true);
  await h.state.load();
  await flush();
  h.state.setPrivateEgress(true);
  h.switchTo(8);
  // 尚未触发读取 B（切换后、watch 之前）：
  assert.equal(await h.state.save(), false);
  await h.state.load(); // B 读取失败
  await flush();
  assert.equal(await h.state.save(), false);
  assert.deepEqual(h.sent.map(row => [row[0], row[1], row[2].exposureAllowPrivateDestinations]), [], '归属不一致不得提交');
});
