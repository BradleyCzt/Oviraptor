// §E5 — the review panel's own honesty tests: a decision always carries the queue revision it
// read, a moved queue is not reported as saved, an unregistered command is not shown as an
// empty history, and "the retest did not fetch it" never enables the close button.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

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
const loadContracts = () => loadModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });

const evidence = (over = {}) => ({
  finding: { id: 41, category: 'source_code', title: '公开 .env', confidence: 75, status: 'new' },
  observations: [
    { id: 100, runId: 7, sourceKey: 'passive_web', kind: 'resource', url: 'https://x.invalid/.env', queriedAt: 't1' },
    { id: 101, runId: 8, sourceKey: 'passive_web', kind: 'resource', url: 'https://x.invalid/.env', queriedAt: 't2' },
  ],
  versions: [
    { revision: 1, runId: 7, contentHash: 'a', ruleVersion: 'r1', redactionVersion: 'd1', severity: 'high', manualStatus: '', reviewNeeded: false, createdAt: 't1' },
    { revision: 2, runId: 8, contentHash: 'b', ruleVersion: 'r1', redactionVersion: 'd1', severity: 'high', manualStatus: '', reviewNeeded: true, createdAt: 't2' },
  ],
  reviewEvents: [],
  reviewNeeded: { revision: 2, ruleVersion: 'r1' },
  ...over,
});

const remediation = (over = {}) => ({
  id: 9, findingId: 41, observationId: 101, runId: 7, actor: '蓝队-李某', status: 'awaiting_retest',
  note: '已删除镜像里的文件', createdAt: 't3', closedAt: '', retestRunIds: [8],
  findingTitle: '公开 .env', findingStatus: 'new', ...over,
});

function mountReview(t, api, ids = { projectId: 7, findingId: 41 }) {
  const contracts = loadContracts();
  const composable = loadModule('src/features/assets/exposure/useExposureReview.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts;
    if (name === './api') return {};
    throw Error(`Unexpected review import ${name}`);
  });
  const events = [];
  const scope = vue.reactive({ ...ids });
  let state;
  const renderer = vue.createRenderer({
    createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null,
  });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureReview({
        projectId: () => scope.projectId,
        findingId: () => scope.findingId,
        api,
        notify: (kind, text) => events.push([kind, text]),
      });
      return () => null;
    },
  });
  app.mount({});
  t.after(() => app.unmount());
  return { state, events, scope };
}

test('an unregistered review command is reported as missing, never as an empty history', async (t) => {
  const h = mountReview(t, { readExposureEvidence: async () => evidence() });
  const ok = await h.state.load();
  await flush();
  assert.equal(ok, false, '缺写入命令时必须算未就绪');
  assert.equal(h.state.available.value, false);
  assert.match(h.state.notice.value, /尚未注册/);
  assert.doesNotMatch(h.state.notice.value, /还没有人工复核事件/, '不能把读不到说成没有记录');
});

test('a decision always carries the revision that was actually read', async (t) => {
  const sent = [];
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: (...args) => { sent.push(args); return Promise.resolve(2); },
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  });
  await h.state.load();
  await flush();
  assert.equal(h.state.revision.value, 2, '队列版本必须取最大 revision');
  assert.equal(h.state.latestObservationId.value, 101);
  h.state.actor.value = '王凡';
  h.state.reason.value = '两份响应的字节不同，需重新判断';
  assert.equal(await h.state.review('reviewing'), true);
  await flush();
  assert.deepEqual(sent[0], [7, 41, 2, 'reviewing', '王凡', '两份响应的字节不同，需重新判断', [101], 0]);
  assert.ok(h.events.some(row => row[0] === 'success' && /r2/.test(row[1])), `${JSON.stringify(h.events)}`);
});

test('an anonymous or reason-less decision never reaches the backend', async (t) => {
  const sent = [];
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: (...args) => { sent.push(args); return Promise.resolve(1); },
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  });
  await h.state.load();
  await flush();
  assert.equal(await h.state.review('confirmed'), false, '没写操作者不能提交');
  h.state.actor.value = '王凡';
  assert.equal(await h.state.review('confirmed'), false, '没写依据不能提交');
  assert.equal(sent.length, 0, '本地校验失败时不能发出 IPC');
  assert.match(h.state.error.value, /依据/);
});

test('a queue that already moved is refused and labelled as not saved', async (t) => {
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: () => Promise.reject(Error('[exposure_review_conflict] 队列已前进到 r3，编辑器读到的是 r2，未写入任何结论')),
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  });
  await h.state.load();
  h.state.actor.value = '王凡';
  h.state.reason.value = '看过了';
  assert.equal(await h.state.review('confirmed'), false);
  await flush();
  assert.equal(h.state.conflict.value, true, '版本冲突要被单独标出来');
  assert.ok(h.events.some(row => row[0] === 'error' && /没有写入/.test(row[1])), `${JSON.stringify(h.events)}`);
  assert.equal(h.state.loaded.value, true, '冲突后旧快照仍是可信的，只是结论没写进去');
});

test('a remediation needs a note and keeps its evidence reference', async (t) => {
  const sent = [];
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: async () => 2,
    listExposureReviewTrail: async () => [],
    saveExposureRemediation: (...args) => { sent.push(args); return Promise.resolve(9); },
    pageExposureRemediations: async () => ({ items: [remediation()], total: 1, hasMore: false, nextCursor: '', pageSize: 20 }),
    readExposureRetestOutcome: async () => ({ basis: 'no_retest_round', canClose: false, note: '还没有复测轮次' }),
    exposureReviewTaxonomy: async () => [{ code: 'unknown', label: '证据不足', count: 0, meaning: '没有观察支撑' }],
  });
  await h.state.load();
  await flush();
  assert.equal(await h.state.recordRemediation(), false, '空白整改说明不该发请求');
  assert.equal(sent.length, 0);
  h.state.actor.value = '蓝队-李某';
  h.state.remediationNote.value = '已从镜像删除并轮换凭据';
  assert.equal(await h.state.recordRemediation(), true, `登记失败：${h.state.error.value} / ${JSON.stringify(h.events)}`);
  assert.deepEqual(sent[0], [7, 41, 101, 7, '蓝队-李某', '已从镜像删除并轮换凭据', 'fixing']);
  assert.ok(h.state.openRemediation.value, '登记后必须能看到待办整改');
  assert.equal(h.state.canClose.value, false, '没有复测依据时不能关闭');
  assert.match(h.state.outcomeLabel.value, /还没有复测/);
});

test('a refused retest says why and keeps the record untouched', async (t) => {
  const started = [];
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: async () => 2,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [remediation({ status: 'fixing' })], total: 1, hasMore: false, nextCursor: '', pageSize: 20 }),
    readExposureRetestOutcome: async () => ({ basis: 'no_retest_round', canClose: false }),
    startExposureRetest: (...args) => { started.push(args); return Promise.reject(Error('[exposure_retest_unauthorized] 当前工作空间没有准入资产，复测不能凭旧授权启动')); },
  });
  await h.state.load();
  await flush();
  assert.equal(await h.state.retest(), false);
  assert.deepEqual(started, [[7, 9]], '复测只能针对这一条整改');
  assert.ok(h.events.some(row => row[0] === 'error' && /准入资产/.test(row[1])), `${JSON.stringify(h.events)}`);
  assert.equal(h.state.openRemediation.value.status, 'fixing', '被拒绝的复测不能改写整改状态');
});

test('identical or unseen retest bytes never enable a close', async (t) => {
  for (const basis of ['retest_content_identical', 'retest_not_seen', 'retest_round_incomplete', 'retest_unreachable']) {
    const marked = [];
    const h = mountReview(t, {
      readExposureEvidence: async () => evidence(),
      reviewExposureFindingScoped: async () => 2,
      listExposureReviewTrail: async () => [],
      pageExposureRemediations: async () => ({ items: [remediation()], total: 1, hasMore: false, nextCursor: '', pageSize: 20 }),
      readExposureRetestOutcome: async () => ({ basis, canClose: false, note: '不能证明已经下线' }),
      markExposureRemediation: (...args) => { marked.push(args); return Promise.reject(Error(`[exposure_remediation_unclosed] ${basis}，不能凭感觉把整改记为已验证`)); },
    });
    await h.state.load();
    await flush();
    assert.equal(h.state.canClose.value, false, `${basis} 不该允许关闭`);
    h.state.actor.value = '蓝队';
    h.state.reason.value = '自认为好了';
    assert.equal(await h.state.mark('closed'), false, `${basis} 必须被后端拒绝`);
    assert.equal(marked.length, 1, '关闭尝试要交给后端判定，由它给出理由');
    assert.match(h.state.error.value, /不能凭感觉/, `${basis} 的拒绝理由必须留在页面上`);
  }
  const changed = [];
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: async () => 2,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [remediation()], total: 1, hasMore: false, nextCursor: '', pageSize: 20 }),
    readExposureRetestOutcome: async () => ({ basis: 'retest_content_changed', canClose: true, previousHash: 'aaaa', currentHash: 'bbbb' }),
    markExposureRemediation: (...args) => { changed.push(args); return Promise.resolve({ basis: 'retest_content_changed', canClose: true }); },
  });
  await h.state.load();
  await flush();
  assert.equal(h.state.canClose.value, true, '内容确实变了才允许关闭');
  h.state.actor.value = '蓝队';
  h.state.reason.value = '复测字节已变';
  assert.equal(await h.state.mark('closed'), true);
  assert.deepEqual(changed[0], [7, 9, 'closed', '蓝队', '复测字节已变']);
});

test('switching finding or project discards a slower older read', async (t) => {
  const gate = {};
  const h = mountReview(t, {
    readExposureEvidence: (findingId) => new Promise(resolve => { gate[findingId] = resolve; }),
    reviewExposureFindingScoped: async () => 1,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  }, { projectId: 7, findingId: 41 });
  void h.state.load();
  await flush();
  gate[41](evidence({ finding: { id: 41, category: 'source_code', title: '甲', confidence: 70, status: 'new' } }));
  await flush();
  assert.equal(h.state.loaded.value, true);
  h.scope.findingId = 42;
  void h.state.load();
  await flush();
  // The old read answers late: it must not overwrite the new selection's state.
  gate[41](evidence({ finding: { id: 41, category: 'source_code', title: '甲迟到', confidence: 70, status: 'confirmed' } }));
  await flush();
  assert.equal(h.state.evidence.value?.finding?.title, '甲', '迟到的旧线索响应不能覆盖当前选择');
  gate[42](evidence({ finding: { id: 42, category: 'document', title: '乙', confidence: 60, status: 'new' } }));
  await flush();
  assert.equal(h.state.evidence.value.finding.title, '乙');
});

test('the panel renders review state without any collection control', async () => {
  const calls = [];
  const api = {
    readExposureEvidence: async () => { calls.push(['evidence']); return evidence(); },
    reviewExposureFindingScoped: async () => { calls.push(['review']); return 2; },
    listExposureReviewTrail: async () => { calls.push(['trail']); return []; },
    pageExposureRemediations: async () => { calls.push(['remediations']); return { items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }; },
    readExposureRetestOutcome: async () => { calls.push(['outcome']); return { basis: 'no_retest_round', canClose: false }; },
    exposureReviewTaxonomy: async () => { calls.push(['taxonomy']); return []; },
    startExposureRetest: async () => { calls.push(['retest']); return 1; },
    markExposureRemediation: async () => { calls.push(['mark']); return {}; },
    saveExposureRemediation: async () => { calls.push(['save']); return 1; },
  };
  const contracts = loadContracts();
  const composable = loadModule('src/features/assets/exposure/useExposureReview.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contracts;
    if (name === './api') return { exposureApi: api };
    throw Error(`Unexpected import ${name}`);
  });
  const file = 'src/features/assets/exposure/ExposureReviewPanel.vue';
  const filename = path.join(__dirname, '..', file);
  const { descriptor } = parse(readSource(file), { filename });
  const script = compileScript(descriptor, { id: 'ExposureReviewPanel.vue' });
  const compiled = compileTemplate({
    source: descriptor.template.content, filename, id: 'ExposureReviewPanel.vue',
    compilerOptions: { bindingMetadata: script.bindings },
  });
  assert.deepEqual(compiled.errors, [], '复核面板模板必须能编译');
  const requireModule = name => {
    if (name === 'vue') return vue;
    if (name === './api') return { exposureApi: api };
    if (name === './useExposureReview') return composable;
    if (name === './contracts') return contracts;
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    throw Error(`Unexpected review panel import: ${name}`);
  };
  const exports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, exports);
  new Function('require', 'exports', transpile(compiled.code))(requireModule, templateExports);
  const { renderToString } = require('@vue/server-renderer');
  const html = await renderToString(vue.createSSRApp(
    { ...exports.default, render: templateExports.render },
    { projectId: 7, findingId: 41 },
  ));
  assert.match(html, /人工复核与整改复测/);
  assert.match(html, /确认暴露/);
  assert.match(html, /另起一轮复测/);
  assert.match(html, /复核轨迹（追加，不覆盖）/);
  assert.match(html, /disabled[^>]*><!---->确认暴露|确认暴露/, '未读取到队列版本前按钮必须禁用');
  assert.doesNotMatch(html, /开始被动采集|停止采集/, '复核面板不能带采集控制');
  assert.deepEqual(calls, [], 'SSR 阶段不得发出 IPC');
  const source = readSource(file);
  assert.doesNotMatch(source, /startExposureScan|resumeExposureScan|cancelExposureScan/,
    '复核面板不得持有启动、恢复或停止采集的能力');
  assert.match(source, /import \{ exposureApi \} from "\.\/api"/, '只能走暴露面自己的 IPC 包装');
  const pageSource = readSource('src/features/assets/exposure/ExposureSurface.vue');
  assert.match(pageSource, /ExposureReviewPanel/, '页面必须挂上复核整改视图');
  assert.match(pageSource, /只列出当前已读到的/, '选择器要说明它只覆盖已读到的一页');
});

// ── P1-1（2026-10-09）：整改/复测只作用于当前线索自己的整改记录 ──
test('P1-1 retest and close never act on another finding\'s remediation', async (t) => {
  const started = [];
  const marked = [];
  const outcomes = [];
  const other = remediation({ id: 5, findingId: 77, status: 'fixing', findingTitle: '别的线索' });
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: async () => 2,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [other], total: 1, hasMore: false, nextCursor: '', pageSize: 20 }),
    readExposureRetestOutcome: async (_p, id) => { outcomes.push(id); return { basis: 'retest_content_changed', canClose: true }; },
    startExposureRetest: async (_p, id) => { started.push(id); return 99; },
    markExposureRemediation: async (_p, id) => { marked.push(id); return { basis: 'retest_content_changed', canClose: true }; },
  });
  assert.equal(await h.state.load(), true);
  await flush();
  assert.equal(h.state.openRemediation.value, null, '别的线索的整改不能成为本线索的“当前整改”');
  assert.deepEqual(outcomes, [], '不能读取别的线索的复测结论');
  assert.equal(h.state.canClose.value, false, '别的线索的复测结果不能点亮本线索的关闭按钮');
  h.state.actor.value = '王凡';
  h.state.reason.value = '复测完成';
  assert.equal(await h.state.retest(), false);
  assert.equal(await h.state.mark('closed'), false);
  assert.deepEqual([started, marked], [[], []], '零请求打到别的线索的整改上');
});

test('P1-1 the finding\'s own remediation is found beyond the first remediation page', async (t) => {
  const started = [];
  const marked = [];
  const pages = [];
  const mine = remediation({ id: 9, findingId: 41, status: 'fixing' });
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: async () => 2,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async (_p, _s, _n, cursor) => {
      pages.push(cursor);
      return cursor === ''
        ? { items: [remediation({ id: 5, findingId: 77, status: 'fixing' })], total: 2, hasMore: true, nextCursor: 'c1', pageSize: 1 }
        : { items: [mine], total: 2, hasMore: false, nextCursor: '', pageSize: 1 };
    },
    readExposureRetestOutcome: async () => ({ basis: 'retest_content_changed', canClose: true }),
    startExposureRetest: async (_p, id) => { started.push(id); return 99; },
    markExposureRemediation: async (_p, id) => { marked.push(id); return { basis: 'retest_content_changed', canClose: true }; },
  });
  assert.equal(await h.state.load(), true);
  await flush();
  assert.deepEqual(pages, ['', 'c1'], '翻页直到读完');
  assert.equal(h.state.openRemediation.value?.id, 9);
  h.state.actor.value = '王凡';
  h.state.reason.value = '复测取回 404';
  assert.equal(await h.state.retest(), true);
  assert.equal(await h.state.mark('closed'), true);
  assert.deepEqual([started, marked], [[9], [9]], '只作用于本线索自己的整改 9');
});

// ───────────── FIX3（2026-10-09 17:29） ─────────────

test('FIX3-R2 a review carries the newest review event it read, so a same-version decision by someone else is a conflict', async (t) => {
  const sent = [];
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: (...args) => { sent.push(args); return Promise.resolve(2); },
    listExposureReviewTrail: async () => [
      { fromStatus: 'new', toStatus: 'confirmed', note: 'a', actor: '甲', expectedRevision: 2, references: [], createdAt: 't', eventId: 11 },
      { fromStatus: 'confirmed', toStatus: 'reviewing', note: 'b', actor: '乙', expectedRevision: 2, references: [], createdAt: 't', eventId: 17 },
    ],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  });
  await h.state.load();
  await flush();
  h.state.actor.value = '丙';
  h.state.reason.value = '复核同一版本';
  assert.equal(await h.state.review('confirmed'), true);
  assert.equal(sent[0][7], 17, '必须带上读到的最新复核事件 id');
});

test('FIX3-R2 the backend same-version conflict is shown as not saved', async (t) => {
  const h = mountReview(t, {
    readExposureEvidence: async () => evidence(),
    reviewExposureFindingScoped: () => Promise.reject(Error('exposure_review_conflict：同一版本 r2 已有别的复核结论（事件 18），编辑器读到的是事件 17，未写入任何结论')),
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  });
  await h.state.load();
  h.state.actor.value = '丙';
  h.state.reason.value = '复核同一版本';
  assert.equal(await h.state.review('dismissed'), false);
  assert.equal(h.state.conflict.value, true);
  assert.ok(h.events.some(row => row[0] === 'error' && /没有写入/.test(row[1])));
});

test('FIX3-R3 switching project while the evidence read is in flight drops the old project answer', async (t) => {
  let release;
  const h = mountReview(t, {
    readExposureEvidence: (findingId, projectId) => projectId === 7 ? new Promise(resolve => { release = () => resolve(evidence()); }) : new Promise(() => {}),
    reviewExposureFindingScoped: async () => 2,
    listExposureReviewTrail: async () => [],
    pageExposureRemediations: async () => ({ items: [], total: 0, hasMore: false, nextCursor: '', pageSize: 20 }),
  });
  const pending = h.state.load();
  await flush();
  h.scope.projectId = 8; // 同一个线索编号，在另一个项目里
  release();
  assert.equal(await pending, false);
  assert.equal(h.state.evidence.value, null, '旧项目的证据不能写进新项目的复核面板');
  assert.equal(h.state.revision.value, 0);
});
