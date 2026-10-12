// §E3 — the cluster view's own honesty tests: a missing command is never shown as "nothing
// grouped", pages append instead of replacing, the filtered total is not a page length, member
// rows are read per cluster on demand, and a member that lost admission stops inheriting.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');

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

const cluster = (id, over = {}) => ({
  id,
  clusterKey: `content:hash${id}`,
  kind: 'content',
  representativeAssetId: id * 10,
  representativeLabel: `a${id}.corp.invalid`,
  representativeObservationId: id * 100,
  representativeRunId: 7,
  representativeObservedAt: '2026-10-08 01:00:00',
  memberCount: 2,
  directCount: 1,
  inheritedCount: 1,
  gapNote: '',
  ruleVersion: 'oviraptor.exposure.rules.v1',
  settingsVersion: 2,
  updatedAt: '2026-10-08 01:02:00',
  cacheTime: '2026-10-08 01:01:00',
  cacheExpiresAt: '2026-11-07 01:01:00',
  ...over,
});

const member = (assetId, over = {}) => ({
  assetId,
  label: `asset-${assetId}.corp.invalid`,
  membership: assetId === 10 ? 'representative' : 'member',
  evidenceSource: assetId === 10 ? 'direct' : 'inherited',
  inheritedFromAssetId: assetId === 10 ? null : 10,
  inheritedFromLabel: assetId === 10 ? '' : 'a10.corp.invalid',
  inheritedRunId: assetId === 10 ? 0 : 7,
  observationId: assetId === 10 ? 100 : null,
  ruleVersion: 'oviraptor.exposure.rules.v1',
  verifiedAt: assetId === 10 ? '2026-10-08 01:00:00' : '',
  admittedNow: true,
  origin: `https://asset-${assetId}.corp.invalid/`,
  contentHash: 'hash',
  ...over,
});

const page = (items, total, over = {}) => ({
  items, total, hasMore: false, nextCursor: '', pageSize: 20, ...over,
});

function loadClusters(api, initialProject = 7) {
  const contracts = loadModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });
  const composable = loadModule('src/features/assets/exposure/useExposureClusters.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts;
    if (name === './api') return {};
    throw Error(`Unexpected cluster composable import ${name}`);
  });
  const events = [];
  const scope = vue.reactive({ id: initialProject });
  let state;
  const renderer = vue.createRenderer({
    createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null,
  });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureClusters({
        projectId: () => scope.id,
        api,
        notify: (kind, text) => events.push([kind, text]),
      });
      return () => null;
    },
  });
  app.mount({});
  return { state, events, scope, contracts, composable, unmount: () => app.unmount() };
}

/// The real panel SFC, with only the exposure IPC wrapper faked.
function loadPanel(api, props = {}) {
  const contracts = loadModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });
  const composable = loadModule('src/features/assets/exposure/useExposureClusters.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts;
    if (name === './api') return { exposureApi: api };
    throw Error(`Unexpected cluster composable import ${name}`);
  });
  const file = 'src/features/assets/exposure/ExposureClustersPanel.vue';
  const filename = path.join(__dirname, '..', file);
  const { descriptor } = parse(readSource(file), { filename });
  const script = compileScript(descriptor, { id: 'ExposureClustersPanel.vue' });
  const compiled = compileTemplate({
    source: descriptor.template.content, filename, id: 'ExposureClustersPanel.vue',
    compilerOptions: { bindingMetadata: script.bindings },
  });
  assert.deepEqual(compiled.errors, [], '簇面板模板必须能编译');
  const requireModule = name => {
    if (name === 'vue') return vue;
    if (name === './api') return { exposureApi: api };
    if (name === './useExposureClusters') return composable;
    if (name === './contracts') return contracts;
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name.endsWith('.css')) return {};
    throw Error(`Unexpected cluster panel import: ${name}`);
  };
  const exports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, exports);
  new Function('require', 'exports', transpile(compiled.code))(requireModule, templateExports);
  const component = { ...exports.default, render: templateExports.render };
  return {
    component,
    renderSource: compiled.code,
    html: props.projectId === undefined && !Object.keys(props).length
      ? renderToString(vue.createSSRApp(component))
      : renderToString(vue.createSSRApp(component, props)),
  };
}

test('a missing cluster command is reported as missing, never as an empty graph', async (t) => {
  const h = loadClusters({});
  t.after(h.unmount);
  await h.state.load();
  await flush();
  assert.equal(h.state.available.value, false, '未注册时必须标记为不可用');
  assert.equal(h.state.clusters.value.length, 0);
  assert.match(h.state.notice.value, /尚未注册/);
  assert.doesNotMatch(h.state.notice.value, /还没有满足成簇条件/, '不能把读不到说成没有结果');
});

test('a failed first cluster read keeps its reason instead of claiming no clusters', async (t) => {
  const calls = [];
  const h = loadClusters({
    pageExposureClusters: async () => { calls.push(1); throw Error('database is locked'); },
  });
  t.after(h.unmount);
  const ok = await h.state.load();
  await flush();
  assert.equal(ok, false);
  assert.equal(calls.length, 1);
  assert.match(h.state.readError.value, /database is locked/);
  assert.equal(h.state.loaded.value, false, '第一次就失败时不能声称已经读到过');
  assert.match(h.state.notice.value, /簇读取失败/);
  assert.deepEqual(h.events.filter(row => row[0] === 'error').map(row => row[1]).flat(), [],
    '首次失败只标记状态，不额外弹窗说快照');
});

test('a refresh failure keeps the last snapshot and marks it stale', async (t) => {
  let fail = false;
  const h = loadClusters({
    pageExposureClusters: async () => {
      if (fail) throw Error('exposure_query_failed：读取中断');
      return page([cluster(1)], 3, { hasMore: true, nextCursor: 'c1' });
    },
  });
  t.after(h.unmount);
  await h.state.load();
  await flush();
  assert.equal(h.state.clusters.value.length, 1);
  fail = true;
  await h.state.load();
  await flush();
  assert.equal(h.state.stale.value, true);
  assert.equal(h.state.clusters.value.length, 1, '刷新失败必须保留旧快照');
  assert.match(h.state.notice.value, /上一次成功读取/);
  assert.ok(h.events.some(row => row[0] === 'error' && /快照/.test(row[1])), `${JSON.stringify(h.events)}`);
});

test('clusters page by appending, and the filtered total is not a page length', async (t) => {
  const asked = [];
  const h = loadClusters({
    pageExposureClusters: async (projectId, kind, search, cursor) => {
      asked.push({ projectId, kind, search, cursor });
      return cursor
        ? page([cluster(3), cluster(4)], 12, { hasMore: false, nextCursor: '' })
        : page([cluster(1), cluster(2)], 12, { hasMore: true, nextCursor: 'next-1' });
    },
  });
  t.after(h.unmount);
  await h.state.load();
  await flush();
  assert.deepEqual(asked[0], { projectId: 7, kind: '', search: '', cursor: '' });
  assert.equal(h.state.clusters.value.length, 2);
  assert.equal(h.state.total.value, 12, '总数必须是全过滤口径，不是本页长度');
  await h.state.loadMore();
  await flush();
  assert.equal(h.state.clusters.value.length, 4, '下一页必须追加而不是替换');
  assert.equal(h.state.total.value, 12);
  assert.deepEqual(asked[1].cursor, 'next-1');
  assert.equal(h.state.hasMore.value, false);
  await h.state.loadMore();
  await flush();
  assert.equal(h.state.clusters.value.length, 4, '没有更多时不再请求');
  assert.equal(asked.length, 2);
});

test('reloading after a filter change drops member rows read under the old predicate', async (t) => {
  const asked = [];
  const h = loadClusters({
    pageExposureClusters: async (projectId, kind, search, cursor) => {
      asked.push({ kind, search, cursor });
      return page([cluster(1)], 1);
    },
    pageExposureClusterMembers: async (projectId, clusterId) => {
      asked.push({ membersOf: clusterId });
      return page([member(10)], 2);
    },
  });
  t.after(h.unmount);
  await h.state.load();
  await h.state.toggle(1);
  await flush();
  assert.equal(h.state.members.value[1].items.length, 1);
  h.state.kindFilter.value = 'domain';
  await h.state.load();
  await flush();
  assert.deepEqual(h.state.members.value, {}, '换过滤条件后旧成员展开必须作废');
  assert.deepEqual(h.state.expanded.value, []);
  assert.deepEqual(
    asked.map(row => ('kind' in row ? `filter:${row.kind}` : 'members')),
    ['filter:', 'members', 'filter:domain'],
    `实际请求：${JSON.stringify(asked)}`,
  );
});

test('members are read one cluster at a time and refuse to look empty on error', async (t) => {
  const asked = [];
  const h = loadClusters({
    pageExposureClusters: async () => page([cluster(1, { id: 1 }), cluster(2, { id: 2 })], 2),
    pageExposureClusterMembers: async (projectId, clusterId, cursor) => {
      asked.push({ clusterId, cursor });
      if (clusterId === 2) throw Error('[exposure_cluster_not_found] 这个簇不属于当前工作空间');
      return cursor
        ? page([member(30)], 3, { hasMore: false, nextCursor: '' })
        : page([member(10), member(20)], 3, { hasMore: true, nextCursor: 'm20' });
    },
  });
  t.after(h.unmount);
  await h.state.load();
  await h.state.toggle(1);
  await flush();
  assert.deepEqual(asked, [{ clusterId: 1, cursor: '' }], '只读被展开的那个簇');
  assert.equal(h.state.members.value[1].items.length, 2);
  assert.equal(h.state.members.value[1].total, 3);
  await h.state.loadMembers(1, true);
  await flush();
  assert.equal(h.state.members.value[1].items.length, 3, '成员也要追加');
  assert.deepEqual(asked[1], { clusterId: 1, cursor: 'm20' });
  await h.state.toggle(2);
  await flush();
  assert.match(h.state.members.value[2].error, /不属于当前工作空间/, '被拒绝的成员读取要保留原因');
  assert.equal(h.state.members.value[2].items.length, 0);
  assert.ok(h.events.some(row => row[0] === 'error' && /不属于当前工作空间/.test(row[1])),
    `拒绝必须可见：${JSON.stringify(h.events)}`);
});

test('direct, inherited and revoked members get three different labels', async (t) => {
  const h = loadClusters({});
  t.after(h.unmount);
  assert.match(h.state.evidenceLabel(member(10)), /自带结论/);
  assert.match(h.state.evidenceLabel(member(20)), /继承自 a10\.corp\.invalid/);
  assert.match(h.state.evidenceLabel(member(20, { admittedNow: false })), /已失效：不再满足准入/);
  assert.match(h.state.evidenceLabel(member(20, { evidenceSource: 'stale_representative_revoked' })), /代表已撤权/);
  assert.match(h.state.cacheLabel(cluster(1)), /缓存于 2026-10-08 01:01:00，有效期至 2026-11-07/);
  assert.match(h.state.cacheLabel(cluster(1, { cacheTime: '', cacheExpiresAt: '' })), /尚无缓存结论/,
    '没有缓存时不能显示成空');
  assert.match(h.state.cacheLabel(cluster(1, { cacheExpiresAt: '' })), /未设期限/);
});

test('a slow cluster read from the previous project cannot land on the new one', async (t) => {
  const gate = {};
  const h = loadClusters({
    pageExposureClusters: async projectId => new Promise(resolve => {
      gate[projectId] = resolve;
    }),
  });
  t.after(h.unmount);
  void h.state.load();
  await flush();
  gate[7]({ items: [cluster(1)], total: 1, hasMore: false, nextCursor: '', pageSize: 20 });
  await flush();
  assert.equal(h.state.clusters.value.length, 1);
  h.scope.id = 8;
  void h.state.load();
  await flush();
  // The old, still-pending read is answered after the switch: it must be discarded.
  gate[7]({ items: [cluster(9)], total: 9, hasMore: false, nextCursor: '', pageSize: 20 });
  await flush();
  assert.equal(h.state.clusters.value.length, 0, '切项目后迟到的旧响应必须作废');
  gate[8]({ items: [cluster(5)], total: 1, hasMore: false, nextCursor: '', pageSize: 20 });
  await flush();
  assert.deepEqual(h.state.clusters.value.map(row => row.id), [5]);
});

test('the cluster panel is a real child of the page and cannot reach the collector', async () => {
  const asked = [];
  const api = {
    pageExposureClusters: async (projectId) => { asked.push(['clusters', projectId]); return page([cluster(1)], 1); },
    pageExposureClusterMembers: async () => page([member(10)], 1),
  };
  const panel = loadPanel(api);
  const html = await panel.html;
  assert.match(html, /内容簇与域名簇/);
  assert.match(html, /重新读取/);
  assert.match(html, /簇只是分组：既不授予归属，也不授予授权/);
  assert.doesNotMatch(html, /开始被动采集|停止采集|恢复该轮/, '簇视图里不能有采集控制');
  assert.deepEqual(asked, [], 'SSR 渲染不得发出 IPC');
  const source = readSource('src/features/assets/exposure/ExposureClustersPanel.vue');
  assert.doesNotMatch(source, /startExposureScan|resumeExposureScan|cancelExposureScan|saveExposureFinding/,
    '簇面板不得持有启动、恢复、停止或写线索的能力');
  assert.doesNotMatch(source, /from "\.\.\/\.\.\/api"/, '簇面板只能走暴露面自己的 IPC 包装');
  assert.match(source, /import \{ exposureApi \} from "\.\/api"/);
  const pageSource = readSource('src/components/ExposureSurface.vue');
  assert.match(pageSource, /ExposureClustersPanel v-if="view === 'clusters'"/, '簇视图必须挂在页签上');
  assert.match(panel.renderSource, /pageExposureClusters|load/, '模板要真的走分页读取');
});

test('the cluster list distinguishes a filtered miss from an empty workspace', async (t) => {
  const h = loadClusters({
    pageExposureClusters: async () => page([], 0),
  });
  t.after(h.unmount);
  await h.state.load();
  await flush();
  assert.match(h.state.notice.value, /还没有满足成簇条件的公开观察/);
  const filtered = loadClusters({
    pageExposureClusters: async () => page([], 5),
  });
  t.after(filtered.unmount);
  await filtered.state.load();
  await flush();
  assert.match(filtered.state.notice.value, /当前过滤没有命中，共 5 个簇/,
    '过滤为空时必须说明还有多少在别处');
});

test('payloads missing their paging fields are rejected, not rendered as blanks', async (t) => {
  for (const broken of [
    null,
    { total: 3, hasMore: false },
    { items: 'nope', total: 3, hasMore: false },
    { items: [], total: 'three', hasMore: false },
    { items: [], total: 0, hasMore: 'yes' },
  ]) {
    const h = loadClusters({ pageExposureClusters: async () => broken });
    t.after(h.unmount);
    const ok = await h.state.load();
    await flush();
    assert.equal(ok, false, `${JSON.stringify(broken)} 不能被当成读到了空列表`);
    assert.notEqual(h.state.readError.value, '', `无效分页结构必须报错：${JSON.stringify(broken)}`);
    assert.equal(h.state.loaded.value, false);
  }
});
