import { ref, watch, type Ref } from "vue";
import { sentinelApi } from "../api";
import type { AgentTimelineItem, GapFollowupPreview, NativeScanStatus } from "../types";

type TaskView = { scanId: string; isCurrent: () => boolean };

// Preview is read-only, yet it opens an actionable draft: never publish a
// preview from a previous task, attempt, or invalidated status snapshot.
export function useAgentDialogFollowup(options: {
  projectId: () => number | undefined;
  scanId: Ref<string>;
  state: Ref<NativeScanStatus | undefined>;
  captureTaskView: () => TaskView;
  actionError: Ref<string>;
  onPreview: (preview: GapFollowupPreview) => void;
  tr: (zh: string, en: string) => string;
}) {
  const { projectId, scanId, state, captureTaskView, actionError, onPreview, tr } = options;
  const preparingFollowup = ref(false);
  let operationSerial = 0;
  async function prepareFollowup(item: AgentTimelineItem) {
    if (preparingFollowup.value || !item.gapAssessment?.newAttemptRequired) return;
    const view = captureTaskView();
    const serial = ++operationSerial;
    const isCurrent = () => view.isCurrent() && serial === operationSerial;
    preparingFollowup.value = true;
    actionError.value = "";
    try {
      const preview = await sentinelApi.previewAgentGapFollowup(view.scanId, item.id);
      if (isCurrent()) onPreview(preview);
    } catch {
      if (isCurrent()) actionError.value = tr("无法准备补充任务预览；请刷新核对证据是否缺失或变化。本操作未创建新任务。", "Unable to prepare the follow-up preview. Refresh and check for missing or changed evidence; this action did not create a new task.");
    } finally {
      if (isCurrent()) preparingFollowup.value = false;
    }
  }
  watch([projectId, scanId, () => state.value?.attemptNumber], () => {
    operationSerial++;
    preparingFollowup.value = false;
  }, { flush: "sync" });
  return { preparingFollowup, prepareFollowup };
}
