// Only this self-contained fixture listens; the production build never imports it.
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
const repo = fileURLToPath(new URL('../', import.meta.url));
const api = path.join(repo, 'src/features/environment/toolApprovalApi.ts');
const transport = path.join(repo, 'tools/tool_approval_dom/transport.mjs');
const mock = { name: 'tool-approval-dom-mock', enforce: 'pre',
  resolveId(source, importer) {
    if (importer?.split('?')[0] === api && source === '@tauri-apps/api/core') return transport;
    if (source.startsWith('@tauri-apps/')) throw new Error(`fixture_native_transport_forbidden:${source}`);
  },
};
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const server = await createServer({ configFile: false, root: path.join(repo, 'tools/tool_approval_dom'), publicDir: false,
    plugins: [mock, vue()], server: { host: '127.0.0.1', port: 1447, strictPort: true,
      fs: { allow: [repo] }, watch: { ignored: ['**/src-tauri/**'] } },
  });
  await server.listen();
  console.log('Tool approval DOM fixture: http://127.0.0.1:1447 — mock IPC only, no app/database/keys');
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, async () => { await server.close(); process.exit(0); });
}
