import { ref, type Ref } from "vue";
import { api } from "../../../api";
import type { ClosureHandoffInput, ClosureHandoffPreview, ClosureHandoffReceipt, SentinelScan } from "../types";

type HandoffForm = {
  projectId: number;
  taskName: string;
  urls: string;
  scanMode: string;
  maxBudgetUsd: number | undefined;
  authSessionIds: string[];
  skillIds: number[];
  instruction: string;
  closure: string;
};

type HandoffContext = {
  source: () => ClosureHandoffPreview | undefined;
  hasFollowup: () => boolean;
  isWebMode: () => boolean;
  generation: () => number;
  isCurrent: (source: ClosureHandoffPreview, generation: number) => boolean;
  form: HandoffForm;
  busy: Ref<boolean>;
  authSessionScopeId: Ref<string>;
  resetIdentitySelection: (scopeId: string) => void;
  tr: (zh: string, en: string) => string;
  notify: (type: "success" | "error" | "info", text: string) => void;
  reload: () => void;
  prepareScan: (scan: SentinelScan) => void;
  openScan: (scan: SentinelScan) => void;
};

function validateReceipt(receipt: ClosureHandoffReceipt, source: ClosureHandoffPreview, input?: ClosureHandoffInput) {
  if (!receipt || receipt.sourceScanId !== source.sourceScanId || receipt.closureId !== source.closureId
    || receipt.executionGranted !== false || receipt.sourceExecutionSettled !== false
    || !receipt.scanId || receipt.scanId === source.sourceScanId || receipt.scan?.id !== receipt.scanId
    || receipt.scan.projectId !== source.projectId || receipt.scan.scanType !== "web"
    || receipt.scan.sourcePath || !receipt.requestId || receipt.sourceHash !== source.sourceHash
    || !/^[a-f0-9]{64}$/.test(receipt.sourceHash)
    || !Number.isFinite(Date.parse(receipt.createdAt)) || Object.keys(receipt).length !== 9
    || (input && (receipt.requestId !== input.requestId || receipt.sourceHash !== input.sourceHash
      || receipt.scan.status !== "draft" || receipt.scan.attemptCount !== 0))) {
    throw new Error("closure_handoff_receipt_mismatch");
  }
}

// All retries retain the first frozen input. A read-only lookup precedes any
// creation so an uncertain IPC response cannot produce a second draft.
export function useClosureHandoff(context: HandoffContext) {
  const handoffConfirmed = ref(false);
  const pendingHandoff = ref<ClosureHandoffInput>();

  async function saveClosureHandoff() {
    const source = context.source();
    if (!source || context.busy.value || !context.isCurrent(source, context.generation())) return;
    const generation = context.generation();
    const current = () => context.isCurrent(source, generation);
    context.busy.value = true;
    try {
      if (context.hasFollowup() || !context.isWebMode() || context.form.projectId !== source.projectId)
        throw new Error("closure_handoff_scope_mismatch");
      const preview = await api.previewWebClosureHandoff(source.sourceScanId);
      if (!current()) return;
      if (preview.sourceScanId !== source.sourceScanId || preview.closureId !== source.closureId
        || preview.sourceHash !== source.sourceHash
        || preview.projectId !== source.projectId || preview.executionSettled !== false || preview.targetRequestsGranted !== 0)
        throw new Error("closure_handoff_preview_mismatch");
      if (preview.savedHandoff) {
        validateReceipt(preview.savedHandoff, source);
        pendingHandoff.value = undefined;
        context.notify("info", context.tr("已找回原交接创建的任务，没有新建或重新启动。", "Recovered the original handoff task without creating or restarting it."));
        context.reload();
        if (preview.savedHandoff.scan.status === "draft") context.prepareScan(preview.savedHandoff.scan);
        else context.openScan(preview.savedHandoff.scan);
        return;
      }
      if (preview.sourceHash !== source.sourceHash) throw new Error("closure_handoff_source_changed_preview_again");
      if (!pendingHandoff.value) {
        if (!handoffConfirmed.value) throw new Error(context.tr("请确认新任务的目标、预算和身份；未选择身份表示匿名访问。", "Confirm targets, budget and identities; no selected identity means anonymous access."));
        if (!context.form.maxBudgetUsd || context.form.maxBudgetUsd <= 0) throw new Error("closure_handoff_budget_required");
        pendingHandoff.value = {
          requestId: globalThis.crypto.randomUUID(), sourceScanId: source.sourceScanId, closureId: source.closureId,
          sourceHash: source.sourceHash, taskName: context.form.taskName,
          urls: context.form.urls.split(/\r?\n/).map(s => s.trim()).filter(Boolean),
          scanMode: context.form.scanMode, maxBudgetUsd: context.form.maxBudgetUsd,
          authSessionIds: [...context.form.authSessionIds], authSessionScopeId: context.authSessionScopeId.value,
          skillIds: [...context.form.skillIds], instruction: context.form.instruction,
          closure: context.form.closure, operatorConfirmed: true,
        };
      }
      const input = pendingHandoff.value;
      if (input.sourceScanId !== source.sourceScanId || input.closureId !== source.closureId || input.sourceHash !== source.sourceHash)
        throw new Error("closure_handoff_pending_scope_mismatch");
      const receipt = await api.createWebClosureHandoff({ ...input, urls: [...input.urls], authSessionIds: [...input.authSessionIds], skillIds: [...input.skillIds] });
      if (!current()) return;
      validateReceipt(receipt, source, input);
      pendingHandoff.value = undefined;
      handoffConfirmed.value = false;
      context.resetIdentitySelection(input.authSessionScopeId);
      context.reload();
      context.notify("success", context.tr("独立关联草稿已保存，未启动扫描；原任务的未知结果和预算占用保持不变。", "Independent linked draft saved without starting. Source unknown effects and reservations are unchanged."));
      context.prepareScan(receipt.scan);
    } catch (error) {
      if (current()) context.notify("error", `${context.tr("交接未确认；再次保存先核对原记录，不会自动扫描。", "Handoff unconfirmed; save again checks the original record without automatically scanning.")} ${String(error)}`);
    } finally { context.busy.value = false; }
  }

  return { handoffConfirmed, pendingHandoff, saveClosureHandoff };
}
