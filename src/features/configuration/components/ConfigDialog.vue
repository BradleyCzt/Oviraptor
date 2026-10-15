<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import {
  Check,
  Database,
  GitBranch,
  KeyRound,
  Plus,
  RefreshCw,
  Settings2,
  Trash2,
} from "@lucide/vue";
import ModalShell from "../../../shared/components/ModalShell.vue";
import { configurationApi } from "../api";
import { scanningRulePacksApi } from "../../sentinel/rulePacksApi";
import type { ConfigProfile } from "../types";
import type { SecurityRulePack } from "../../sentinel/rulePacksTypes";
import { useI18n } from "../../../i18n";

const props = defineProps<{ profile?: ConfigProfile }>();
const emit = defineEmits<{ close: []; saved: [id: number] }>();
const { tr } = useI18n();

interface AgentModelProfile {
  id: string;
  name: string;
  llm: string;
  apiBase: string;
  apiKey: string;
  localApiKey: string;
  deployment: "cloud" | "local";
}

let agentProfileSequence = 0;
function newAgentModelProfile(name = ""): AgentModelProfile {
  agentProfileSequence += 1;
  return {
    id:
      globalThis.crypto?.randomUUID?.() ||
      `agent-model-${Date.now()}-${agentProfileSequence}`,
    name: name || tr("新模型", "New model"),
    llm: "",
    apiBase: "",
    apiKey: "",
    localApiKey: "",
    deployment: "cloud",
  };
}

const defaults = {
  modelProfiles: [] as AgentModelProfile[],
  activeModelProfileId: "",
  modelDeployment: "cloud",
  modelApiBase: "",
  modelApiKey: "",
  localApiKey: "",
  agentLocalFullPower: false,
  agentFrontendPacketMode: "balanced",
  agentFrontendPacketBudgetKb: 24,
  agentPromptAuditMode: "off",
  windowsRuntimeDirectory: "C:\\oviraptor\\runtime",
  scriptsDirectory: "",
  configPath: "",
  fofaEmail: "",
  fofaKey: "",
  hackerOneUsername: "",
  hackerOneToken: "",
  proxyUrl: "",
  noProxy: "127.0.0.1,localhost",
  agentBatchSize: 15,
  agentQuickScore: 30,
  agentStandardScore: 55,
  agentDeepScore: 80,
  agentQuickTimeout: 240,
  agentStandardTimeout: 600,
  agentDeepTimeout: 1200,
  agentQuickTokenLimit: 200000,
  agentStandardTokenLimit: 400000,
  agentDeepTokenLimit: 800000,
  agentQuickRequestLimit: 6,
  agentStandardRequestLimit: 14,
  agentDeepRequestLimit: 24,
  agentNoToolTurnLimit: 6,
  agentProxyEnabled: false,
  authorizedProxyPool: [] as string[],
  collectionMode: "all",
  fofaProfile: "professional",
  pageSize: 500,
  maxPages: 0,
  maxDerivedDomains: 200,
  interval: 6,
  collectionTimeout: 45,
  fullHistory: false,
  enableCidr24: false,
  includeWeakFingerprints: false,
  runRefine: true,
  runProbe: true,
  includeOther: true,
  includeWeak: false,
  priorityRate: 20,
  otherRate: 10,
  workers: 96,
  probeTimeout: 6,
  probeTotalTimeout: 8,
  probeRetries: 0,
  contentThreshold: 12,
  gamblingKeywords: ["在线赌博", "博彩平台", "真人视讯", "体育投注"],
  pornKeywords: ["色情网站", "成人网站", "成人视频", "情色直播"],
  negativeKeywords: ["打击赌博", "扫黄打非", "反诈", "公安", "法院"],
  replaceDefaultContentRules: false,
  semgrepRuleRepository: "",
  semgrepRuleReference: "",
  codeqlRuleRepository: "",
  codeqlRuleReference: "",
  nativeAnalyzers: { semgrep: { image: "" }, codeql: { image: "" } },
  owaspBenchmarkRepository: "",
  owaspBenchmarkReference: "",
};
const storedSettings = props.profile?.settings ?? {};
// The configuration API supplies current profiles and discards retired settings.
function normalizedAgentProfiles(): AgentModelProfile[] {
  const source = Array.isArray(storedSettings.modelProfiles)
    ? storedSettings.modelProfiles
    : null;
  if (source) {
    return source
      .filter((value: any) => value && typeof value === "object")
      .map((value: any, index: number) => ({
        id: String(value.id || `agent-model-${index + 1}`),
        name: String(value.name || `${tr("模型", "Model")} ${index + 1}`),
        llm: String(value.llm || ""),
        apiBase: String(value.apiBase || ""),
        apiKey: String(value.apiKey || ""),
        localApiKey: String(value.localApiKey || ""),
        deployment: value.deployment === "local" ? "local" : "cloud",
      }));
  }
  return [];
}
const initialAgentProfiles = normalizedAgentProfiles();
function initialNativeAnalyzers() {
  const stored = storedSettings.nativeAnalyzers as Record<string, { image?: unknown }> | undefined;
  return {
    semgrep: { image: String(stored?.semgrep?.image ?? "") },
    codeql: { image: String(stored?.codeql?.image ?? "") },
  };
}
const form = reactive({
  name: props.profile?.name ?? tr("新配置", "New profile"),
  description: props.profile?.description ?? "",
  isDefault: props.profile?.isDefault ?? false,
  settings: {
    ...defaults,
    ...storedSettings,
    modelProfiles: initialAgentProfiles,
    nativeAnalyzers: initialNativeAnalyzers(),
    activeModelProfileId:
      String(storedSettings.activeModelProfileId || "") ||
      initialAgentProfiles[0]?.id ||
      "",
  },
});
const tab = ref<
  | "accounts"
  | "runtime"
  | "agent"
  | "collection"
  | "probe"
  | "rules"
  | "securityRules"
>("accounts");
const busy = ref(false);
const error = ref("");
const rulePacks = ref<SecurityRulePack[]>([]);
const ruleBusy = ref("");
const ruleMessage = ref("");
const modelTestBusy = ref(false);
const modelTestStatus = ref<"idle" | "passed" | "failed">("idle");
const modelTestMessage = ref("");
const fofaTestBusy = ref(false);
const fofaTestStatus = ref<"idle" | "passed" | "failed">("idle");
const fofaTestMessage = ref("");
const modelTestKeys = reactive<Record<string, string>>({});
const initialModelTestKeys = initialAgentProfiles.reduce<Record<string, string>>(
  (keys, profile) => {
    keys[profile.id] = profileTestSignatureSource(profile);
    return keys;
  },
  {},
);
const ruleDefinitions = [
  {
    key: "semgrep-rules",
    name: "Semgrep Rules",
    engine: "semgrep",
    repositoryKey: "semgrepRuleRepository",
    referenceKey: "semgrepRuleReference",
    builtinRepository: "https://github.com/semgrep/semgrep-rules.git",
    builtinReference: "develop",
    description: "多语言模式匹配、污点分析和安全规则。",
  },
  {
    key: "codeql-queries",
    name: "CodeQL Queries",
    engine: "codeql",
    repositoryKey: "codeqlRuleRepository",
    referenceKey: "codeqlRuleReference",
    builtinRepository: "https://github.com/github/codeql.git",
    builtinReference: "main",
    description: "跨过程数据流、控制流和语义查询。",
  },
  {
    key: "owasp-benchmark",
    name: "OWASP Benchmark",
    engine: "benchmark",
    repositoryKey: "owaspBenchmarkRepository",
    referenceKey: "owaspBenchmarkReference",
    builtinRepository: "https://github.com/OWASP/Benchmark.git",
    builtinReference: "master",
    description: "用于规则覆盖率与误报回归测试，不直接作为漏洞规则执行。",
  },
] as const;
const keywordText = computed({
  get: () => (form.settings.gamblingKeywords ?? []).join("\n"),
  set: (value) => {
    form.settings.gamblingKeywords = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const pornText = computed({
  get: () => (form.settings.pornKeywords ?? []).join("\n"),
  set: (value) => {
    form.settings.pornKeywords = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const negativeText = computed({
  get: () => (form.settings.negativeKeywords ?? []).join("\n"),
  set: (value) => {
    form.settings.negativeKeywords = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const authorizedProxyText = computed({
  get: () => (form.settings.authorizedProxyPool ?? []).join("\n"),
  set: (value) => {
    form.settings.authorizedProxyPool = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const activeAgentProfile = computed(() =>
  form.settings.modelProfiles.find(
    (profile) => profile.id === form.settings.activeModelProfileId,
  ),
);
const localProfileActive = computed(
  () => activeAgentProfile.value?.deployment === "local",
);
const cloudPolicyVisible = computed(
  () => !activeAgentProfile.value || activeAgentProfile.value.deployment === "cloud",
);

function profileTestSignatureSource(profile: AgentModelProfile) {
  // The display name is local metadata and must not invalidate a connectivity test.
  return `${profile.id}\u0000${profile.llm}\u0000${profile.apiBase}\u0000${profile.apiKey}\u0000${profile.localApiKey}\u0000${profile.deployment}`;
}
function profileHasAcceptedTest(profile: AgentModelProfile) {
  const signature = profileTestSignatureSource(profile);
  return (
    signature === modelTestKeys[profile.id] ||
    signature === initialModelTestKeys[profile.id]
  );
}
async function testActiveAgentProfile() {
  const profile = activeAgentProfile.value;
  if (!profile) return;
  modelTestBusy.value = true;
  modelTestStatus.value = "idle";
  modelTestMessage.value = "";
  try {
    const result = await configurationApi.testModelProfile({
      llm: profile.llm.trim(),
      deployment: profile.deployment,
      apiBase: profile.apiBase.trim(),
      apiKey: profile.deployment === "local" ? profile.localApiKey : profile.apiKey,
    });
    // Keep the same raw signature format used by profileHasAcceptedTest.
    modelTestKeys[profile.id] = profileTestSignatureSource(profile);
    modelTestStatus.value = "passed";
    modelTestMessage.value = result.message;
  } catch (reason) {
    modelTestStatus.value = "failed";
    modelTestMessage.value = String(reason);
  } finally {
    modelTestBusy.value = false;
  }
}

async function testFofaApi() {
  fofaTestBusy.value = true;
  fofaTestStatus.value = "idle";
  fofaTestMessage.value = "";
  try {
    const result = await configurationApi.testFofaApi({
      key: String(form.settings.fofaKey || "").trim(),
      proxyUrl: String(form.settings.proxyUrl || "").trim(),
    });
    fofaTestStatus.value = "passed";
    const detail = [result.account, result.plan].filter(Boolean).join(" · ");
    fofaTestMessage.value = `${result.message}${detail ? ` · ${detail}` : ""}`;
  } catch (reason) {
    fofaTestStatus.value = "failed";
    fofaTestMessage.value = String(reason);
  } finally {
    fofaTestBusy.value = false;
  }
}

function activateAgentProfile(id: string) {
  form.settings.activeModelProfileId = id;
  syncActiveModelFields();
  const profile = activeAgentProfile.value;
  const accepted = profile ? profileHasAcceptedTest(profile) : false;
  modelTestStatus.value = accepted ? "passed" : "idle";
  modelTestMessage.value = accepted
    ? tr("该模型已通过连通性测试。", "This model passed the connectivity test.")
    : "";
}
function addAgentProfile() {
  const profile = newAgentModelProfile();
  form.settings.modelProfiles.push(profile);
  activateAgentProfile(profile.id);
}
function removeAgentProfile(id: string) {
  const index = form.settings.modelProfiles.findIndex(
    (profile) => profile.id === id,
  );
  if (index < 0) return;
  form.settings.modelProfiles.splice(index, 1);
  if (form.settings.activeModelProfileId === id) {
    form.settings.activeModelProfileId =
      form.settings.modelProfiles[Math.min(index, form.settings.modelProfiles.length - 1)]
        ?.id || "";
  }
  syncActiveModelFields();
  delete modelTestKeys[id];
  delete initialModelTestKeys[id];
  modelTestStatus.value =
    activeAgentProfile.value && profileHasAcceptedTest(activeAgentProfile.value)
      ? "passed"
      : "idle";
  modelTestMessage.value = "";
}

watch(
  () => activeAgentProfile.value?.deployment,
  (deployment) => {
    if (deployment !== "local" && form.settings.agentLocalFullPower) {
      form.settings.agentLocalFullPower = false;
    }
    if (deployment !== "local") form.settings.agentPromptAuditMode = "off";
  },
);
watch(
  () => (activeAgentProfile.value ? profileTestSignatureSource(activeAgentProfile.value) : ""),
  () => {
    const profile = activeAgentProfile.value;
    if (!profile) {
      modelTestStatus.value = "idle";
      modelTestMessage.value = "";
      return;
    }
    if (profileHasAcceptedTest(profile)) {
      modelTestStatus.value = "passed";
      modelTestMessage.value = tr(
        "该模型已通过连通性测试。",
        "This model passed the connectivity test.",
      );
      return;
    }
    if (modelTestStatus.value === "passed") {
      modelTestStatus.value = "idle";
      modelTestMessage.value = "";
    }
  },
);
/// §8.1: the flat neutral keys mirror the profile in use. The cloud key is never
/// mirrored into the local one, and vice versa, so a local endpoint cannot borrow
/// a credential that belongs to the cloud side.
function syncActiveModelFields() {
  const profile = activeAgentProfile.value;
  form.settings.modelDeployment = profile?.deployment === "local" ? "local" : "cloud";
  form.settings.modelApiBase = profile?.apiBase.trim() || "";
  form.settings.modelApiKey = profile?.deployment === "cloud" ? profile.apiKey.trim() : "";
  form.settings.localApiKey = profile?.deployment === "local" ? profile.localApiKey.trim() : "";
}

function configuredValue(key: string) {
  return String((form.settings as Record<string, unknown>)[key] ?? "").trim();
}
function rulePack(key: string) {
  return rulePacks.value.find((pack) => pack.key === key);
}
function ruleStatus(pack?: SecurityRulePack) {
  if (!pack || pack.status === "not_installed")
    return tr("未安装", "Not installed");
  if (pack.status === "syncing") return tr("同步中", "Syncing");
  if (pack.status === "ready") return tr("已就绪", "Ready");
  return tr("错误", "Error");
}
async function loadRulePacks() {
  try {
    rulePacks.value = await scanningRulePacksApi.listSecurityRulePacks();
  } catch (reason) {
    error.value = String(reason);
  }
}
async function persistRulePack(definition: (typeof ruleDefinitions)[number]) {
  return scanningRulePacksApi.saveSecurityRulePack({
    key: definition.key,
    name: definition.name,
    engine: definition.engine,
    repository: configuredValue(definition.repositoryKey),
    reference: configuredValue(definition.referenceKey),
    enabled: true,
  });
}
async function syncRulePack(definition: (typeof ruleDefinitions)[number]) {
  ruleBusy.value = definition.key;
  error.value = "";
  ruleMessage.value = "";
  try {
    const id = await persistRulePack(definition);
    await scanningRulePacksApi.syncSecurityRulePack(id);
    ruleMessage.value = `${definition.name} ${tr("已进入后台同步", "is syncing in the background")}`;
    for (let attempt = 0; attempt < 600; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 700));
      await loadRulePacks();
      const pack = rulePack(definition.key);
      if (pack?.status === "ready") {
        ruleMessage.value = `${definition.name} ${tr("同步完成", "synchronized")}：${pack.previousVersion || "首次安装"} -> ${pack.version || "—"}`;
        return;
      }
      if (pack?.status === "error") throw new Error(pack.error || tr("规则库同步失败", "Rule sync failed"));
    }
    throw new Error(tr("规则库仍在后台同步，请稍后回到配置中心查看", "Rule sync is still running; check Configuration later"));
  } catch (reason) {
    error.value = String(reason);
  } finally {
    ruleBusy.value = "";
  }
}

async function save() {
  busy.value = true;
  error.value = "";
  try {
    syncActiveModelFields();
    for (const [engine, config] of Object.entries(form.settings.nativeAnalyzers)) {
      config.image = config.image.trim();
      if (config.image && !/^[^\s@]+@sha256:[0-9a-fA-F]{64}$/.test(config.image)) {
        throw new Error(tr(`${engine} 镜像必须固定到 @sha256: 加 64 位摘要；不能使用 latest 等可变标签。`, `${engine} requires an image pinned with @sha256: and a 64-character digest.`));
      }
    }
    if (activeAgentProfile.value) {
      if (!profileHasAcceptedTest(activeAgentProfile.value)) {
        throw new Error(
          tr(
            "请先完成当前 Agent 模型连通性测试，测试通过后才能保存配置。",
            "Run and pass the current Agent model connectivity test before saving.",
          ),
        );
      }
    }
    await Promise.all(ruleDefinitions.map(persistRulePack));
    const id = await configurationApi.saveProfile({
      id: props.profile?.id && props.profile.id > 0 ? props.profile.id : undefined,
      ...form,
    });
    emit("saved", id);
  } catch (reason) {
    error.value = String(reason);
  } finally {
    busy.value = false;
  }
}
onMounted(loadRulePacks);
</script>

<template>
  <ModalShell
    :title="tr('配置方案', 'Configuration profile')"
    wide
    @close="$emit('close')"
  >
    <template #eyebrow
      ><span class="eyebrow"><Settings2 :size="14" /> PROFILE</span></template
    >
    <div class="profile-heading">
      <label class="field grow"
        ><span>{{ tr("配置名称", "Profile name") }}</span
        ><input v-model="form.name"
      /></label>
      <label class="check-field"
        ><input v-model="form.isDefault" type="checkbox" />
        {{ tr("设为默认", "Set as default") }}</label
      >
    </div>
    <label class="field"
      ><span>{{ tr("说明", "Description") }}</span
      ><input v-model="form.description"
    /></label>
    <p v-if="error" class="form-error profile-error-top">{{ error }}</p>
    <div class="tab-strip">
      <button
        v-for="item in [
          ['accounts', tr('账号与 API', 'Accounts & API')],
          ['runtime', tr('运行环境', 'Runtime')],
          ['agent', tr('Agent 策略', 'Agent policy')],
          ['collection', tr('采集', 'Collection')],
          ['probe', tr('探测', 'Probe')],
          ['rules', tr('内容规则', 'Content rules')],
          ['securityRules', tr('代码规则库', 'Code rules')],
        ]"
        :key="item[0]"
        :class="{ active: tab === item[0] }"
        @click="tab = item[0] as any"
      >
        {{ item[1] }}
      </button>
    </div>
    <div v-if="tab === 'accounts'" class="account-settings">
      <div class="account-settings-intro">
        <KeyRound :size="17" />
        <div>
          <strong>{{ tr("账号与 API", "Accounts and APIs") }}</strong>
          <span>{{
            tr(
              "模型和第三方平台凭据集中保存在当前配置方案中。切换 Agent 模型后，应用会直接为新任务注入配置，不需要修改或 source ~/.zshrc。",
              "Model and provider credentials live in this profile. Switching the Agent model injects configuration into new tasks directly; no ~/.zshrc edit or source command is required.",
            )
          }}</span>
        </div>
      </div>

      <section class="account-provider-section agent-model-section">
        <header>
          <div>
            <strong>{{ tr("模型与 Agent", "Model & agent") }}</strong>
            <span>{{
              tr(
                "保存多个 OpenAI 兼容模型，启用标记决定下一次扫描使用哪一个。",
                "Save multiple OpenAI-compatible models; the active marker controls the next scan.",
              )
            }}</span>
          </div>
          <button
            class="button ghost compact"
            type="button"
            @click="addAgentProfile"
          >
            <Plus :size="14" />{{ tr("添加模型", "Add model") }}
          </button>
        </header>
        <p v-if="error" class="form-error model-error-inline">{{ error }}</p>

        <div v-if="form.settings.modelProfiles.length" class="model-switch-list">
          <button
            v-for="profile in form.settings.modelProfiles"
            :key="profile.id"
            type="button"
            :class="{ active: profile.id === form.settings.activeModelProfileId }"
            :aria-pressed="profile.id === form.settings.activeModelProfileId"
            @click="activateAgentProfile(profile.id)"
          >
            <span>
              <strong>{{ profile.name || tr("未命名模型", "Unnamed model") }}</strong>
              <small>{{ profile.deployment === "local" ? tr("本地自建", "Self-hosted") : tr("云端", "Cloud") }} · {{ profile.llm || tr("尚未填写模型 ID", "Model ID not set") }}</small>
            </span>
            <em v-if="profile.id === form.settings.activeModelProfileId">
              <Check :size="12" />{{ tr("当前启用", "Active") }}
            </em>
            <em v-else>{{ tr("切换", "Switch") }}</em>
          </button>
        </div>
        <div v-else class="model-profile-empty">
          {{ tr("还没有模型配置。添加后即可在这里切换。", "No model profiles yet. Add one to enable switching.") }}
        </div>

        <div v-if="activeAgentProfile" class="model-profile-editor">
          <header>
            <div>
              <strong>{{ tr("编辑当前模型", "Edit active model") }}</strong>
              <span>{{ activeAgentProfile.name }}</span>
            </div>
            <div class="model-editor-actions">
              <button
                class="button danger compact model-delete-button"
                type="button"
                :title="tr('删除此模型', 'Delete this model')"
                @click="removeAgentProfile(activeAgentProfile.id)"
              >
                <Trash2 :size="14" />{{ tr("删除模型", "Delete model") }}
              </button>
              <button
                class="button secondary compact model-test-button"
                type="button"
                :disabled="modelTestBusy"
                @click="testActiveAgentProfile"
              >
                <RefreshCw :size="14" :class="{ spinning: modelTestBusy }" />
                {{ modelTestBusy ? tr("测试中", "Testing") : tr("测试模型", "Test model") }}
              </button>
            </div>
          </header>
          <div class="form-grid two">
            <label class="field"
              ><span>{{ tr("显示名称", "Display name") }}</span
              ><input v-model="activeAgentProfile.name" placeholder="DeepSeek V4"
            /></label>
            <label class="field"
              ><span>{{ activeAgentProfile.deployment === "local" ? tr("本地模型 ID", "Local model ID") : tr("云端模型 ID", "Cloud model ID") }}</span
              ><input v-model="activeAgentProfile.llm" placeholder="deepseek/deepseek-v4-pro"
            /></label>
            <label class="field span-two"
              ><span>{{ tr("部署类型", "Deployment") }}</span
              ><select
                v-model="activeAgentProfile.deployment"
              >
                <option value="cloud">{{ tr("云端 API", "Cloud API") }}</option>
                <option value="local">{{ tr("本地自建 OpenAI 兼容服务", "Self-hosted OpenAI-compatible") }}</option>
              </select>
              <small>{{
                activeAgentProfile.deployment === "local"
                  ? tr("只显示本地服务地址和模型 ID；云端凭据与费用策略不会参与本地任务。", "Only the local endpoint and model ID are shown; cloud credentials and cost policy do not apply to local tasks.")
                  : tr("只显示云端 API 凭据和云端发现预算；本地算力、Hook 与火力全开选项会隐藏。", "Only cloud API credentials and cloud discovery budgets are shown; local compute, hook, and full-power options are hidden.")
              }}</small>
            </label>
            <label v-if="activeAgentProfile.deployment === 'cloud'" class="field span-two"
              ><span>{{ tr("云端 API Base URL", "Cloud API Base URL") }}</span
              ><input
                v-model="activeAgentProfile.apiBase"
                placeholder="https://api.example.com/v1"
            /></label>
            <label v-if="activeAgentProfile.deployment === 'cloud'" class="field span-two"
              ><span>{{ tr("云端 API Key", "Cloud API key") }}</span
              ><input
                v-model="activeAgentProfile.apiKey"
                type="password"
                autocomplete="new-password"
                :placeholder="tr('请输入云端模型 API Key', 'Enter the cloud model API key')"
            /></label>
            <label v-if="activeAgentProfile.deployment === 'local'" class="field span-two"
              ><span>{{ tr("本地服务地址", "Local service endpoint") }}</span
              ><input v-model="activeAgentProfile.apiBase" placeholder="http://127.0.0.1:11434/v1"
            /></label>
            <label v-if="activeAgentProfile.deployment === 'local'" class="field span-two"
              ><span>{{ tr("本地服务 API Key（可选）", "Local service API key (optional)") }}</span
              ><input
                v-model="activeAgentProfile.localApiKey"
                type="password"
                autocomplete="new-password"
                :placeholder="tr('无鉴权服务保持留空', 'Leave blank when authentication is disabled')"
            /></label>
            <small v-if="activeAgentProfile.deployment === 'local'" class="helper span-two">{{ tr("留空时只向本机服务发送占位凭据 local；不会继承云端 API Key、LLM_API_KEY 或其他全局凭据。上下文窗口、KV Cache 和推理线程仍由本地服务管理。", "When blank, only the local placeholder credential is sent. Cloud API keys, LLM_API_KEY, and other global credentials are never inherited. Context size, KV cache, and inference threads remain controlled by the local service.") }}</small>
          </div>
          <p v-if="modelTestMessage" class="model-test-message" :class="modelTestStatus">
            <Check v-if="modelTestStatus === 'passed'" :size="13" />{{ modelTestMessage }}
          </p>
        </div>
      </section>

      <section class="account-provider-section">
        <header>
          <div>
            <strong>FOFA</strong>
            <span>{{ tr("资产检索账号", "Asset search account") }}</span>
          </div>
          <button class="button secondary compact" type="button" :disabled="fofaTestBusy" @click="testFofaApi">
            <RefreshCw :size="14" :class="{ spinning: fofaTestBusy }" />
            {{ fofaTestBusy ? tr("探测中", "Testing") : tr("探测 API", "Test API") }}
          </button>
        </header>
        <div class="form-grid two">
          <label class="field"
            ><span>FOFA account / email</span
            ><input v-model="form.settings.fofaEmail" autocomplete="username"
          /></label>
          <label class="field"
            ><span>FOFA key</span
            ><input
              v-model="form.settings.fofaKey"
              type="password"
              autocomplete="new-password"
              :placeholder="tr('仅保存在本机应用数据库', 'Stored only in the local app database')"
          /></label>
          <label class="field span-two"
            ><span>{{ tr("旧版 FOFA config 路径（Key 留空时使用）", "Legacy FOFA config path (used when key is blank)") }}</span
            ><input v-model="form.settings.configPath"
          /></label>
        </div>
        <p v-if="fofaTestMessage" class="model-test-message" :class="fofaTestStatus">
          <Check v-if="fofaTestStatus === 'passed'" :size="13" />{{ fofaTestMessage }}
        </p>
      </section>

      <section class="account-provider-section">
        <header>
          <div>
            <strong>HackerOne</strong>
            <span>{{ tr("漏洞平台 API 账号", "Vulnerability platform API account") }}</span>
          </div>
        </header>
        <div class="form-grid two">
          <label class="field"
            ><span>HackerOne API identifier</span
            ><input v-model="form.settings.hackerOneUsername" autocomplete="username"
          /></label>
          <label class="field"
            ><span>HackerOne API token</span
            ><input
              v-model="form.settings.hackerOneToken"
              type="password"
              autocomplete="new-password"
              :placeholder="tr('Token 值，仅保存在本机', 'Token value, local only')"
          /></label>
        </div>
      </section>
      <p class="helper account-secret-note">
        {{
          tr(
            "凭据仅保存在本机 SQLite；任务日志与任务快照不会写入明文 Key。",
            "Credentials remain in local SQLite; plaintext keys are excluded from task logs and snapshots.",
          )
        }}
      </p>
    </div>
    <div v-else-if="tab === 'runtime'" class="form-grid two">
      <label class="field"
        ><span>{{
          tr(
            "Windows runtime directory（建议）",
            "Windows runtime directory (recommended)",
          )
        }}</span
        ><input
          v-model="form.settings.windowsRuntimeDirectory"
          placeholder="C:\oviraptor\runtime"
      /></label>
      <label class="field"
        ><span>{{ tr("代理地址（Clash）", "Proxy URL (Clash)") }}</span
        ><input
          v-model="form.settings.proxyUrl"
          placeholder="http://127.0.0.1:7890"
      /></label>
      <label class="field"
        ><span>NO_PROXY</span
        ><input
          v-model="form.settings.noProxy"
          placeholder="127.0.0.1,localhost"
      /></label>
      <div class="proxy-pool-setting span-two">
        <label class="check-field"
          ><input v-model="form.settings.agentProxyEnabled" type="checkbox" />
          {{
            tr(
              "启用自有 / 已授权代理池",
              "Enable owned / authorized proxy pool",
            )
          }}</label
        >
        <label class="field"
          ><span>{{
            tr(
              "授权代理（每行一个，可用 国家标签|URL）",
              "Authorized proxies (one per line; COUNTRY|URL)",
            )
          }}</span
          ><textarea
            v-model="authorizedProxyText"
            rows="4"
            placeholder="CN|http://127.0.0.1:7890&#10;GLOBAL|socks5://127.0.0.1:1080"
          ></textarea>
        </label>
        <p class="helper">
          {{
            tr(
              "仅接受你拥有或明确获准使用的代理。Web 资产任务会按子批次轮换；模型 API 如需直连，请把其域名加入 NO_PROXY。",
              "Only use proxies you own or are explicitly allowed to use. Web batches rotate through the list; add model API hosts to NO_PROXY when they must connect directly.",
            )
          }}
        </p>
      </div>
      <p class="helper span-two">
        {{
          tr(
            "资产业务 worker 已内置为 Rust，无需 pandas、requests、openpyxl 或外部脚本目录。当前生产工具无需 Python 或 Redis；浏览器与 JavaScript AST 仍需要 Node.js。",
            "Asset workers are built in with Rust; pandas, requests, openpyxl and external worker directories are not required. Current production tools require neither Python nor Redis; browser and JavaScript AST helpers still require Node.js.",
          )
        }}
      </p>
      <p class="helper span-two">
        {{
          tr(
            "Windows 浏览器与 AST 工具需要 Node.js；远程 Worker 连接需要 Tailscale。",
            "On Windows, browser and AST tools need Node.js; remote Worker connections need Tailscale.",
          )
        }}
      </p>
    </div>
    <div v-else-if="tab === 'agent'" class="agent-policy-settings">
      <div class="skill-format-guide">
        <strong>{{ tr("多 Agent 运行环境", "Multi-agent runtime") }}</strong
        ><span>{{
          localProfileActive
            ? tr("当前使用本地模型：云端费用和请求预算已隐藏，运行时会根据模型参数规模自动限制并发、输出长度和首轮等待时间。", "A local model is active. Cloud cost and request budgets are hidden; runtime concurrency, output length, and first-turn wait are adapted to model size automatically.")
            : tr("当前使用云端 API：Native Root 协调专家任务，授权与预算由运行时校验；WAF、验证码与持续限流仍会停止目标请求。", "A cloud API is active. Native Root coordinates specialist tasks with runtime authorization and budget checks; WAF, CAPTCHA, and sustained rate limiting stop target requests.")
        }}</span>
      </div>
      <section class="agent-packet-policy">
        <div>
          <strong>{{ tr("SRC 专项能力已内置", "SRC specialist capabilities are built in") }}</strong>
          <span>{{ tr("原始 HTTP、有界竞争、按契约受控写入、攻击链关联和每任务 HTTP OAST 会自动准备，不再需要额外开关。目标无法回连当前工作站时，任务会只把 OAST 类别标记为网络不可达。", "Raw HTTP, bounded races, contract-gated writes, attack-chain correlation, and per-task HTTP OAST are prepared automatically. If the target cannot route back to this workstation, only OAST-dependent checks are marked unreachable.") }}</span>
        </div>
        <p class="helper">{{ tr("内置适配器由原生运行时执行；写请求仍必须包含清理、回滚、次数上限和业务不变量，不可逆操作继续禁止。", "The adapter uses the native runtime. Write contracts still require cleanup, rollback, attempt limits, and a business invariant; irreversible actions remain disabled.") }}</p>
      </section>
      <div v-if="localProfileActive" class="agent-governance-grid">
        <article class="agent-governance-card">
          <div>
            <strong>{{ tr("本地模型火力全开", "Local model full power") }}</strong>
            <span>{{ tr("仅当当前启用模型标记为“本地自建”时生效。", "Only applies when the active model is marked self-hosted.") }}</span>
          </div>
          <label class="switch-control">
            <input
              v-model="form.settings.agentLocalFullPower"
              type="checkbox"
              :disabled="!activeAgentProfile || activeAgentProfile.deployment !== 'local'"
            />
            <span></span>
          </label>
          <p>{{
            tr(
              "开启后使用本地模型算力，但不再强制 deep 或放宽到数小时；目标仍按证据自适应分流，并在模型、Token、工具和日志持续无进展时自动结束当前 URL。",
              "Uses local model capacity without forcing deep mode or multi-hour limits. Targets keep evidence-based adaptive routing and stop the current URL when model, token, tool, and log progress stalls.",
            )
          }}</p>
        </article>
        <article class="agent-governance-card">
          <div>
            <strong>{{ tr("本地模型 Hook 与提示词审计", "Local model hook and prompt audit") }}</strong>
            <span>{{ tr("Token 始终统计；这里控制新任务保存多少请求内容。", "Tokens are always counted; this controls how much request content new tasks retain.") }}</span>
          </div>
          <label class="field">
            <select
              v-model="form.settings.agentPromptAuditMode"
            >
              <option value="off">{{ tr("仅统计 Token", "Token counts only") }}</option>
              <option value="metadata">{{ tr("请求元数据与哈希", "Request metadata and hashes") }}</option>
              <option value="full">{{ tr("保存本机完整模型请求", "Store full model requests locally") }}</option>
            </select>
          </label>
          <p>{{
            tr(
              "本地自建模型通过回环 Hook 采集。全文档位会在本机记录原生 Agent 最终组装的 system / developer / user / tool schema、请求内容与响应摘要，仅对超长字段做截断；云模型仍只保留 Oviraptor 生成的 instruction。",
              "Self-hosted models are captured through a loopback hook. Full mode stores the native Agent's assembled system, developer, user, tool schema, request content, and response summary on this device, truncating only oversized fields. Cloud models retain only the Oviraptor-generated instruction.",
            )
          }}</p>
        </article>
      </div>
      <section v-if="localProfileActive" class="agent-packet-policy local-model-policy-note">
        <div><strong>{{ tr("本地模型自动资源策略", "Automatic local-model resource policy") }}</strong><span>{{ tr("从模型名称识别参数规模：20B–59B 严格串行、单次最多输出 4096 Token、首轮最长等待 1200 秒；60B 以上最多输出 3072 Token、等待 1800 秒；13B 以下仅在火力全开时允许最多 2 个上游请求。停止或暂停任务会立即断开仍在执行的本地推理，避免旧任务继续占用 CPU。", "Model size is inferred from its name: 20B–59B runs serially with at most 4,096 output tokens and a 1,200-second first-turn window; 60B+ uses 3,072 tokens and 1,800 seconds; below 13B allows at most two upstream requests only in full-power mode. Stopping or pausing a task immediately disconnects local inference so old tasks cannot keep consuming CPU.") }}</span></div>
        <p class="helper">{{ tr("模型服务自身的上下文窗口、KV Cache、量化方式和 CPU/GPU 线程仍由 MLX、LM Studio 或 Ollama 配置；Oviraptor 负责阻止并发放大和孤儿推理，不会偷偷把 Quick / Standard 改成 Deep。", "The model server still owns context length, KV cache, quantization, and CPU/GPU threads in MLX, LM Studio, or Ollama. Oviraptor prevents concurrency amplification and orphan inference, and never silently promotes Quick or Standard to Deep.") }}</p>
      </section>

      <fieldset v-if="cloudPolicyVisible" class="agent-policy-controls">
      <div class="form-grid two">
        <label class="field"
          ><span>{{ tr("前端预解析每批 URL 数", "Frontend recon URLs per batch") }}</span
          ><input v-model.number="form.settings.agentBatchSize" type="number" min="1" max="50"
        /></label>
        <label class="field"
          ><span>{{ tr("无进展熔断轮次", "No-progress request limit") }}</span
          ><input v-model.number="form.settings.agentNoToolTurnLimit" type="number" min="1" max="100"
        /></label>
      </div>
      <div class="agent-policy-table">
        <div class="agent-policy-head">
          <span>{{ tr("模式", "Mode") }}</span>
          <span>{{ tr("起始评分", "Score") }}</span>
          <span>{{ tr("请求软预算", "Requests") }}</span>
          <span>{{ tr("超时（秒）", "Timeout (s)") }}</span>
          <span>{{ tr("Token 软预算", "Soft token budget") }}</span>
        </div>
        <div>
          <strong>Quick</strong>
          <input v-model.number="form.settings.agentQuickScore" type="number" min="1" max="90" />
          <input v-model.number="form.settings.agentQuickRequestLimit" type="number" min="1" max="100" />
          <input v-model.number="form.settings.agentQuickTimeout" type="number" min="30" />
          <input v-model.number="form.settings.agentQuickTokenLimit" type="number" min="0" step="10000" />
        </div>
        <div>
          <strong>Standard</strong>
          <input v-model.number="form.settings.agentStandardScore" type="number" min="2" max="95" />
          <input v-model.number="form.settings.agentStandardRequestLimit" type="number" min="1" max="200" />
          <input v-model.number="form.settings.agentStandardTimeout" type="number" min="60" />
          <input v-model.number="form.settings.agentStandardTokenLimit" type="number" min="0" step="10000" />
        </div>
        <div>
          <strong>Deep</strong>
          <input v-model.number="form.settings.agentDeepScore" type="number" min="3" max="100" />
          <input v-model.number="form.settings.agentDeepRequestLimit" type="number" min="1" max="300" />
          <input v-model.number="form.settings.agentDeepTimeout" type="number" min="120" />
          <input v-model.number="form.settings.agentDeepTokenLimit" type="number" min="0" step="10000" />
        </div>
      </div>
      <p class="helper">
        {{
          tr(
            "这里配置的是起始软预算：仍有新端点、响应差异或验证证据时会继续，连续无增量后才收口；Oviraptor 另有按模式和本地/云端区分的绝对硬上限。只有确认 WAF、验证码、机器人挑战或持续限流才进入熔断区。",
            "This is the initial soft budget: work continues while new endpoints, response differences, or verification evidence are appearing, and closes only after consecutive no-progress turns. Oviraptor separately enforces deployment-aware hard ceilings. Only confirmed WAF, CAPTCHA, bot challenge, or sustained rate limiting enters the fuse zone.",
          )
        }}
      </p>
      </fieldset>
    </div>
    <div v-else-if="tab === 'collection'" class="form-grid three">
      <label class="field"
        ><span>Collection mode</span
        ><select v-model="form.settings.collectionMode">
          <option>all</option>
          <option>discover</option>
          <option>expand</option>
        </select></label
      >
      <label class="field"
        ><span>FOFA profile</span
        ><select v-model="form.settings.fofaProfile">
          <option>professional</option>
          <option>business</option>
          <option>basic</option>
          <option>auto</option>
        </select></label
      >
      <label class="field"
        ><span>Page size</span
        ><input v-model.number="form.settings.pageSize" type="number"
      /></label>
      <label class="field"
        ><span>Interval (s)</span
        ><input
          v-model.number="form.settings.interval"
          type="number"
          step="0.5"
      /></label>
      <label class="field"
        ><span>Timeout (s)</span
        ><input v-model.number="form.settings.collectionTimeout" type="number"
      /></label>
      <label class="field"
        ><span>Max pages (0=all)</span
        ><input v-model.number="form.settings.maxPages" type="number"
      /></label>
      <label class="field"
        ><span>{{
          tr("派生域名回查上限/公司", "Derived domain limit/company")
        }}</span
        ><input
          v-model.number="form.settings.maxDerivedDomains"
          type="number"
          min="0"
      /></label>
      <label class="check-field"
        ><input v-model="form.settings.fullHistory" type="checkbox" /> Full
        history</label
      >
      <label class="check-field"
        ><input v-model="form.settings.enableCidr24" type="checkbox" /> Enable
        CIDR /24</label
      >
      <label class="check-field"
        ><input
          v-model="form.settings.includeWeakFingerprints"
          type="checkbox"
        />
        Weak fingerprints</label
      >
    </div>
    <div v-else-if="tab === 'probe'" class="form-grid three">
      <label class="field"
        ><span>Priority rate</span
        ><input v-model.number="form.settings.priorityRate" type="number"
      /></label>
      <label class="field"
        ><span>Other rate</span
        ><input v-model.number="form.settings.otherRate" type="number"
      /></label>
      <label class="field"
        ><span>{{tr('探测并发（1–256）','Probe workers (1–256)')}}</span
        ><input v-model.number="form.settings.workers" type="number" min="1" max="256"
      /></label>
      <label class="field"
        ><span>{{tr('单次请求超时（秒）','Request timeout (seconds)')}}</span
        ><input v-model.number="form.settings.probeTimeout" type="number"
      /></label>
      <label class="field"><span>{{tr('每个 URL 总时间预算（秒）','Total network budget per URL (seconds)')}}</span><input v-model.number="form.settings.probeTotalTimeout" type="number" min="1" max="60" /><small>{{tr('连接、重定向、响应读取及重试共用；超时结束本轮并保留记录。调度排队和响应归并计算另计。','Shared by connection, redirects, body reads and retries. Timeout stops the current probe and retains its record. Scheduling and response grouping are separate.')}}</small></label>
      <label class="field"
        ><span>Retries</span
        ><input v-model.number="form.settings.probeRetries" type="number"
      /></label>
      <label class="field"
        ><span>Content threshold</span
        ><input v-model.number="form.settings.contentThreshold" type="number"
      /></label>
      <label class="check-field"
        ><input v-model="form.settings.runRefine" type="checkbox" /> Run
        refine</label
      >
      <label class="check-field"
        ><input v-model="form.settings.runProbe" type="checkbox" /> Run
        probe</label
      >
      <label class="check-field"
        ><input v-model="form.settings.includeOther" type="checkbox" /> Include
        Q2/Q3</label
      >
      <label class="check-field"
        ><input v-model="form.settings.includeWeak" type="checkbox" /> Include
        Q1 weak</label
      >
    </div>
    <div v-else-if="tab === 'rules'" class="rule-grid">
      <label class="field"
        ><span>Gambling keywords（每行一个）</span
        ><textarea v-model="keywordText" rows="8"></textarea>
      </label>
      <label class="field"
        ><span>Porn keywords（每行一个）</span
        ><textarea v-model="pornText" rows="8"></textarea>
      </label>
      <label class="field"
        ><span>Negative/context keywords</span
        ><textarea v-model="negativeText" rows="8"></textarea>
      </label>
      <label class="check-field span-three"
        ><input
          v-model="form.settings.replaceDefaultContentRules"
          type="checkbox"
        />
        {{
          tr(
            "仅使用自定义规则（不叠加内置高置信规则）",
            "Use custom rules only (do not include built-in high-confidence rules)",
          )
        }}</label
      >
      <p class="helper span-three">
        {{
          tr(
            "规则会随任务生成独立 JSON 快照并由探测脚本加载；修改规则后缓存策略也会自动更新。",
            "Rules are snapshotted per job and loaded by the probe worker; changing them also invalidates the probe cache.",
          )
        }}
      </p>
    </div>
    <div v-else class="security-rule-settings">
      <section class="security-rule-card">
        <header><strong>{{ tr('Native 分析器镜像', 'Native analyzer images') }}</strong></header>
        <p class="helper">{{ tr('仅用于源码 / 灰盒 / CI 分析。填写已批准且已加载到本机 Docker 的固定摘要镜像；应用不自动下载镜像。Web URL 调查不要求这些镜像。', 'For source, greybox and CI analysis only. Use approved, digest-pinned images already loaded into local Docker. Images are never downloaded automatically; Web URL tasks do not require them.') }}</p>
        <div class="form-grid two">
          <label v-for="engine in (['semgrep', 'codeql'] as const)" :key="engine" class="field span-two">
            <span>{{ engine }} · image@sha256</span>
            <input v-model="form.settings.nativeAnalyzers[engine].image" :aria-label="`${engine} pinned image`" placeholder="registry.example/analyzer@sha256:…" spellcheck="false" />
          </label>
        </div>
        <p class="helper">{{ tr('还需要启用下方对应的本地规则包。留空、规则缺失或镜像不可用会产生覆盖缺口，不表示扫描通过。', 'Enable the corresponding local rule pack below. Missing configuration, rules or images produce coverage gaps, not a passing scan.') }}</p>
      </section>
      <div class="security-rule-intro">
        <Database :size="17" />
        <div>
          <strong>{{ tr("代码审计规则源", "Code audit rule sources") }}</strong
          ><span>{{
            tr(
              "地址或分支留空时自动使用系统内置源。同步仅写入本机版本化缓存，并与上一个本地版本比较。",
              "Blank repository or reference fields use the built-in source. Sync writes to a versioned local cache and compares it with the previous local version.",
            )
          }}</span>
        </div>
      </div>
      <article
        v-for="definition in ruleDefinitions"
        :key="definition.key"
        class="security-rule-card"
      >
        <header>
          <div class="rule-title">
            <GitBranch :size="16" />
            <div>
              <strong>{{ definition.name }}</strong
              ><span>{{ definition.description }}</span>
            </div>
          </div>
          <span
            class="rule-pack-status"
            :class="rulePack(definition.key)?.status || 'not_installed'"
            >{{ ruleStatus(rulePack(definition.key)) }}</span
          >
        </header>
        <div class="form-grid two">
          <label class="field"
            ><span>{{ tr("GitHub 仓库", "GitHub repository") }}</span
            ><input
              v-model="form.settings[definition.repositoryKey]"
              :placeholder="definition.builtinRepository"
            /><small v-if="!configuredValue(definition.repositoryKey)"
              >{{ tr("使用系统内置源", "Using built-in source") }}：{{
                definition.builtinRepository
              }}</small
            ></label
          >
          <label class="field"
            ><span>{{ tr("分支 / 标签", "Branch / tag") }}</span
            ><input
              v-model="form.settings[definition.referenceKey]"
              :placeholder="definition.builtinReference"
            /><small v-if="!configuredValue(definition.referenceKey)"
              >{{ tr("使用内置分支", "Using built-in reference") }}：{{
                definition.builtinReference
              }}</small
            ></label
          >
        </div>
        <div class="rule-version-row">
          <div>
            <span>{{ tr("同步前版本", "Previous version") }}</span
            ><code>{{ rulePack(definition.key)?.previousVersion || "—" }}</code>
          </div>
          <div>
            <span>{{ tr("当前本地版本", "Current local version") }}</span
            ><code>{{ rulePack(definition.key)?.version || "—" }}</code>
          </div>
          <div>
            <span>{{ tr("上次同步", "Last sync") }}</span
            ><time>{{ rulePack(definition.key)?.lastSyncAt || "—" }}</time>
          </div>
          <button
            type="button"
            class="button secondary compact"
            :disabled="!!ruleBusy"
            @click="syncRulePack(definition)"
          >
            <RefreshCw
              :size="14"
              :class="{ spinning: ruleBusy === definition.key }"
            />{{
              ruleBusy === definition.key
                ? tr("同步中", "Syncing")
                : tr("同步并比较", "Sync and compare")
            }}
          </button>
        </div>
        <div
          v-if="rulePack(definition.key)?.status === 'syncing'"
          class="rule-sync-progress"
        >
          <header>
            <span>{{ rulePack(definition.key)?.progressMessage || tr("正在准备", "Preparing") }}</span>
            <b>{{ rulePack(definition.key)?.progress || 0 }}%</b>
          </header>
          <div class="progress-track"><i :style="{ width: `${rulePack(definition.key)?.progress || 0}%` }"></i></div>
          <small>{{ rulePack(definition.key)?.progressStage || "prepare" }}</small>
        </div>
        <div
          v-if="rulePack(definition.key)?.status === 'ready'"
          class="rule-change-summary"
        >
          <span class="added"
            >+ {{ rulePack(definition.key)?.addedCount || 0 }}
            {{ tr("新增", "added") }}</span
          ><span class="modified"
            >~ {{ rulePack(definition.key)?.modifiedCount || 0 }}
            {{ tr("修改", "modified") }}</span
          ><span class="deleted"
            >- {{ rulePack(definition.key)?.deletedCount || 0 }}
            {{ tr("删除", "deleted") }}</span
          >
        </div>
        <div
          v-if="
            Array.isArray(rulePack(definition.key)?.changeSummary) &&
            rulePack(definition.key)!.changeSummary.length
          "
          class="rule-change-files"
        >
          <code
            v-for="change in rulePack(definition.key)!.changeSummary.slice(
              0,
              12,
            )"
            :key="`${change.status}-${change.path}`"
            ><b :class="change.status">{{
              change.status === "added"
                ? "+"
                : change.status === "deleted"
                  ? "-"
                  : "~"
            }}</b
            >{{ change.path }}</code
          ><small v-if="rulePack(definition.key)!.changeSummary.length > 12">{{
            tr(
              `另有 ${rulePack(definition.key)!.changeSummary.length - 12} 项，数据库最多保留 200 项差异明细。`,
              `${rulePack(definition.key)!.changeSummary.length - 12} more; up to 200 changes are retained.`,
            )
          }}</small>
        </div>
        <p v-if="rulePack(definition.key)?.error" class="rule-pack-error">
          {{ rulePack(definition.key)?.error }}
        </p>
      </article>
      <p v-if="ruleMessage" class="rule-pack-message">{{ ruleMessage }}</p>
    </div>
    <p v-if="error" class="form-error">{{ error }}</p>
    <template #footer>
      <button class="button ghost" @click="$emit('close')">
        {{ tr("取消", "Cancel") }}
      </button>
      <button class="button primary" :disabled="busy" @click="save">
        {{ busy ? tr("保存中…", "Saving…") : tr("保存配置", "Save profile") }}
      </button>
    </template>
  </ModalShell>
</template>
