<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  Activity,
  CheckCircle2,
  CircleAlert,
  Copy,
  Laptop,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Server,
  Square,
  Trash2,
} from "@lucide/vue";
import { api } from "../../../api";
import type { EnvironmentReport } from "../../environment/types";
import type { LocalWorkerSettings, RemoteWorkerNode, WorkerHealth } from "../types";
import type { SentinelScan } from "../../sentinel/types";

const emit = defineEmits<{
  message: [type: "success" | "error" | "info", text: string];
}>();

const local = ref<LocalWorkerSettings>();
const nodes = ref<RemoteWorkerNode[]>([]);
const loading = ref(true);
const savingLocal = ref(false);
const busyNode = ref<number>();
const form = ref({ name: "", endpoint: "", accessToken: "" });
const health = ref<Record<number, WorkerHealth>>({});
const environments = ref<Record<number, EnvironmentReport>>({});
const scans = ref<Record<number, SentinelScan[]>>({});

function dependencyStatus(report: EnvironmentReport, name: string) {
  return report.dependencies.find((item) => item.name.toLowerCase() === name.toLowerCase());
}

function message(type: "success" | "error" | "info", text: string) {
  emit("message", type, text);
}

async function load() {
  loading.value = true;
  try {
    [local.value, nodes.value] = await Promise.all([
      api.getLocalWorkerSettings(),
      api.listWorkerNodes(),
    ]);
  } catch (error) {
    message("error", String(error));
  } finally {
    loading.value = false;
  }
}

async function saveLocal() {
  if (!local.value) return;
  savingLocal.value = true;
  try {
    local.value = await api.saveLocalWorkerSettings({
      enabled: local.value.enabled,
      port: Number(local.value.port),
      accessToken: local.value.accessToken,
    });
    message("success", local.value.status);
  } catch (error) {
    message("error", String(error));
  } finally {
    savingLocal.value = false;
  }
}

async function copy(value: string, label: string) {
  try {
    await navigator.clipboard.writeText(value);
    message("success", `${label}已复制`);
  } catch {
    message("error", `无法复制${label}，请手动选择复制`);
  }
}

async function addNode() {
  try {
    const id = await api.saveWorkerNode({
      ...form.value,
      enabled: true,
    });
    form.value = { name: "", endpoint: "", accessToken: "" };
    await load();
    message("success", `Worker 节点 #${id} 已保存`);
  } catch (error) {
    message("error", String(error));
  }
}

async function removeNode(node: RemoteWorkerNode) {
  if (busyNode.value) return;
  busyNode.value = node.id;
  try {
    await api.deleteWorkerNode(node.id);
    await load();
    message("success", `${node.name} 已从主控端移除；Worker 本机数据未删除`);
  } catch (error) {
    message("error", String(error));
  } finally {
    busyNode.value = undefined;
  }
}

async function inspectNode(node: RemoteWorkerNode) {
  busyNode.value = node.id;
  try {
    const [nodeHealth, nodeEnvironment, nodeScans] = await Promise.all([
      api.testWorkerNode(node.id),
      api.getRemoteWorkerEnvironment(node.id),
      api.listRemoteWorkerScans(node.id),
    ]);
    health.value[node.id] = nodeHealth;
    environments.value[node.id] = nodeEnvironment;
    scans.value[node.id] = nodeScans;
    message("success", `${node.name} 连接正常，环境与任务状态已刷新`);
    nodes.value = await api.listWorkerNodes();
  } catch (error) {
    message("error", String(error));
    nodes.value = await api.listWorkerNodes().catch(() => nodes.value);
  } finally {
    busyNode.value = undefined;
  }
}

async function syncNode(node: RemoteWorkerNode) {
  busyNode.value = node.id;
  try {
    const count = await api.syncWorkerNode(node.id);
    message("success", `${node.name} 已读取 ${count} 条只读历史记录，可在任务中心的历史产物中查看；不会创建或恢复本地扫描任务，已删除或较旧轮次可能不显示。`);
    nodes.value = await api.listWorkerNodes();
  } catch (error) {
    message("error", String(error));
  } finally {
    busyNode.value = undefined;
  }
}

async function control(
  node: RemoteWorkerNode,
  scan: SentinelScan,
  action: "pause" | "resume" | "cancel",
) {
  busyNode.value = node.id;
  try {
    await api.controlRemoteWorkerScan({
      nodeId: node.id,
      scanId: scan.id,
      action,
    });
    scans.value[node.id] = await api.listRemoteWorkerScans(node.id);
    message("success", `远程任务已${action === "pause" ? "暂停" : action === "resume" ? "继续" : "取消"}`);
  } catch (error) {
    message("error", String(error));
  } finally {
    busyNode.value = undefined;
  }
}

onMounted(load);
</script>

<template>
  <div class="worker-settings">
    <section class="panel local-worker">
      <div class="panel-heading">
        <div>
          <span class="eyebrow">THIS COMPUTER</span>
          <h3>本机 Worker</h3>
          <p>M1 也可以作为主控端兼任 Worker；服务只监听 Tailscale 私网地址。关闭窗口后应用留在任务栏，选择“退出应用”才会停止 Worker。</p>
        </div>
        <span v-if="local" class="worker-state" :class="{ online: local.running }">
          {{ local.running ? "运行中" : "未运行" }}
        </span>
      </div>
      <div v-if="loading" class="worker-empty">正在读取 Worker 设置…</div>
      <template v-else-if="local">
        <div class="worker-form local-form">
          <label class="worker-toggle">
            <input v-model="local.enabled" type="checkbox" />
            <span></span>
            <b>启用本机 Worker</b>
          </label>
          <label>
            <span>监听端口</span>
            <input v-model.number="local.port" type="number" min="1024" max="65535" />
          </label>
          <button class="button primary" :disabled="savingLocal" @click="saveLocal">
            <RefreshCw v-if="savingLocal" :size="14" class="spinning" />
            {{ savingLocal ? "正在启动…" : "保存并重启 Worker" }}
          </button>
        </div>
        <p class="worker-status" :class="{ error: local.enabled && !local.running }">{{ local.status }}</p>
        <div class="connection-values">
          <label>
            <span>Tailnet 地址</span>
            <code>{{ local.endpoint || "尚未检测到 Tailscale IPv4" }}</code>
            <button v-if="local.endpoint" @click="copy(local.endpoint, '节点地址')"><Copy :size="13" /></button>
          </label>
          <label>
            <span>访问令牌</span>
            <code>{{ local.accessToken }}</code>
            <button @click="copy(local.accessToken, '访问令牌')"><Copy :size="13" /></button>
          </label>
        </div>
      </template>
    </section>

    <section class="panel worker-guide">
      <div class="panel-heading">
        <div><span class="eyebrow">CONNECTION</span><h3>连接方式：Tailscale 私网</h3></div>
      </div>
      <ol>
        <li><b>两台电脑安装 Tailscale</b><span>使用同一个账号登录。无需公网 IP、路由器端口映射、OpenSSH 或反向代理。</span></li>
        <li><b>Intel Mac / Windows 运行对应平台的 Oviraptor</b><span>进入本页完成环境检测，再开启本机 Worker，复制节点地址和令牌。</span></li>
        <li><b>M1 添加远程节点</b><span>粘贴地址和令牌后点“检测”；任务在 Worker 上继续运行，主控端可查看、控制并同步结果。</span></li>
      </ol>
      <details>
        <summary>远程 Worker 运行环境（源码分析器依赖单独准备）</summary>
        <div class="manual-grid">
          <article>
            <Laptop :size="18" />
            <div><strong>macOS（Apple / Intel）</strong><p>远程连接需要安装并登录 Tailscale；浏览器与 AST 工具需要 Node.js：</p><code>brew install node</code><p>资产采集和 Native HTTP 不需要 Python、Redis 或 Docker；可选源码分析器仍需要 Docker 和已批准的固定摘要镜像。</p></div>
          </article>
          <article>
            <Server :size="18" />
            <div><strong>Windows 11 x64</strong><p>自动安装仅准备 Tailscale 与 Node.js LTS。手动环境检查：</p><code>node --version; tailscale status</code><p>仅源码分析器需要另行准备 Docker 和固定摘要镜像。Native 工具能力须单独审批与隔离；当前 Windows Native HTTP artifact 写入适配器尚不可用。</p></div>
          </article>
        </div>
      </details>
      <p class="worker-login-note">无人值守提示：两端必须保持开机、已登录系统且 Tailscale 在线。普通 macOS Tailscale 在重启后、尚未登录用户前通常不会联网。</p>
    </section>

    <section class="panel add-worker">
      <div class="panel-heading">
        <div><span class="eyebrow">CONTROLLER</span><h3>添加远程 Worker</h3><p>这里填写 Worker 电脑上显示的两项连接信息。</p></div>
      </div>
      <div class="worker-form node-form">
        <label><span>名称</span><input v-model="form.name" placeholder="例如：家中 Intel Mac Pro" /></label>
        <label><span>节点地址</span><input v-model="form.endpoint" placeholder="http://100.x.x.x:19427" /></label>
        <label><span>访问令牌</span><input v-model="form.accessToken" type="password" placeholder="粘贴 Worker 令牌" /></label>
        <button class="button primary" @click="addNode"><Plus :size="14" />保存节点</button>
      </div>
    </section>

    <div class="worker-node-list">
      <article v-for="node in nodes" :key="node.id" class="panel worker-node">
        <header>
          <div class="node-title">
            <span class="node-icon"><Server :size="18" /></span>
            <div><strong>{{ node.name }}</strong><code>{{ node.endpoint }}</code></div>
          </div>
          <div class="node-actions">
            <button class="button ghost compact" :disabled="busyNode === node.id" @click="inspectNode(node)">
              <Activity :size="13" />检测
            </button>
            <button class="button secondary compact" :disabled="busyNode === node.id" @click="syncNode(node)">
              <RefreshCw :size="13" :class="{ spinning: busyNode === node.id }" />同步结果
            </button>
            <button class="icon-button danger" title="移除节点" :disabled="busyNode === node.id" @click="removeNode(node)"><Trash2 :size="14" /></button>
          </div>
        </header>
        <p v-if="node.lastError" class="node-error"><CircleAlert :size="13" />{{ node.lastError }}</p>
        <p v-else-if="node.lastSeenAt" class="node-ok"><CheckCircle2 :size="13" />最后连接：{{ node.lastSeenAt }}<template v-if="node.lastSyncAt"> · 最后同步：{{ node.lastSyncAt }}</template></p>
        <div v-if="health[node.id]" class="node-health">
          <span>{{ health[node.id].os }} · {{ health[node.id].arch }}</span>
          <span>运行 {{ health[node.id].runningScans }}</span>
          <span>完成 {{ health[node.id].completedScans }}</span>
          <span>{{ health[node.id].hostname }}</span>
        </div>
        <div v-if="environments[node.id]" class="remote-environment">
          <b>环境检测</b>
          <span v-for="dep in environments[node.id].dependencies" :key="dep.name" :class="{ missing: !dep.available }">
            {{ dep.name }} · {{ dep.available ? dep.version || "OK" : `缺失：${dep.detail}` }}
          </span>
          <span v-if="dependencyStatus(environments[node.id], 'docker')" :class="{ missing: !dependencyStatus(environments[node.id], 'docker')?.available }">
            Docker · {{ dependencyStatus(environments[node.id], 'docker')?.available ? dependencyStatus(environments[node.id], 'docker')?.version : dependencyStatus(environments[node.id], 'docker')?.detail }}
          </span>
        </div>
        <div v-if="scans[node.id]?.length" class="remote-scans">
          <div v-for="scan in scans[node.id].slice(0, 8)" :key="scan.id">
            <span><b>{{ scan.taskName || scan.projectName }}</b><small>{{ scan.status }} · {{ scan.currentCheckpoint }}</small></span>
            <span class="scan-actions">
              <button v-if="['queued','scanning'].includes(scan.status)" title="暂停" @click="control(node, scan, 'pause')"><Pause :size="12" /></button>
              <button v-if="['paused','failed','interrupted'].includes(scan.status)" title="继续" @click="control(node, scan, 'resume')"><Play :size="12" /></button>
              <button v-if="!['completed','cancelled'].includes(scan.status)" title="取消" @click="control(node, scan, 'cancel')"><Square :size="11" /></button>
            </span>
          </div>
        </div>
      </article>
      <div v-if="!loading && !nodes.length" class="panel worker-empty">尚未添加远程 Worker。先在 Intel Mac 或 Windows 上开启 Worker，再把地址和令牌填到上方。</div>
    </div>
  </div>
</template>

<style scoped>
.worker-settings{display:flex;flex-direction:column;gap:16px}.panel-heading{padding-bottom:14px}.worker-state{margin:2px 18px 0 0;padding:5px 9px;border-radius:7px;background:#f0f1f3;color:#75757b;font-size:9px;font-weight:700}.worker-state.online{background:#e7f7ef;color:#147951}.worker-form{display:grid;gap:10px;padding:0 18px 18px;align-items:end}.local-form{grid-template-columns:1fr 180px auto}.node-form{grid-template-columns:1fr 1.3fr 1.5fr auto}.worker-form label{display:flex;flex-direction:column;gap:5px}.worker-form label>span{font-size:9px;color:var(--muted)}.worker-form input{width:100%;height:38px;border:1px solid var(--line);border-radius:8px;background:#f8f8fa;padding:0 10px;outline:0}.worker-form input:focus{border-color:#007aff;box-shadow:0 0 0 3px #007aff1f}.worker-toggle{position:relative!important;display:flex!important;flex-direction:row!important;align-items:center!important;gap:9px!important;height:38px}.worker-toggle input{position:absolute;opacity:0}.worker-toggle>span{width:39px;height:23px;border-radius:13px;background:#c8c8cc;position:relative}.worker-toggle>span:after{content:"";position:absolute;left:3px;top:3px;width:17px;height:17px;border-radius:50%;background:#fff;transition:.18s;box-shadow:0 1px 4px #0003}.worker-toggle input:checked+span{background:#34c759}.worker-toggle input:checked+span:after{transform:translateX(16px)}.worker-toggle b{font-size:11px}.worker-status{margin:0 18px 12px;padding:9px 11px;border-radius:8px;background:#eef8f2;color:#17714f;font-size:10px}.worker-status.error{background:#fff4e8;color:#9a641d}.connection-values{display:grid;grid-template-columns:1fr 1fr;gap:10px;padding:0 18px 18px}.connection-values label{min-width:0;display:grid;grid-template-columns:1fr auto;gap:5px;padding:10px;border:1px solid var(--line);border-radius:9px;background:#fafafd}.connection-values span{grid-column:1/-1;font-size:9px;color:var(--muted)}.connection-values code{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:9px}.connection-values button,.scan-actions button{border:0;background:transparent;color:#617085;display:grid;place-items:center}.worker-guide ol{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin:0;padding:0 18px 18px;list-style:none;counter-reset:guide}.worker-guide li{counter-increment:guide;display:flex;flex-direction:column;gap:5px;padding:12px;border:1px solid var(--line);border-radius:9px;background:#fafafd}.worker-guide li:before{content:counter(guide);width:21px;height:21px;border-radius:50%;display:grid;place-items:center;background:#007aff;color:#fff;font-size:9px;font-weight:700}.worker-guide li b{font-size:10px}.worker-guide li span{font-size:9px;line-height:1.6;color:var(--muted)}.worker-guide details{margin:0 18px 18px;border-top:1px solid var(--line);padding-top:12px}.worker-guide summary{cursor:pointer;color:#176bcc;font-size:10px;font-weight:700}.worker-login-note{margin:0 18px 18px;padding:9px 11px;border-radius:8px;background:#fff7e8;color:#8c611e;font-size:9px;line-height:1.55}.manual-grid{display:grid;grid-template-columns:1fr 1fr;gap:10px;margin-top:11px}.manual-grid article{display:flex;align-items:flex-start;gap:10px;padding:12px;border:1px solid var(--line);border-radius:9px}.manual-grid article>div{display:flex;min-width:0;flex-direction:column;gap:5px}.manual-grid strong{font-size:10px}.manual-grid p{margin:0;color:var(--muted);font-size:9px;line-height:1.55}.manual-grid code{padding:7px;border-radius:6px;background:#151e2d;color:#dce6f4;white-space:pre-wrap;overflow-wrap:anywhere;font-size:8px}.worker-node-list{display:flex;flex-direction:column;gap:10px}.worker-node{padding:16px}.worker-node>header{display:flex;align-items:center;justify-content:space-between;gap:12px}.node-title{display:flex;min-width:0;align-items:center;gap:10px}.node-icon{width:36px;height:36px;border-radius:9px;background:#eaf3ff;color:#1970d7;display:grid;place-items:center}.node-title>div{min-width:0;display:flex;flex-direction:column;gap:4px}.node-title strong{font-size:12px}.node-title code{color:var(--muted);font-size:9px}.node-actions{display:flex;gap:7px}.node-ok,.node-error{display:flex;align-items:center;gap:5px;margin:12px 0 0;font-size:9px}.node-ok{color:#147951}.node-error{color:#bd4545}.node-health{display:flex;gap:7px;flex-wrap:wrap;margin-top:11px}.node-health span{padding:5px 7px;border-radius:6px;background:#f1f2f5;color:#596575;font-size:8px}.remote-environment{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:7px;margin-top:11px;padding-top:11px;border-top:1px solid var(--line)}.remote-environment b{grid-column:1/-1;font-size:9px}.remote-environment span{padding:7px;border-radius:6px;background:#eef8f2;color:#16704e;font-size:8px;overflow-wrap:anywhere}.remote-environment span.missing{background:#fff0f0;color:#b44343}.remote-scans{margin-top:11px;border-top:1px solid var(--line)}.remote-scans>div{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:8px 2px;border-bottom:1px solid #f0f1f4}.remote-scans>div>span:first-child{display:flex;min-width:0;flex-direction:column;gap:3px}.remote-scans b{font-size:9px}.remote-scans small{font-size:8px;color:var(--muted)}.scan-actions{display:flex;gap:4px}.scan-actions button{width:27px;height:27px;border:1px solid var(--line);border-radius:6px;background:#fff}.worker-empty{padding:32px;text-align:center;color:var(--muted);font-size:10px}@media(max-width:1000px){.node-form,.local-form{grid-template-columns:1fr 1fr}.node-form .button,.local-form .button{grid-column:1/-1}.worker-guide ol{grid-template-columns:1fr}.remote-environment{grid-template-columns:1fr 1fr}}@media(max-width:700px){.node-form,.local-form,.connection-values,.manual-grid,.remote-environment{grid-template-columns:1fr}.worker-node>header{align-items:flex-start;flex-direction:column}.node-actions{flex-wrap:wrap}}
</style>
