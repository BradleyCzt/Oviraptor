// Execute the public production API and its real feature leaf. Only IPC is substituted.
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const entry = path.join(__dirname, '../src/features/sentinel/api.ts');
const skills = path.join(__dirname, '../src/features/skills/catalogApi.ts');
const reconciliation = path.join(__dirname, '../src/features/sentinel/reconciliation/api.ts');

function loadSentinelApi(invoke) {
  if (typeof invoke !== 'function') throw Error('sentinel_api_harness_requires_invoke');
  const cache = new Map();
  function load(filename) {
    if (filename !== entry && filename !== reconciliation && filename !== skills) throw Error(`Unexpected API module: ${filename}`);
    if (cache.has(filename)) return cache.get(filename);
    const exports = {}; cache.set(filename, exports);
    const compiled = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
    }).outputText;
    new Function('require', 'exports', compiled)(name => {
      if (name === '@tauri-apps/api/core') return { invoke };
      if (filename === entry && name === '../skills/catalogApi') return load(skills);
      if (filename === entry && name === './reconciliation/api') return load(reconciliation);
      throw Error(`Unexpected API dependency: ${name}`);
    }, exports);
    return exports;
  }
  return load(entry).sentinelApi;
}

module.exports = { loadSentinelApi };
