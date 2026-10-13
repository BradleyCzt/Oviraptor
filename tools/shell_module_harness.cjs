const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const transpile = source => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
const load = (relative, imports = () => { throw Error('Unexpected shell import'); }) => {
  const exports = {};
  new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '..', relative), 'utf8')))(imports, exports);
  return exports;
};
const pureModules = new Map();
function loadPure(relative) {
  const filename=path.resolve(__dirname,'..',relative);
  if(pureModules.has(filename))return pureModules.get(filename);
  const exports={};pureModules.set(filename,exports);
  new Function('require','exports',transpile(fs.readFileSync(filename,'utf8')))(name=>{
    if(!name.startsWith('.'))throw Error(`Unexpected nonlocal pure-module import ${name}`);
    let target=path.resolve(path.dirname(filename),name);
    target=fs.existsSync(`${target}.ts`)?`${target}.ts`:path.join(target,'index.ts');
    return loadPure(path.relative(path.join(__dirname,'..'),target));
  },exports);
  return exports;
}
const catalog = loadPure('src/shell/moduleCatalog.ts');
const navigation = loadPure('src/shell/navigation.ts');
const plugins = loadPure('src/plugins/index.ts');
module.exports = { catalog, navigation, plugins, load, loadPure, transpile };
