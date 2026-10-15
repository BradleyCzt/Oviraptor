// Enforce architectural edges against the real source tree, including lazy Vue imports.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const { parse } = require('@vue/compiler-sfc');

const root = path.join(__dirname, '..');
const sourceRoot = path.join(root, 'src');
const sourceFiles = directory => fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
  const filename = path.join(directory, entry.name);
  return entry.isDirectory() ? sourceFiles(filename) : /\.(ts|vue)$/.test(filename) ? [filename] : [];
});
const relative = filename => path.relative(root, filename).split(path.sep).join('/');
function resolveImport(filename, specifier) {
  const base = path.resolve(path.dirname(filename), specifier);
  return [base, `${base}.ts`, `${base}.vue`, path.join(base, 'index.ts')]
    .find(candidate => fs.existsSync(candidate) && fs.statSync(candidate).isFile());
}
function imports(filename) {
  let text = fs.readFileSync(filename, 'utf8');
  if (filename.endsWith('.vue')) {
    const { descriptor, errors } = parse(text, { filename });
    assert.deepEqual(errors, [], relative(filename));
    text = [descriptor.script?.content, descriptor.scriptSetup?.content].filter(Boolean).join('\n');
  }
  const ast = ts.createSourceFile(filename, text, ts.ScriptTarget.Latest, true);
  const edges = [];
  function visit(node) {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier) {
      edges.push({ specifier: node.moduleSpecifier.text, dynamic: false,
        typeOnly: Boolean(node.isTypeOnly || node.importClause?.isTypeOnly) });
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword &&
        node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0])) {
      edges.push({ specifier: node.arguments[0].text, dynamic: true, typeOnly: false });
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  return edges.map(edge => ({ ...edge,
    target: edge.specifier.startsWith('.') ? resolveImport(filename, edge.specifier) : undefined }));
}
const graph = new Map(sourceFiles(sourceRoot).map(filename => [filename, imports(filename)]));

test('every relative source import resolves after feature migration', () => {
  const missing = [];
  for (const [filename, edges] of graph) for (const edge of edges) {
    if (edge.specifier.startsWith('.') && !edge.target)
      missing.push(`${relative(filename)} -> ${edge.specifier}`);
  }
  assert.deepEqual(missing, []);
});

test('shared UI contains no business, application facade, or platform execution dependency', () => {
  const violations = [];
  for (const [filename, edges] of graph) {
    if (!relative(filename).startsWith('src/shared/')) continue;
    for (const edge of edges) {
      if (edge.target && !relative(edge.target).startsWith('src/shared/'))
        violations.push(`${relative(filename)} -> ${relative(edge.target)}`);
      if (edge.specifier.startsWith('@tauri-apps/')) violations.push(`${relative(filename)} -> ${edge.specifier}`);
    }
  }
  assert.deepEqual(violations, []);
});

test('Skills owns its catalog without importing Scanning or the aggregate application API', () => {
  const violations = [];
  for (const [filename, edges] of graph) {
    if (!relative(filename).startsWith('src/features/skills/')) continue;
    for (const edge of edges) {
      if (edge.target && (relative(edge.target).startsWith('src/features/sentinel/') ||
          relative(edge.target) === 'src/api.ts'))
        violations.push(`${relative(filename)} -> ${relative(edge.target)}`);
    }
  }
  assert.deepEqual(violations, []);
});

test('feature-owned type contracts never depend on the legacy aggregate type facade', () => {
  const violations = [];
  for (const [filename, edges] of graph) {
    if (!/^src\/(?:features\/[^/]+|shell)\/(?:types|catalogTypes)\.ts$/.test(relative(filename))) continue;
    for (const edge of edges) if (edge.target === path.join(sourceRoot, 'types.ts'))
      violations.push(relative(filename));
  }
  assert.deepEqual(violations, []);
  const facade = ts.createSourceFile('types.ts', fs.readFileSync(path.join(sourceRoot, 'types.ts'), 'utf8'), ts.ScriptTarget.Latest, true);
  assert.ok(facade.statements.length > 0);
  assert.ok(facade.statements.every(statement => ts.isExportDeclaration(statement) && statement.isTypeOnly),
    'legacy types.ts must stay a type-only compatibility facade');
});

test('internal features consume explicit owners instead of aggregate API or type compatibility exports', () => {
  const facades = new Set(['src/api.ts', 'src/types.ts']);
  const violations = [];
  for (const [filename, edges] of graph) {
    if (!relative(filename).startsWith('src/features/')) continue;
    for (const edge of edges) if (edge.target && facades.has(relative(edge.target)))
      violations.push(`${relative(filename)} -> ${relative(edge.target)}`);
  }
  assert.deepEqual(violations, []);
});

test('project and Worker core APIs do not import scanning execution or unrelated configuration owners', () => {
  const allowed = {
    'src/features/workspaces/api.ts': new Set(['src/features/workspaces/types.ts']),
    'src/features/runtime/api.ts': new Set(['src/features/runtime/types.ts', 'src/features/environment/types.ts']),
  };
  const violations = [];
  for (const [filename, edges] of graph) {
    const permitted = allowed[relative(filename)];
    if (!permitted) continue;
    for (const edge of edges) if (edge.target && !permitted.has(relative(edge.target)))
      violations.push(`${relative(filename)} -> ${relative(edge.target)}`);
  }
  assert.deepEqual(violations, []);
});

test('the startup eager graph cannot mount business workspaces before explicit navigation', () => {
  const visited = new Set();
  function visit(filename) {
    if (visited.has(filename)) return;
    visited.add(filename);
    for (const edge of graph.get(filename) || []) {
      if (edge.target && !edge.dynamic && !edge.typeOnly) visit(edge.target);
    }
  }
  visit(path.join(sourceRoot, 'App.vue'));
  assert.deepEqual([...visited].filter(filename =>
    /^src\/features\/(assets|sentinel|hackerone|skills)\/.*\.vue$/.test(relative(filename))).map(relative), []);
});
