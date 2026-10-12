const ts = require('typescript');
const vue = require('vue');
function compile(source, imports) {
  const exports = {};
  new Function('require', 'exports', ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText)(name => {
    if (name === 'vue') return vue;
    if (name in imports) return imports[name];
    throw new Error(`Unmocked tool approval fixture import: ${name}`);
  }, exports);
  return exports;
}
function node(kind, text = '') {
  return { kind, tagName: kind.toUpperCase(), text, children: [], props: {}, listeners: {}, value: '', type: 'text',
    getRootNode() { return global.document; },
    addEventListener(name, callback) { (this.listeners[name] ??= []).push(callback); },
    removeEventListener(name, callback) { this.listeners[name] = (this.listeners[name] ?? []).filter(c => c !== callback); },
    dispatchEvent(event) { for (const callback of this.listeners[event.type] ?? []) callback({ target: this, ...event }); return true; },
  };
}
function text(n) { return `${n.kind === 'comment' ? '' : n.text} ${n.children.map(text).join(' ')}`.replace(/\s+/g, ' ').trim(); }
function descendants(n, kind) { return [...(n.kind === kind ? [n] : []), ...n.children.flatMap(c => descendants(c, kind))]; }
function mountHost(t, component) {
  // Vue's actual vModelText directive uses activeElement. No directive is replaced.
  const priorDocument = global.document;
  const priorDocumentType = global.Document;
  const priorShadowRootType = global.ShadowRoot;
  // Mirror the document/root identity used by Vue runtime-dom beforeUpdate.
  global.Document = class FixtureDocument { constructor() { this.activeElement = null; } };
  global.ShadowRoot = class FixtureShadowRoot {};
  global.document = new global.Document();
  const renderer = vue.createRenderer({
    createElement: kind => node(kind), createText: value => node('text', value), createComment: value => node('comment', value),
    insert(child, parent, anchor) { if (child.parent) { const n = child.parent.children.indexOf(child); if (n >= 0) child.parent.children.splice(n, 1); }
      child.parent = parent; const n = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(n < 0 ? parent.children.length : n, 0, child); },
    remove(child) { const n = child.parent?.children.indexOf(child); if (n >= 0) child.parent.children.splice(n, 1); },
    setElementText(n, value) { n.text = value; n.children = []; }, setText(n, value) { n.text = value; },
    parentNode: n => n.parent, nextSibling: n => n.parent?.children[n.parent.children.indexOf(n) + 1],
    patchProp(n, key, _old, value) { n.props[key] = value; if (key === 'value') n.value = value ?? ''; if (key === 'type') n.type = value; },
  });
  const root = node('root'); const app = renderer.createApp(component); app.mount(root);
  t.after(() => {
    app.unmount();
    for (const [key, original] of [["document", priorDocument], ["Document", priorDocumentType], ["ShadowRoot", priorShadowRootType]]) {
      if (original === undefined) delete global[key]; else global[key] = original;
    }
  });
  return { root, app };
}
module.exports = { compile, node, text, descendants, mountHost };
