<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { CheckCircle2, Copy, RefreshCw, Send, ShieldAlert, UserRound, X } from "@lucide/vue";
import type { InvestigationApiModel, InvestigationHypothesis, InvestigationValidation, SentinelOpportunity } from "../types";
import { sentinelApi } from "../api";
import { buildRawHttpRequest, buildRawHttpResponse, parseRawHttpRequest, prettyHttpBody } from "../httpMessage";

const props = defineProps<{
  scanId: string;
  targetUrl: string;
  opportunity: SentinelOpportunity;
  api?: InvestigationApiModel;
  hypothesis?: InvestigationHypothesis;
}>();
const emit = defineEmits<{ close: []; saved: [validation: InvestigationValidation] }>();

type ReplayResponse = { status: number; statusText: string; headers: Record<string, string>; body: string; decodedBody?: string; contentType?: string; contentEncoding?: string; elapsedMs: number; identityId?: string };
type IdentityChoice = { key: string; label: string; available: boolean; observation?: Record<string, any>; status?: number; replayed?: boolean };
const anonymousIdentity = (value: unknown) => String(value || "").trim().toLowerCase() === "anonymous";
const method = ref("GET");
const url = ref("");
const headers = ref<Record<string, string>>({});
const body = ref("");
const requestRaw = ref("");
const requestTab = ref<"message" | "structure">("message");
const responseTab = ref<"pretty" | "raw" | "headers">("pretty");
const wrapLines = ref(true);
const allowMutation = ref(false);
const busy = ref(false);
const error = ref("");
const response = ref<ReplayResponse>();
const verdict = ref("");
const severity = ref("medium");
const confidence = ref("medium");
const note = ref("");
const nextAction = ref("");
const saved = ref<InvestigationValidation>();
const history = ref<InvestigationValidation[]>([]);
const selectedIdentityKey = ref("");
const responseIdentityKey = ref("");

const opportunityRecord = computed<Record<string, any>>(() => props.opportunity.record || {});
const fallbackUrl = computed(() => String(
  opportunityRecord.value.url || opportunityRecord.value.endpoint || opportunityRecord.value.route ||
  props.opportunity.recommendedAction?.targetUrl || props.targetUrl || "",
));
const fallbackMethod = computed(() => String(
  opportunityRecord.value.method || opportunityRecord.value.httpMethod ||
  props.opportunity.recommendedAction?.method || "GET",
).toUpperCase());
const fallbackHeaders = computed<Record<string, string>>(() => {
  const record = opportunityRecord.value;
  const value = record.effectiveRequestHeaders || record.requestHeaders || record.headers ||
    props.opportunity.recommendedAction?.headers || {};
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, String(item)]));
});
const fallbackBody = computed(() => String(
  opportunityRecord.value.postData || opportunityRecord.value.requestBody || opportunityRecord.value.body ||
  props.opportunity.recommendedAction?.body || "",
));
const repeaterTitle = computed(() => props.api
  ? `请求重放 · ${String(props.api.method || fallbackMethod.value).toUpperCase()} ${props.api.normalizedPath || "/"}`
  : props.opportunity.title);
const repeaterSubtitle = computed(() => String(props.api?.url || fallbackUrl.value || props.targetUrl || ""));
const identityChoices = computed<IdentityChoice[]>(() => {
  const apiKeys = Array.isArray(props.api?.identityKeys) ? props.api!.identityKeys.map(String).filter(Boolean) : [];
  const observations = Array.isArray(props.api?.payload?.identityObservations)
    ? props.api!.payload.identityObservations.filter((item: any) => item && typeof item === "object")
    : [];
  const keys = [...new Set([...apiKeys, ...observations.map((item: any) => String(item.identityKey || "")).filter(Boolean)])];
  if (!keys.length) return [{ key: "captured", label: "原始采集", available: true }];
  return keys.map((key, index) => {
    const observation = observations.find((item: any) => String(item.identityKey || "") === key);
    return {
      key,
      label: anonymousIdentity(key) ? "匿名访问" : `账号 ${String.fromCharCode(65 + keys.slice(0, index).filter((item) => !anonymousIdentity(item)).length)}`,
      available: Boolean(observation?.observed !== false && (observation?.url || props.api?.url)),
      observation,
      status: observation?.status,
      replayed: Boolean(observation?.replayed),
    };
  });
});
const authenticatedIdentityChoices = computed(() => identityChoices.value.filter((item) => !anonymousIdentity(item.key) && item.key !== "captured"));
const showIdentitySelector = computed(() => authenticatedIdentityChoices.value.length > 0);
const selectedIdentity = computed(() => identityChoices.value.find((item) => item.key === selectedIdentityKey.value) || identityChoices.value[0]);
const responseIdentityLabel = computed(() => showIdentitySelector.value ? identityChoices.value.find((item) => item.key === responseIdentityKey.value)?.label || "" : "");
const sendButtonLabel = computed(() => busy.value ? "发送中…" : showIdentitySelector.value ? `以 ${selectedIdentity.value?.label || "当前账号"} 发送` : "发送请求");
const responseWaitingLabel = computed(() => showIdentitySelector.value ? `等待 ${selectedIdentity.value?.label || "当前账号"} 的 HTTP 响应` : "等待 HTTP 响应");
const assessmentNotePlaceholder = computed(() => showIdentitySelector.value ? "记录账号差异、判断依据和缺失证据" : "记录判断依据、响应差异和缺失证据");
const selectedObservation = computed<Record<string, any>>(() => selectedIdentity.value?.observation || {});
const initialHeaders = computed<Record<string, string>>(() => {
  const observation = selectedObservation.value;
  const value = observation.requestHeaders || observation.effectiveRequestHeaders || observation.headers ||
    props.api?.payload?.headers || props.api?.payload?.effectiveRequestHeaders || props.api?.payload?.requestHeaders || fallbackHeaders.value;
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, String(item)]));
});
const parsedPreview = computed(() => {
  try { return parseRawHttpRequest(requestRaw.value, url.value || fallbackUrl.value); } catch { return undefined; }
});
const mutation = computed(() => !["GET", "HEAD", "OPTIONS"].includes(String(parsedPreview.value?.method || requestRaw.value.trim().split(/\s+/)[0] || "GET").toUpperCase()));
const canSend = computed(() => Boolean(parsedPreview.value?.url) && (!mutation.value || allowMutation.value) && !busy.value);
const decodedBody = computed(() => prettyHttpBody(response.value?.decodedBody || response.value?.body || ""));
const rawResponse = computed(() => buildRawHttpResponse({
  status: response.value?.status ?? null,
  statusText: response.value?.statusText,
  headers: response.value?.headers,
  body: response.value?.decodedBody || response.value?.body || "",
}));
const responseHeaders = computed(() => {
  if (!response.value) return "";
  return Object.entries(response.value.headers || {}).map(([name, value]) => `${name}: ${value}`).join("\r\n");
});
const requestStats = computed(() => {
  const parsed = parsedPreview.value;
  return {
    method: parsed?.method || "无法解析",
    url: parsed?.url || "请检查请求行与 Host",
    headerCount: parsed ? Object.keys(parsed.headers).length : 0,
    bodyBytes: parsed ? new TextEncoder().encode(parsed.body).length : 0,
  };
});
const aiAssessment = computed(() => {
  if (!response.value) return "发送请求后生成初步判断。";
  if (response.value.status >= 500) return "请求触发服务端错误，暂不能直接认定为安全问题，建议保留证据并继续核查。";
  if ([401, 403].includes(response.value.status)) return showIdentitySelector.value ? "请求被身份或权限边界拦截；需要与另一账号使用同一请求进行对比。" : "匿名请求被身份或权限边界拦截；这是访问边界证据，不能单独认定为漏洞。";
  if (response.value.status >= 400) return "请求返回客户端错误，建议检查参数契约和授权边界后再判断。";
  const text = response.value.decodedBody || response.value.body || "";
  if (/password|token|secret|role|user_id|account_id|permission/i.test(text)) return showIdentitySelector.value ? "响应包含身份、权限或对象字段，建议进入账号间同请求对比。" : "匿名响应包含身份、权限或对象字段，建议先确认字段是否为公开数据，再决定是否补充登录态对照。";
  return showIdentitySelector.value ? "收到正常响应，当前没有足够证据确认问题；可标记为正常，或继续补充账号间对比证据。" : "收到正常匿名响应，当前没有足够证据确认问题；可标记为正常，或继续核查响应内容与业务影响。";
});

function restoreCapturedRequest() {
  const observation = selectedObservation.value;
  method.value = String(observation.method || props.api?.method || fallbackMethod.value || "GET").toUpperCase();
  url.value = String(observation.url || props.api?.url || fallbackUrl.value || props.targetUrl || "");
  headers.value = { ...initialHeaders.value };
  body.value = String(observation.requestBody || observation.postData || observation.body || props.api?.payload?.postData || props.api?.payload?.requestBody || props.api?.payload?.body || fallbackBody.value || "");
  requestRaw.value = buildRawHttpRequest({ method: method.value, url: url.value, headers: headers.value, body: body.value });
  error.value = "";
}
function resetFromContext() {
  selectedIdentityKey.value = identityChoices.value.find((item) => item.available)?.key || identityChoices.value[0]?.key || "";
  restoreCapturedRequest();
  response.value = undefined;
  responseIdentityKey.value = "";
  verdict.value = "";
  saved.value = undefined;
  allowMutation.value = false;
  responseTab.value = "pretty";
  requestTab.value = "message";
  void loadHistory();
}
function selectIdentity(key: string) {
  const choice = identityChoices.value.find((item) => item.key === key);
  if (!choice?.available || choice.key === selectedIdentityKey.value) return;
  selectedIdentityKey.value = choice.key;
  restoreCapturedRequest();
  response.value = undefined;
  responseIdentityKey.value = "";
  saved.value = undefined;
}
async function loadHistory() {
  try {
    const rows = await sentinelApi.listInvestigationValidations(props.scanId, props.opportunity.id > 0 ? props.opportunity.id : undefined);
    history.value = props.opportunity.id > 0 ? rows : rows.filter((row) => !props.api?.apiKey || row.apiKey === props.api.apiKey);
  } catch { history.value = []; }
}
watch(() => [props.api, props.opportunity, props.scanId], resetFromContext, { immediate: true });

function copy(value: string) { void navigator.clipboard?.writeText(value); }
function setVerdict(value: string) {
  verdict.value = value;
  if (value === "confirmed_issue") severity.value = severity.value === "none" ? "medium" : severity.value;
  if (value === "normal" || value === "not_applicable") severity.value = "none";
  if (value === "needs_more_evidence") nextAction.value = showIdentitySelector.value ? "继续同请求账号对比并补齐缺失响应" : "补充请求参数、响应内容或登录态对照证据";
  if (value === "confirmed_issue") nextAction.value = "创建行动项并进入漏洞结论复核";
}
function historyIdentityLabel(identityId?: string) {
  if (!showIdentitySelector.value || anonymousIdentity(identityId)) return "匿名请求";
  return identityChoices.value.find((choice) => choice.key === identityId)?.label || identityId || "未标注账号";
}
function applyRawRequest() {
  const parsed = parseRawHttpRequest(requestRaw.value, url.value || fallbackUrl.value);
  method.value = parsed.method;
  url.value = parsed.url;
  headers.value = parsed.headers;
  body.value = parsed.body;
  return parsed;
}
async function send() {
  error.value = "";
  let parsed;
  try { parsed = applyRawRequest(); } catch (e) { error.value = String(e); return; }
  if (!canSend.value) { error.value = mutation.value ? "状态变更请求必须确认授权后才能发送" : "请求报文缺少有效 URL"; return; }
  busy.value = true;
  response.value = undefined;
  saved.value = undefined;
  try {
    response.value = await sentinelApi.replayInvestigationRequest({ ...parsed, allowMutation: allowMutation.value, timeoutMs: 120000, identityId: selectedIdentity.value?.key || "" });
    responseIdentityKey.value = response.value.identityId || selectedIdentity.value?.key || "";
    verdict.value = "";
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
function responseForCopy() {
  if (!response.value) return "";
  if (responseTab.value === "headers") return responseHeaders.value;
  return responseTab.value === "pretty" ? decodedBody.value : rawResponse.value;
}
async function save() {
  if (!response.value || !verdict.value) { error.value = "请先发送请求，并选择验证结论"; return; }
  try { applyRawRequest(); } catch (e) { error.value = String(e); return; }
  busy.value = true;
  error.value = "";
  try {
    const result = await sentinelApi.saveInvestigationValidation({
      scanId: props.scanId, targetUrl: props.targetUrl, opportunityId: props.opportunity.id > 0 ? props.opportunity.id : undefined, hypothesisId: props.hypothesis?.id,
      apiKey: props.api?.apiKey, identityId: responseIdentityKey.value || selectedIdentity.value?.key, method: method.value, requestUrl: url.value, requestHeaders: headers.value, requestBody: body.value,
      responseStatus: response.value.status, responseStatusText: response.value.statusText, responseHeaders: response.value.headers,
      responseBody: response.value.body, decodedBody: decodedBody.value, verdict: verdict.value, severity: severity.value,
      confidence: confidence.value, aiAssessment: aiAssessment.value, note: note.value, nextAction: nextAction.value,
      evidenceRefs: [props.api?.apiKey, `opportunity:${props.opportunity.id}`].filter(Boolean) as string[],
    });
    saved.value = result;
    history.value = [result, ...history.value.filter((row) => row.id !== result.id)];
    emit("saved", result);
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
</script>

<template>
  <div class="repeater-backdrop" @click.self="emit('close')">
    <section class="sentinel-repeater" role="dialog" aria-modal="true">
      <header class="repeater-topbar">
        <div class="repeater-heading-copy"><span class="eyebrow">HTTP REPEATER · {{ opportunity.category }}</span><h2 :title="repeaterTitle">{{ repeaterTitle }}</h2><p :title="repeaterSubtitle">{{ repeaterSubtitle }}</p></div>
        <button class="icon-button" type="button" title="关闭" @click="emit('close')"><X :size="18" /></button>
      </header>
      <div class="repeater-context"><span>任务 {{ scanId }}</span><span v-if="api">{{ api.method }} {{ api.normalizedPath }}</span><span v-if="hypothesis">契约 {{ hypothesis.status }}</span><span>{{ mutation ? "状态变更：发送前授权" : "只读：可直接发送" }}</span></div>
      <div v-if="showIdentitySelector" class="identity-replay-bar">
        <div class="identity-replay-title"><UserRound :size="16" /><span><b>选择重放身份</b><small>切换后载入该账户实际采集的 URL、Cookie、请求头与请求体，不混用 A/B 会话。</small></span></div>
        <div class="identity-replay-options">
          <button v-for="choice in identityChoices" :key="choice.key" type="button" :disabled="!choice.available" :class="{ selected: choice.key === selectedIdentity?.key }" @click="selectIdentity(choice.key)">
            <i></i><span><b>{{ choice.label }}</b><small>{{ choice.available ? `${choice.replayed ? '交叉重放采集' : '浏览器自然采集'}${choice.status ? ` · HTTP ${choice.status}` : ''}` : '该身份没有完整请求' }}</small></span>
          </button>
        </div>
        <strong class="identity-replay-current">本次将以 {{ selectedIdentity?.label || "原始采集" }} 发送</strong>
      </div>
      <main class="repeater-main">
        <section class="repeater-pane request-pane">
          <header class="message-heading"><div><b>Request</b><small>完整 HTTP 请求报文</small></div><div><button class="text-button" type="button" @click="restoreCapturedRequest"><RefreshCw :size="13" />恢复采集</button><button class="text-button" type="button" @click="copy(requestRaw)"><Copy :size="13" />复制</button></div></header>
          <nav class="message-tabs"><button :class="{active:requestTab==='message'}" @click="requestTab='message'">报文</button><button :class="{active:requestTab==='structure'}" @click="requestTab='structure'">结构</button><button :class="{active:wrapLines}" @click="wrapLines=!wrapLines">{{ wrapLines ? '自动换行' : '单行滚动' }}</button></nav>
          <textarea v-if="requestTab==='message'" v-model="requestRaw" class="http-editor" :class="{nowrap:!wrapLines}" spellcheck="false" />
          <div v-else class="message-structure"><dl><div><dt>方法</dt><dd>{{ requestStats.method }}</dd></div><div><dt>完整 URL</dt><dd><code>{{ requestStats.url }}</code></dd></div><div><dt>请求头</dt><dd>{{ requestStats.headerCount }} 个</dd></div><div><dt>请求体</dt><dd>{{ requestStats.bodyBytes }} bytes</dd></div></dl><p>结构视图只用于快速确认；切回“报文”可直接修改请求行、请求头和正文。</p></div>
          <div class="repeater-sendbar"><label v-if="mutation" class="mutation-warning"><input v-model="allowMutation" type="checkbox" />授权发送这一次状态变更请求</label><span v-else class="readonly-badge">GET / HEAD / OPTIONS 无需人工放行</span><button class="button primary" :disabled="!canSend" type="button" @click="send"><Send :size="14" />{{ sendButtonLabel }}</button></div>
        </section>
        <section class="repeater-pane response-pane">
          <header class="message-heading"><div><b>Response <em v-if="responseIdentityLabel">{{ responseIdentityLabel }}</em></b><small v-if="response">{{ response.status }} {{ response.statusText }} · {{ response.elapsedMs }} ms · {{ response.contentType || "unknown" }}</small><small v-else>{{ responseWaitingLabel }}</small></div><button v-if="response" class="text-button" type="button" @click="copy(responseForCopy())"><Copy :size="13" />复制</button></header>
          <nav class="message-tabs"><button :class="{active:responseTab==='pretty'}" @click="responseTab='pretty'">Pretty</button><button :class="{active:responseTab==='raw'}" @click="responseTab='raw'">Raw</button><button :class="{active:responseTab==='headers'}" @click="responseTab='headers'">Headers</button></nav>
          <pre v-if="response" class="http-viewer" :class="{nowrap:!wrapLines}">{{ responseTab === 'pretty' ? decodedBody : responseTab === 'raw' ? rawResponse : responseHeaders }}</pre><div v-else class="response-empty"><Send :size="22" /><span>发送后在这里显示完整 HTTP 响应</span></div>
          <details v-if="response" class="assessment-drawer"><summary><ShieldAlert :size="15" /><b>AI 判断与验证结论</b><span>{{ aiAssessment }}</span></summary><div class="assessment-card"><div class="verdict-buttons"><button :class="{selected:verdict==='confirmed_issue'}" @click="setVerdict('confirmed_issue')">确认存在问题</button><button :class="{selected:verdict==='normal'}" @click="setVerdict('normal')">正常 / 可忽略</button><button :class="{selected:verdict==='needs_more_evidence'}" @click="setVerdict('needs_more_evidence')">需要更多证据</button><button :class="{selected:verdict==='unauthorized_stop'}" @click="setVerdict('unauthorized_stop')">权限边界 / 停止</button></div><div class="assessment-fields"><select v-model="severity"><option value="none">无</option><option value="low">低</option><option value="medium">中</option><option value="high">高</option><option value="critical">严重</option></select><select v-model="confidence"><option>low</option><option>medium</option><option>high</option></select><input v-model="nextAction" placeholder="下一步动作" /></div><textarea v-model="note" :placeholder="assessmentNotePlaceholder"></textarea><button class="button primary save-verdict" :disabled="busy || !verdict" @click="save"><CheckCircle2 :size="14" />{{ saved ? '已保存并同步' : '保存验证结论' }}</button></div></details>
          <details v-if="history.length" class="validation-history"><summary>验证历史（{{ history.length }} 次）</summary><article v-for="item in history" :key="item.id"><header><b>{{ historyIdentityLabel(item.identityId) }} · {{ item.verdict }}</b><span>{{ item.responseStatus }} · {{ item.severity }} · {{ item.confidence }}</span><time>{{ item.updatedAt }}</time></header><p>{{ item.aiAssessment || item.note || "未填写说明" }}</p><code>{{ item.method }} {{ item.requestUrl }}</code></article></details>
        </section>
      </main>
      <p v-if="error" class="repeater-error">{{ error }}</p>
    </section>
  </div>
</template>

<style scoped>
.repeater-backdrop{position:fixed;inset:0;z-index:80;display:grid;place-items:center;padding:14px;background:rgba(11,20,36,.58);backdrop-filter:blur(4px)}
.sentinel-repeater{width:min(1680px,98vw);height:min(980px,96vh);display:flex;flex-direction:column;overflow:hidden;border:1px solid #c9d6e8;border-radius:14px;background:#f6f9fd;box-shadow:0 28px 90px rgba(0,0,0,.28);color:#24344d}
.repeater-topbar{display:flex;justify-content:space-between;gap:18px;padding:15px 18px 11px;border-bottom:1px solid #dce5f0;background:#fff}.repeater-heading-copy{min-width:0;flex:1}.repeater-topbar h2{margin:3px 0 2px;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:17px}.repeater-topbar p{margin:0;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#718096;font:10px ui-monospace,monospace}.icon-button{display:grid;place-items:center;width:32px;height:32px;flex:0 0 32px;border:1px solid #d6e0ed;border-radius:8px;background:#fff;color:#54657b}
.repeater-context{display:flex;gap:6px;flex-wrap:wrap;padding:7px 18px;color:#6e7e92;font:9px ui-monospace,monospace;background:#f8fbff;border-bottom:1px solid #e3eaf3}.repeater-context span{padding:3px 7px;border:1px solid #dce6f1;border-radius:999px;background:#fff}
.identity-replay-bar{display:grid;grid-template-columns:minmax(220px,.8fr) minmax(360px,1.4fr) auto;align-items:center;gap:12px;padding:9px 18px;border-bottom:1px solid #dbe5f0;background:#eef6ff}.identity-replay-title{display:flex;align-items:center;gap:8px;color:#285f9e}.identity-replay-title span{min-width:0}.identity-replay-title b,.identity-replay-title small{display:block}.identity-replay-title b{font-size:11px}.identity-replay-title small{margin-top:2px;color:#667b94;font-size:8px}.identity-replay-options{display:flex;gap:6px;min-width:0;overflow-x:auto}.identity-replay-options button{display:flex;align-items:center;gap:7px;min-width:128px;padding:6px 9px;border:1px solid #cad8e9;border-radius:8px;background:#fff;color:#4b5e75;text-align:left}.identity-replay-options button.selected{border-color:#327be0;background:#e4f0ff;color:#1f5da9;box-shadow:0 0 0 1px rgba(50,123,224,.14)}.identity-replay-options button:disabled{opacity:.48}.identity-replay-options i{width:8px;height:8px;flex:0 0 auto;border-radius:50%;background:#9aa9ba}.identity-replay-options button.selected i{background:#20a46b;box-shadow:0 0 0 3px rgba(32,164,107,.14)}.identity-replay-options b,.identity-replay-options small{display:block}.identity-replay-options b{font-size:10px}.identity-replay-options small{margin-top:1px;white-space:nowrap;font-size:8px;color:#78899e}.identity-replay-current{padding:6px 9px;border-radius:7px;background:#1f6ed4;color:#fff;white-space:nowrap;font-size:9px}.message-heading em{display:inline-block;margin-left:6px;padding:2px 6px;border-radius:999px;background:#e5f0ff;color:#2468bd;font-size:9px;font-style:normal}
.repeater-main{display:grid;grid-template-columns:minmax(0,1.08fr) minmax(0,.92fr);gap:1px;min-height:0;flex:1;background:#d7e0eb}.repeater-pane{display:flex;flex-direction:column;min-width:0;min-height:0;padding:0;background:#fff}.message-heading{display:flex;justify-content:space-between;gap:10px;align-items:center;padding:11px 14px 7px}.message-heading>div:last-child{display:flex;gap:9px}.message-heading b{display:block;font-size:16px}.message-heading small{display:block;margin-top:2px;color:#7c899a;font-size:9px}.text-button{display:inline-flex;align-items:center;gap:4px;border:0;background:transparent;color:#4c79b5;font-size:9px}
.message-tabs{display:flex;align-items:center;gap:1px;padding:0 10px;border-bottom:1px solid #dfe5ec}.message-tabs button{border:0;border-bottom:2px solid transparent;padding:7px 10px;background:transparent;color:#5f6d7f;font-size:10px}.message-tabs button.active{border-bottom-color:#f06d55;color:#26384f;font-weight:800}.message-tabs button:last-child{margin-left:auto;color:#6e8199}
.http-editor,.http-viewer{box-sizing:border-box;width:100%;min-height:0;flex:1;margin:0;border:0;outline:0;padding:12px 14px;background:#fff;color:#202a38;font:12px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;tab-size:2;white-space:pre-wrap;overflow:auto;overflow-wrap:anywhere}.http-editor{resize:none}.http-editor.nowrap,.http-viewer.nowrap{white-space:pre;overflow-wrap:normal}.http-viewer{border-bottom:1px solid #e1e7ee}.message-structure{display:grid;align-content:start;gap:15px;min-height:0;flex:1;padding:18px;overflow:auto}.message-structure dl{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;margin:0}.message-structure dl>div{min-width:0;padding:11px;border:1px solid #e0e6ed;border-radius:8px;background:#f8fafc}.message-structure dt{color:#7a8798;font-size:9px}.message-structure dd{margin:5px 0 0;color:#2f4056;font-size:11px}.message-structure code{overflow-wrap:anywhere}.message-structure p{margin:0;color:#708096;font-size:10px}
.repeater-sendbar{display:flex;align-items:center;gap:10px;padding:9px 12px;border-top:1px solid #dfe5ec;background:#f8fafc}.mutation-warning{display:flex;align-items:center;gap:6px;flex:1;color:#a26317;font-size:9px}.readonly-badge{flex:1;color:#388064;font-size:9px}.repeater-sendbar .button{min-width:86px;justify-content:center}.response-empty{display:grid;place-items:center;gap:8px;flex:1;color:#8b9aae;font-size:10px;background:#fff}
.assessment-drawer,.validation-history{flex:0 0 auto;border-top:1px solid #dbe4ee;background:#f8fbff}.assessment-drawer>summary,.validation-history>summary{display:grid;grid-template-columns:auto auto minmax(0,1fr);align-items:center;gap:7px;padding:9px 12px;cursor:pointer;color:#315f9d;font-size:10px}.assessment-drawer>summary span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#667991;font-weight:400}.assessment-card{display:grid;gap:8px;padding:0 12px 12px}.verdict-buttons{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:5px}.verdict-buttons button{padding:7px;border:1px solid #d5dfeb;border-radius:7px;background:#fff;color:#56687f;font-size:9px}.verdict-buttons button.selected{border-color:#4e88db;background:#eaf3ff;color:#215ea9;font-weight:700}.assessment-fields{display:grid;grid-template-columns:90px 90px minmax(0,1fr);gap:6px}.assessment-fields select,.assessment-fields input,.assessment-card textarea{min-width:0;border:1px solid #d4deea;border-radius:7px;background:#fff;color:#24344d;padding:7px;font-size:9px}.assessment-card textarea{min-height:48px;resize:vertical}.save-verdict{justify-content:center}.validation-history article{display:grid;gap:3px;padding:8px 12px;border-top:1px solid #edf1f6}.validation-history article header{display:flex;gap:8px;align-items:center}.validation-history article span,.validation-history article time,.validation-history article p{margin:0;color:#77879b;font-size:9px}.validation-history article code{overflow-wrap:anywhere;color:#3f608e;font:9px ui-monospace,monospace}.repeater-error{margin:0;padding:7px 14px;color:#c63f4c;font-size:10px;background:#fff1f2}
@media(max-width:900px){.repeater-main{grid-template-columns:1fr;overflow:auto}.sentinel-repeater{height:97vh}.repeater-pane{min-height:520px}.verdict-buttons{grid-template-columns:repeat(2,1fr)}}
@media(max-width:560px){.repeater-backdrop{padding:5px}.repeater-topbar{padding:12px}.repeater-context{padding:7px 12px}.assessment-fields,.message-structure dl{grid-template-columns:1fr}.assessment-drawer>summary{grid-template-columns:auto 1fr}.assessment-drawer>summary span{grid-column:1/-1}}
</style>
