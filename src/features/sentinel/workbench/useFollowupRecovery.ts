import { ref, watch, type Ref } from "vue";
import { sentinelApi } from "../api";
import type { GapFollowupDraftInput, GapFollowupPreview, GapFollowupSubmission } from "../types";

type RecoveryForm = { urls: string; taskName: string };

export function useFollowupRecovery(options: {
  source: () => GapFollowupPreview | undefined;
  generation: () => number;
  isDisposed: () => boolean;
  form: RecoveryForm;
  busy: Ref<boolean>;
  resetIdentitySelection: () => void;
  notify: (type: "success" | "error" | "info", text: string) => void;
}) {
  const { source, generation, isDisposed, form, busy, resetIdentitySelection, notify } = options;
  const pendingFollowupInput = ref<GapFollowupDraftInput>();
  const followupSubmission = ref<GapFollowupSubmission>();
  const followupRecoveryBusy = ref(false);
  const followupRecoveryError = ref("");
  const showReleaseFollowup = ref(false);
  let recoveryEpoch = 0;

  async function restoreFollowupSubmission(): Promise<boolean> {
    const currentSource = source();
    const epoch = ++recoveryEpoch;
    const viewGeneration = generation();
    if (!currentSource) {
      followupRecoveryBusy.value = false;
      followupRecoveryError.value = "";
      followupSubmission.value = undefined;
      return true;
    }
    followupRecoveryBusy.value = true;
    followupRecoveryError.value = "";
    const current = () => !isDisposed() && epoch === recoveryEpoch && viewGeneration === generation();
    try {
      const saved = await sentinelApi.getAgentGapFollowupSubmission(currentSource.sourceScanId, currentSource.assessmentMessageId);
      if (!current()) return false;
      if (saved && (saved.input.sourceScanId !== currentSource.sourceScanId || saved.input.assessmentMessageId !== currentSource.assessmentMessageId))
        throw new Error("followup_submission_source_mismatch");
      followupSubmission.value = saved || undefined;
      // An uncertain submission from another source is kept until its owner is revisited.
      const pending = pendingFollowupInput.value;
      if (saved && (!pending || (pending.sourceScanId === currentSource.sourceScanId && pending.assessmentMessageId === currentSource.assessmentMessageId)))
        pendingFollowupInput.value = saved.input;
      return true;
    } catch (error) {
      if (current()) followupRecoveryError.value = String(error);
      return false;
    } finally {
      if (epoch === recoveryEpoch) followupRecoveryBusy.value = false;
    }
  }

  async function releaseFollowupSubmission() {
    const saved = followupSubmission.value;
    if (!saved || busy.value || isDisposed()) return;
    const viewGeneration = generation();
    busy.value = true;
    try {
      await sentinelApi.releaseAgentGapFollowupSubmission(saved.input.requestId, saved.createdScanId || saved.scan?.id);
      if (pendingFollowupInput.value?.requestId === saved.input.requestId) pendingFollowupInput.value = undefined;
      if (isDisposed() || viewGeneration !== generation()) return;
      followupSubmission.value = undefined;
      showReleaseFollowup.value = false;
      form.urls = source()?.targetUrl || "";
      form.taskName = "补充证据任务";
      resetIdentitySelection();
      await restoreFollowupSubmission();
    } catch (error) {
      // A lost response may follow a committed release; reconcile before another save.
      if (pendingFollowupInput.value?.requestId === saved.input.requestId) pendingFollowupInput.value = undefined;
      if (!isDisposed() && viewGeneration === generation()) {
        notify("error", String(error));
        await restoreFollowupSubmission();
      }
    } finally { busy.value = false; }
  }

  watch(() => [source()?.sourceScanId, source()?.assessmentMessageId, source()?.sourceHash], () => {
    showReleaseFollowup.value = false;
    followupSubmission.value = undefined;
    void restoreFollowupSubmission();
  }, { immediate: true, flush: "sync" });

  return { pendingFollowupInput, followupSubmission, followupRecoveryBusy,
    followupRecoveryError, showReleaseFollowup, restoreFollowupSubmission, releaseFollowupSubmission };
}
