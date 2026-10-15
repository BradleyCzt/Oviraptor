// Run actual API modules and compatibility exports, substituting only Tauri IPC.
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const sourceRoot = path.join(__dirname, '../src');

function loadApiOwner(relative, invoke) {
  const cache = new Map();
  function load(filename) {
    if (!filename.startsWith(sourceRoot + path.sep) || !/Api\.ts$|\/api\.ts$/.test(filename))
      throw Error(`Unexpected API module: ${filename}`);
    if (cache.has(filename)) return cache.get(filename);
    const exports = {}; cache.set(filename, exports);
    const compiled = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
    }).outputText;
    new Function('require', 'exports', compiled)(specifier => {
      if (specifier === '@tauri-apps/api/core') return { invoke };
      if (!specifier.startsWith('.')) throw Error(`Unexpected API dependency: ${specifier}`);
      return load(path.resolve(path.dirname(filename), specifier + '.ts'));
    }, exports);
    return exports;
  }
  return load(path.join(sourceRoot, relative));
}
module.exports = { loadApiOwner };
