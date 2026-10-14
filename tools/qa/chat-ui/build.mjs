// Test-only bundle: builds the real application; never imported by production.
import {createRequire} from 'node:module';
import path from 'node:path';
import fs from 'node:fs';
import crypto from 'node:crypto';
const repo=path.resolve(process.env.OVIRAPTOR_REPO_ROOT || process.cwd());
const output=path.resolve(process.env.OVIRAPTOR_QA_OUTPUT || path.join(repo,'qa-artifacts/chat-ui'));
function manifest(){const files=['package.json','package-lock.json','vite.config.ts'];function walk(directory){for(const name of fs.readdirSync(path.join(repo,directory)).sort()){const relative=path.join(directory,name);if(fs.statSync(path.join(repo,relative)).isDirectory())walk(relative);else files.push(relative);}}walk('src');return files.sort().map(file=>crypto.createHash('sha256').update(fs.readFileSync(path.join(repo,file))).digest('hex')+'  '+file).join('\n')+'\n';}
fs.mkdirSync(output,{recursive:true});
const before=manifest();fs.writeFileSync(path.join(output,'source-before.sha256'),before);
const require=createRequire(path.join(repo,'package.json'));
const {build}=await import(require.resolve('vite'));
const vue=(await import(require.resolve('@vitejs/plugin-vue'))).default;
await build({root:repo,configFile:false,define:{'process.env.NODE_ENV':JSON.stringify('production')},plugins:[vue()],build:{outDir:path.join(output,'bundle'),emptyOutDir:true,assetsInlineLimit:10000000,lib:{entry:path.join(repo,'src/main.ts'),name:'OviraptorSyntheticQA',formats:['iife'],fileName:'app'},rollupOptions:{output:{inlineDynamicImports:true}}}});
fs.mkdirSync(output,{recursive:true});
const after=manifest();fs.writeFileSync(path.join(output,'source-after-build.sha256'),after);if(before!==after)throw Error('Application source changed during QA bundle build');
console.log('Synthetic browser QA bundle:',output);
