const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const root = path.join(__dirname, '..', 'src', 'plugins');
const cache = new Map();
function load(name) {
  const file = path.resolve(root, name.endsWith('.ts') ? name : `${name}.ts`);
  if (!file.startsWith(`${root}${path.sep}`)) throw Error('Unexpected plugin import');
  if (cache.has(file)) return cache.get(file);
  const exports = {};
  cache.set(file, exports);
  const output = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  new Function('require', 'exports', output)(specifier => {
    if (!specifier.startsWith('./')) throw Error(`Unexpected plugin dependency ${specifier}`);
    return load(path.relative(root, path.resolve(path.dirname(file), specifier)));
  }, exports);
  return exports;
}
module.exports = load('index');
