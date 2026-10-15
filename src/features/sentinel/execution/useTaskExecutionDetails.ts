import { onBeforeUnmount, ref, watch } from "vue";
import type { AgentAttemptMailboxPage, AgentAttemptToolPage, HistoricalImportPreview, NativeScanStatus, SentinelRunnerLogView, SentinelScan, SentinelScanAttempt } from "../types";
import type { AgentKnowledgeEntry, AgentLearningCandidate } from "../../skills/catalogTypes";
import { sentinelApi } from "../api";
import { knowledgeCatalogApi } from "../../skills/catalogApi";
import { validateHistoricalPreviews } from "../results/historicalPreviewContract";
import { useLiveRunnerLog } from "./useLiveRunnerLog";

// Task-center read-only detail state; no task execution or mailbox acknowledgement.
export function useTaskExecutionDetails(props: { readonly preview?: SentinelScan }) {
  const detailTab = ref<"overview" | "execution" | "evidence" | "learning">("overview");
  const detailLoading = ref(false);
  const detailErrors = ref<string[]>([]);
  const attempts = ref<SentinelScanAttempt[]>([]);
  const selectedAttempt = ref<number>();
  const runnerLog = ref<SentinelRunnerLogView>();
  const runnerLogLoading = ref(false);
  const runnerLogError = ref("");
  const runnerLogConnectionError = ref("");
  const mailboxPage = ref<AgentAttemptMailboxPage>();
  const mailboxLoading = ref(false);
  const mailboxError = ref("");
  const toolPage = ref<AgentAttemptToolPage>();
  const toolLoading = ref(false);
  const toolError = ref("");
  const nativeStatus = ref<NativeScanStatus>();
  const historicalPreviews = ref<HistoricalImportPreview[]>([]);
  const learningCandidates = ref<AgentLearningCandidate[]>([]);
  const knowledge = ref<AgentKnowledgeEntry[]>([]);
  let detailRequestSerial = 0;
  let runnerLogRequestSerial = 0;
  let mailboxRequestSerial = 0;
  let toolRequestSerial = 0;
  let disposed = false;
  onBeforeUnmount(() => {
    disposed = true;
    ++detailRequestSerial;
    ++runnerLogRequestSerial;
    ++mailboxRequestSerial;
    ++toolRequestSerial;
  });
  const isSelected = (scanId: string, attemptNumber: number) => !disposed
    && props.preview?.id === scanId && selectedAttempt.value === attemptNumber;
  async function loadTools(scanId: string, attemptNumber: number, older = false) {
    if (!isSelected(scanId, attemptNumber)) return;
    const prior = older ? toolPage.value : undefined;
    if (older && (!prior?.hasOlder || !prior.olderCursor || toolLoading.value)) return;
    const serial = ++toolRequestSerial;
    if (!older) toolPage.value = undefined;
    toolError.value = "";
    toolLoading.value = true;
    try {
      const page = await sentinelApi.getNativeAttemptToolHistory(scanId, attemptNumber, older ? prior?.olderCursor ?? undefined : undefined);
      if (serial !== toolRequestSerial || props.preview?.id !== scanId || selectedAttempt.value !== attemptNumber) return;
      if (page.schemaVersion !== 2 || page.scanId !== scanId || page.attemptNumber !== attemptNumber) {
        throw new Error("execution_history_scope_mismatch");
      }
      toolPage.value = older && prior ? { ...page, invocations: [...page.invocations, ...prior.invocations] } : page;
    } catch {
      if (serial === toolRequestSerial && props.preview?.id === scanId
        && selectedAttempt.value === attemptNumber) toolError.value = "此轮工具调用暂时不可读取";
    } finally {
      if (serial === toolRequestSerial) toolLoading.value = false;
    }
  }
  async function loadMailbox(scanId: string, attemptNumber: number, older = false) {
    if (!isSelected(scanId, attemptNumber)) return;
    const prior = older ? mailboxPage.value : undefined;
    if (older && (!prior?.hasOlder || !prior.olderCursor || mailboxLoading.value)) return;
    const serial = ++mailboxRequestSerial;
    if (!older) mailboxPage.value = undefined;
    mailboxError.value = "";
    mailboxLoading.value = true;
    try {
      const page = await sentinelApi.getNativeAttemptMailboxHistory(scanId, attemptNumber, older ? prior?.olderCursor ?? undefined : undefined);
      if (serial !== mailboxRequestSerial || props.preview?.id !== scanId || selectedAttempt.value !== attemptNumber) return;
      if (page.scanId !== scanId || page.attemptNumber !== attemptNumber) {
        throw new Error("mailbox_history_scope_mismatch");
      }
      mailboxPage.value = older && prior ? {
        ...page,
        messages: [...page.messages, ...prior.messages],
      } : page;
    } catch {
      if (serial === mailboxRequestSerial && props.preview?.id === scanId
        && selectedAttempt.value === attemptNumber) mailboxError.value = "此轮协作消息暂时不可读取";
    } finally {
      if (serial === mailboxRequestSerial) mailboxLoading.value = false;
    }
  }
  async function loadAttemptLog(scanId: string, attemptNumber: number, retainSnapshot = false) {
    if (!isSelected(scanId, attemptNumber)) return;
    const serial = ++runnerLogRequestSerial;
    if (!retainSnapshot) runnerLog.value = undefined;
    runnerLogError.value = "";
    runnerLogLoading.value = true;
    try {
      const result = await sentinelApi.readSentinelRunnerLog(scanId, attemptNumber, 200);
      if (result.scanId !== scanId || result.attempt !== attemptNumber) {
        throw new Error("runner_log_scope_mismatch");
      }
      if (serial === runnerLogRequestSerial && props.preview?.id === scanId
        && selectedAttempt.value === attemptNumber) runnerLog.value = result;
    } catch {
      if (serial === runnerLogRequestSerial && props.preview?.id === scanId
        && selectedAttempt.value === attemptNumber) runnerLogError.value = "此轮日志暂时不可读取";
    } finally {
      if (serial === runnerLogRequestSerial) runnerLogLoading.value = false;
    }
  }
  useLiveRunnerLog(
    () => detailTab.value === "execution" && props.preview && selectedAttempt.value !== undefined
      ? { scanId: props.preview.id, attempt: selectedAttempt.value } : undefined,
    () => runnerLogLoading.value,
    ({ scanId, attempt }) => loadAttemptLog(scanId, attempt, true),
    (unavailable) => { runnerLogConnectionError.value = unavailable ? "实时日志连接不可用，暂以定时校对恢复" : ""; },
  );
  function chooseAttempt(attemptNumber: number) {
    if (disposed || !props.preview || !attempts.value.some((attempt) => attempt.attemptNumber === attemptNumber)) return;
    selectedAttempt.value = attemptNumber;
    void loadAttemptLog(props.preview.id, attemptNumber);
    void loadMailbox(props.preview.id, attemptNumber);
    void loadTools(props.preview.id, attemptNumber);
  }
  watch(() => ({ id: props.preview?.id, updatedAt: props.preview?.updatedAt }), async (current, previous) => {
    const scanId = current.id;
    const serial = ++detailRequestSerial;
    if (scanId !== previous?.id) {
      detailTab.value = "overview";
      selectedAttempt.value = undefined;
    }
    ++runnerLogRequestSerial;
    runnerLog.value = undefined;
    runnerLogLoading.value = false;
    runnerLogError.value = "";
    ++mailboxRequestSerial;
    mailboxPage.value = undefined;
    mailboxLoading.value = false;
    mailboxError.value = "";
    ++toolRequestSerial;
    toolPage.value = undefined;
    toolLoading.value = false;
    toolError.value = "";
    attempts.value = [];
    nativeStatus.value = undefined;
    historicalPreviews.value = [];
    learningCandidates.value = [];
    knowledge.value = [];
    detailErrors.value = [];
    detailLoading.value = false;
    if (!scanId) return;
    detailLoading.value = true;
    const isCurrent = () => !disposed && serial === detailRequestSerial && props.preview?.id === scanId;
    // Publish independent sections as they settle; a slow optional status read must
    // not hide completed history/evidence reads. Also catch synchronous IPC failures.
    async function readDetail<T>(read: () => Promise<T>, publish: (value: T) => void, error?: string) {
      try {
        const value = await read();
        if (isCurrent()) publish(value);
      } catch {
        if (isCurrent() && error) detailErrors.value.push(error);
      }
    }
    await Promise.all([
      readDetail(() => sentinelApi.listSentinelScanAttempts(scanId), (value) => {
        attempts.value = value;
        if (!attempts.value.some((attempt) => attempt.attemptNumber === selectedAttempt.value)) {
          selectedAttempt.value = attempts.value[0]?.attemptNumber;
        }
        if (detailTab.value === "execution" && selectedAttempt.value !== undefined) {
          void loadAttemptLog(scanId, selectedAttempt.value);
          void loadMailbox(scanId, selectedAttempt.value);
          void loadTools(scanId, selectedAttempt.value);
        }
      }, "执行历史暂时不可读取"),
      // Draft or non-Web tasks may not have a Native run; absence is not a history error.
      readDetail(() => sentinelApi.getNativeScanStatus(scanId), (value) => { nativeStatus.value = value; }),
      readDetail(() => knowledgeCatalogApi.listAgentLearningCandidates(undefined, scanId),
        (value) => { learningCandidates.value = value; }, "学习候选暂时不可读取"),
      readDetail(() => knowledgeCatalogApi.listAgentKnowledge(scanId),
        (value) => { knowledge.value = value; }, "知识记录暂时不可读取"),
      readDetail(() => sentinelApi.listHistoricalImportPreviews(scanId),
        (value) => { historicalPreviews.value = validateHistoricalPreviews(value, scanId); }, "历史导入预览暂时不可读取"),
    ]);
    if (isCurrent()) detailLoading.value = false;
  }, { immediate: true });
  watch(detailTab, (tab) => {
    if (tab === "execution" && props.preview && selectedAttempt.value !== undefined
      && !runnerLog.value && !runnerLogLoading.value && !runnerLogError.value) {
      void loadAttemptLog(props.preview.id, selectedAttempt.value);
    }
    if (tab === "execution" && props.preview && selectedAttempt.value !== undefined
      && !mailboxPage.value && !mailboxLoading.value && !mailboxError.value) {
      void loadMailbox(props.preview.id, selectedAttempt.value);
    }
    if (tab === "execution" && props.preview && selectedAttempt.value !== undefined
      && !toolPage.value && !toolLoading.value && !toolError.value) {
      void loadTools(props.preview.id, selectedAttempt.value);
    }
  });
  return {
    detailTab, detailLoading, detailErrors, attempts, selectedAttempt,
    runnerLog, runnerLogLoading, runnerLogError, runnerLogConnectionError, mailboxPage, mailboxLoading, mailboxError,
    toolPage, toolLoading, toolError, nativeStatus, historicalPreviews, learningCandidates, knowledge,
    loadTools, loadMailbox, loadAttemptLog, chooseAttempt,
  };
}
