<script setup lang="ts">
// §E2 — the exposure surface's own collection settings panel. Read, edit, save as a new
// version; nothing here starts a round, and a refused save stays visibly refused.
import { onMounted, ref, watch } from "vue";
import { Download, Save } from "@lucide/vue";
import { exposureApi } from "./api";
import { useExposureSettings, type SettingsNumberKey } from "./useExposureSettings";
import { buildImportRequest, capabilityLine, capabilityTone, exposureErrorText, sourceLabel } from "./contracts";
import { newOperationId } from "./incidents";

const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();

const settings = useExposureSettings({
  projectId: () => props.projectId,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  current, draft, history, loading, saving, error, conflict, available,
  bounds, versionLabel, allowedSources, sources, notice, isDefault, concurrencyNotice,
  capabilities, ownerText, ownerInvalid, documentText, documentInvalid,
  credentialRef, setCredentialRef,
  load, loadHistory, save, setNumber, setPrivateEgress, setOfficialOwners, setDocumentUrls, toggleSource,
} = settings;

const numbers: { key: SettingsNumberKey; label: string; unit: string }[] = [
  { key: "exposureMaxItems", label: "每轮工作项上限", unit: "项：网页按资产、证书/归档按域名" },
  { key: "exposureMaxRequests", label: "每轮请求上限", unit: "次（含失败尝试）" },
  { key: "exposureMaxResponseBytes", label: "单响应保留上限", unit: "字节" },
  { key: "exposureMaxTotalBytes", label: "每轮字节上限", unit: "字节" },
  { key: "exposureTimeoutSeconds", label: "请求超时", unit: "秒" },
  { key: "exposureCtPageSize", label: "证书透明度每域名名称上限", unit: "个，超出部分不记录" },
  { key: "exposureArchivePageSize", label: "归档索引每域名行数", unit: "行，只读一页" },
  { key: "exposureAuthorizationDays", label: "授权有效期", unit: "天（0=不设期限）" },
];

onMounted(() => {
  void load();
});
watch(() => props.projectId, () => void load());

const importChannel = ref("document_import");
const importOperator = ref("");
const importDeclaration = ref("");
const importDeclaredUrl = ref("");
const importBatchLabel = ref("");
const importPublic = ref(false);
const importPublicNote = ref("");
const importExtractedFrom = ref("");
const importFormat = ref("text");
const importFiles = ref<{ name: string; format: string; contentBase64: string }[]>([]);
const importBusy = ref(false);
const importNotice = ref("");
const importResult = ref("");

watch(importChannel, (channel) => {
  if (channel === "public_intel_import" && (importFormat.value === "text" || importFormat.value === "extracted_text")) {
    importFormat.value = "json";
  }
});

async function onImportFiles(event: Event): Promise<void> {
  const list = (event.target as HTMLInputElement).files;
  if (!list) return;
  const next: { name: string; format: string; contentBase64: string }[] = [];
  for (const file of Array.from(list)) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = "";
    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    next.push({ name: file.name, format: importFormat.value, contentBase64: btoa(binary) });
  }
  importFiles.value = next;
  importNotice.value = `已在本地读取 ${next.length} 个文件；声明的地址只记录，不会被访问`;
}

async function submitImport(): Promise<void> {
  if (!props.projectId) {
    importNotice.value = "请先选择工作空间";
    return;
  }
  const built = buildImportRequest({
    channel: importChannel.value,
    operator: importOperator.value,
    sourceDeclaration: importDeclaration.value,
    declaredUrl: importDeclaredUrl.value,
    batchLabel: importBatchLabel.value,
    publicMaterial: importPublic.value,
    publicMaterialNote: importPublicNote.value,
    extractedFrom: importExtractedFrom.value,
    files: importFiles.value,
  }, newOperationId());
  if (!built.ok) {
    importNotice.value = built.error;
    importResult.value = "";
    return;
  }
  importBusy.value = true;
  try {
    const result = await exposureApi.importExposureDocuments(props.projectId, built.payload);
    importResult.value = JSON.stringify(result).slice(0, 4000);
    importNotice.value = "导入已写入";
  } catch (reason) {
    importResult.value = "";
    importNotice.value = `导入未写入：${exposureErrorText(reason)}`;
  } finally {
    importBusy.value = false;
  }
}
</script>

<template>
  <section class="panel exposure-settings">
    <header>
      <div>
        <strong>暴露面采集配置</strong>
        <small>{{ versionLabel }}{{ isDefault ? " · 尚未保存" : "" }}</small>
      </div>
      <div>
        <button class="button ghost" :disabled="!projectId" @click="loadHistory"><Download :size="14" />查看版本历史</button>
        <button class="button primary" :disabled="!projectId || saving || !available" @click="save">
          <Save :size="14" />{{ saving ? "正在保存…" : "保存为新版本" }}
        </button>
      </div>
    </header>
    <p class="exposure-gap">{{ notice }}</p>
    <p class="exposure-gap">{{ concurrencyNotice }}</p>
      <p class="exposure-gap">并行度已从采集合同移除，执行器串行。</p>
    <p v-if="conflict" class="exposure-error"><strong>版本冲突</strong><span>{{ error }}</span></p>
    <p v-if="loading" class="exposure-empty">正在读取配置…</p>
    <div v-else-if="current" class="exposure-settings-grid">
      <div class="exposure-settings-sources">
        <span>启用来源</span>
        <label v-for="name in (allowedSources.length ? allowedSources : sources)" :key="name">
          <input type="checkbox" :checked="sources.includes(name)" @change="toggleSource(name)" />
          {{ sourceLabel(name) }}
        </label>
        <small>勾选来源只是选择数据源，不构成跨范围授权；新发现的名字仍要人工复核。GitHub 与指定文档 URL 默认不启用。</small>
      </div>
      <div class="exposure-settings-capabilities">
        <span>来源能力</span>
        <p v-if="capabilities === null" class="exposure-integration-pending">integration_pending：后端未报告 GitHub 能力与文档来源能力，不能假定可用</p>
        <ul v-else>
          <li
            v-for="cap in capabilities"
            :key="`${cap.sourceKey}:${cap.capability}`"
            :class="{ ready: capabilityTone(cap) === 'ok', unconfigured: capabilityTone(cap) === 'gap', 'exposure-integration-pending': capabilityTone(cap) === 'pending', unsupported: capabilityTone(cap) === 'unsupported' }"
          >
            {{ sourceLabel(cap.sourceKey) }} · {{ capabilityLine(cap) }}
          </li>
        </ul>
        <small>仓库元数据可用不代表代码搜索可用；没有凭据时代码搜索不发任何请求。文件导入与情报导入命令已注册（IR-14 已落地）。下面的表单只提交本机选择的文件。</small>
      </div>
      <form class="exposure-import" @submit.prevent="submitImport">
        <strong>导入文件或情报报告</strong>
        <small>只接收本机选择的文件。声明的 URL 只作为来源记录保存，应用不会打开它。PDF、Office、图片不能导入。</small>
        <label><span>渠道</span>
          <select v-model="importChannel">
            <option value="document_import">文件导入（文本 / JSON / CSV / 已提取文本）</option>
            <option value="public_intel_import">公开情报报告（仅 JSON / CSV）</option>
          </select>
        </label>
        <label><span>操作人</span><input v-model="importOperator" maxlength="80" /></label>
        <label class="wide"><span>来源说明</span><input v-model="importDeclaration" maxlength="500" placeholder="这份材料从哪来" /></label>
        <label class="wide"><span>声明的 URL（可选，只记录）</span><input v-model="importDeclaredUrl" placeholder="不会被访问" /></label>
        <label><span>批次名（可选）</span><input v-model="importBatchLabel" /></label>
        <label class="exposure-check"><input v-model="importPublic" type="checkbox" />声明为公开材料</label>
        <label v-if="importPublic"><span>公开理由</span><input v-model="importPublicNote" maxlength="200" /></label>
        <label v-if="importFormat === 'extracted_text'"><span>提取前的原格式</span><input v-model="importExtractedFrom" /></label>
        <label><span>格式</span>
          <select v-model="importFormat">
            <option v-if="importChannel === 'document_import'" value="text">text</option>
            <option value="json">json</option>
            <option value="csv">csv</option>
            <option v-if="importChannel === 'document_import'" value="extracted_text">extracted_text</option>
          </select>
        </label>
        <label><span>文件</span><input type="file" multiple @change="onImportFiles" /></label>
        <button type="submit" :disabled="importBusy">写入导入</button>
        <small v-if="importNotice">{{ importNotice }}</small>
        <pre v-if="importResult">{{ importResult }}</pre>
      </form>
      <label class="exposure-settings-owners">
        <span>GitHub 正式开源账号（人工维护，每行一个）</span>
        <textarea :value="ownerText" rows="3" placeholder="例如 acme-official" @input="setOfficialOwners(($event.target as HTMLTextAreaElement).value)" />
        <small v-if="ownerInvalid.length" class="exposure-error">不合规：{{ ownerInvalid.join("、") }}（不会保存）</small>
        <small v-else>这些账号下的仓库归为“公开材料”排除；名字像集团的其它账号不会因此被排除或认定为集团所有。</small>
      </label>
      <label class="exposure-settings-owners">
        <span>GitHub 凭据引用（钥匙串账户名，不是 token）</span>
        <input :value="credentialRef" placeholder="例如 exposure-github" @input="setCredentialRef(($event.target as HTMLInputElement).value)" />
        <small>token 只放在系统钥匙串，服务名 oviraptor.exposure.github。这里只保存引用名，不保存明文，也不会发到日志里。</small>
      </label>
      <label class="exposure-settings-documents">
        <span>指定公开文档 URL（每行一个，最多 20 个；“URL | 理由”表示人工声明为公开材料）</span>
        <textarea :value="documentText" rows="3" placeholder="https://www.example.org/standard.txt" @input="setDocumentUrls(($event.target as HTMLTextAreaElement).value)" />
        <small v-if="documentInvalid.length" class="exposure-error">不合规：{{ documentInvalid.join("、") }}（不会保存）</small>
        <small v-else>只在启用“指定公开文档 URL”来源时读取这些精确地址，每个一次、跳转不跟随、只限 80/443；PDF/Office/图片只记元数据。公开声明只排除“提及”线索，凭据形态值仍按风险处理。</small>
      </label>
      <label v-for="row in numbers" :key="row.key">
        <span>{{ row.label }}（{{ row.unit }}）</span>
        <input
          type="number"
          :value="draft[row.key]"
          :min="row.key === 'exposureAuthorizationDays' ? 0 : 1"
          @input="setNumber(row.key, ($event.target as HTMLInputElement).value)"
        />
      </label>
      <label class="exposure-settings-flag">
        <input
          type="checkbox"
          :checked="Boolean(draft.exposureAllowPrivateDestinations)"
          @change="setPrivateEgress(($event.target as HTMLInputElement).checked)"
        />
        <span>允许内网/回环地址出口</span>
        <small>默认关闭。开启只对之后的轮次生效；测试镜像必须在配置里显式批准，不能因为“选了来源”就访问。</small>
      </label>
      <p v-if="bounds.units" class="exposure-settings-bounds">
        上界：请求 {{ (bounds.maxRequests as number) }} 次/轮 · 单响应 {{ (bounds.maxResponseBytes as number) }} 字节 · 工作项 {{ (bounds.maxItems as number) }} 个/轮 · 超时 {{ (bounds.timeoutSeconds as number) }} 秒
      </p>
    </div>
    <div v-if="history?.items.length" class="exposure-settings-history">
      <strong>版本历史</strong>
      <div v-for="row in history.items" :key="row.version">
        <span>v{{ row.version }} · {{ row.createdAt }}</span>
        <small v-if="row.corrupt || !row.settings">该版本内容无法解析，原字节保留在历史中</small>
        <small v-else>
          工作项 {{ (row.settings.exposureMaxItems as number) ?? "?" }} · 请求 {{ (row.settings.exposureMaxRequests as number) ?? "?" }}
          · 来源 {{ (row.settings.exposureSources as string[])?.join("/") ?? "?" }}
        </small>
      </div>
      <p v-if="history.truncated" class="exposure-gap">只显示最近 {{ history.items.length }} 个版本，更早的版本未列出（不是完整历史）。</p>
    </div>
  </section>
</template>
