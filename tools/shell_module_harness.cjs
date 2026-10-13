const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const transpile = source => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
const load = (relative, imports = () => { throw Error('Unexpected shell import'); }) => {
  const exports = {};
  new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '..', relative), 'utf8')))(imports, exports);
  return exports;
};
const catalog = load('src/shell/moduleCatalog.ts');
const navigation = load('src/shell/navigation.ts', name => {
  if (name === './moduleCatalog') return catalog;
  throw Error(`Unexpected navigation import ${name}`);
});
module.exports = { catalog, navigation, load, transpile };
