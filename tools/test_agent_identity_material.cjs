const test = require('node:test');
const assert = require('node:assert/strict');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { loadWorkspaceModule } = require('./result_component_harness.cjs');

test('identity material preserves plaintext, escapes HTML and fences late task replies', async () => {
  const replies = [];
  const component = loadWorkspaceModule('src/features/sentinel/components/AgentIdentityMaterial.vue', {
    '../api': { sentinelApi: { getSentinelScanIdentityMaterial(project, scan) {
      return new Promise(resolve => replies.push({ project, scan, resolve }));
    } } },
  }).default;
  let bindings;
  const mounted = { ...component, setup(props, context) {
    bindings = component.setup(props, context);
    return () => null;
  } };
  const renderer = vue.createRenderer({
    createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
    insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
    parentNode: () => null, nextSibling: () => null,
  });
  const props = vue.reactive({ projectId: 1, scanId: 'A' });
  const app = renderer.createApp({ setup: () => () => vue.h(mounted, { ...props }) });
  const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };
  app.mount({});
  assert.equal(replies[0].scan, 'A');
  props.projectId = 2; props.scanId = 'B'; await flush();
  assert.deepEqual([replies[1].project, replies[1].scan], [2, 'B']);
  const current = [{ session: { id: 'identity-b', name: 'B', status: 'valid' },
    document: { Cookie: 'session=fixture-b', Authorization: 'Bearer fixture-b-token', password: '<script>fixture</script>' } }];
  replies[1].resolve(current); await flush();
  replies[0].resolve([{ session: { id: 'identity-a' }, document: { Cookie: 'wrong-task' } }]); await flush();
  assert.deepEqual(bindings.materials.value, current);
  const html = await renderToString(vue.createSSRApp({ render() {
    return component.render({}, [], props, vue.proxyRefs(bindings), {}, {});
  } }));
  assert.match(html, /session=fixture-b/);
  assert.match(html, /Bearer fixture-b-token/);
  assert.match(html, /&lt;script&gt;fixture&lt;\/script&gt;/);
  assert.doesNotMatch(html, /wrong-task|\[REDACTED\]|<script>/);
  props.scanId = 'C'; await flush();
  assert.deepEqual(bindings.materials.value, []);
  app.unmount();
  replies[2].resolve(current); await flush();
  assert.deepEqual(bindings.materials.value, []);
});
