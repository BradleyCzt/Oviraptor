import { computed, onUnmounted, ref, watch } from "vue";
import type { ConversationApi } from "./api";
import type { ConversationAccess, ConversationAccessInput, ConversationAccessSkill, ConversationScope } from "./types";

/** Grants are backend-owned. Reads, selection changes, and model text never grant access. */
export function useConversationAccess(deps: {
  scope: () => ConversationScope | undefined;
  writable: () => boolean;
  replyActive: () => boolean;
  blocked: () => boolean;
  api: ConversationApi;
  chooseDirectory: () => Promise<string | null>;
}) {
  const access = ref<ConversationAccess>();
  const skills = ref<ConversationAccessSkill[]>([]);
  const freshAccess = ref(false);
  const loading = ref(false), saving = ref(false), choosing = ref(false);
  const error = ref(""), skillsError = ref("");
  const disclosureAccepted = ref(false);
  const stagedDirectory = ref("");
  let epoch = 0, request = 0, disposed = false;
  const scope = computed(deps.scope);
  const busy = computed(() => saving.value || choosing.value);
  const canExpand = computed(() => Boolean(scope.value && access.value && deps.writable() && !deps.replyActive() && !deps.blocked() && !loading.value && !busy.value && freshAccess.value));
  const canRevoke = computed(() => Boolean(scope.value && access.value && deps.writable() && !loading.value && !busy.value && freshAccess.value));
  const current = (generation: number, input: ConversationScope) => !disposed && generation === epoch && scope.value?.projectId === input.projectId && scope.value?.conversationId === input.conversationId;
  function validate(value: ConversationAccess, input: ConversationScope) {
    if (!value || value.projectId !== input.projectId || value.conversationId !== input.conversationId || typeof value.directorySupported !== "boolean" || typeof value.agentEnabled !== "boolean" || value.version !== 1 || !Number.isSafeInteger(value.revision) || value.revision < 1 || typeof value.readAssets !== "boolean" || typeof value.readScans !== "boolean" || typeof value.providerDisclosureAccepted !== "boolean" || typeof value.skillsRequireReview !== "boolean" || typeof value.skillRevision !== "string" || !value.skillRevision || (value.providerIdentity !== null && typeof value.providerIdentity !== "string") || (value.providerLabel !== null && typeof value.providerLabel !== "string") || !Array.isArray(value.selectedSkillIds) || value.selectedSkillIds.some(id => !Number.isSafeInteger(id) || id < 1) || !Array.isArray(value.directories) || value.directories.some(item => !item || typeof item.id !== "string" || !item.id || typeof item.path !== "string" || typeof item.label !== "string")) throw Error("Invalid conversation access response; permissions have not been assumed");
  }
  async function refresh() {
    const input = scope.value;
    if (!input) return;
    const generation = epoch, read = ++request;
    loading.value = true;
    try {
      const value = await deps.api.getConversationAccess({ ...input });
      if (!current(generation, input) || read !== request) return;
      validate(value, input);
      const sameConsent = access.value?.providerIdentity === value.providerIdentity && access.value?.skillRevision === value.skillRevision;
      disclosureAccepted.value = value.providerDisclosureAccepted || sameConsent && disclosureAccepted.value;
      freshAccess.value = true; access.value = value; error.value = "";
    } catch (failure) { if (current(generation, input) && read === request) { freshAccess.value = false; error.value = String(failure); } }
    finally { if (current(generation, input) && read === request) loading.value = false; }
  }
  async function loadSkills() {
    const input = scope.value;
    if (!input) return;
    const generation = epoch;
    try {
      const value = await deps.api.listConversationAccessSkills({ ...input });
      if (!current(generation, input)) return;
      if (!Array.isArray(value) || value.some(skill => !Number.isSafeInteger(skill.id) || typeof skill.name !== "string" || typeof skill.description !== "string" || typeof skill.enabled !== "boolean")) throw Error("Invalid skills catalog");
      skills.value = value; skillsError.value = "";
    } catch (failure) { if (current(generation, input)) skillsError.value = String(failure); }
  }
  async function mutate(action: (input: ConversationScope, value: ConversationAccess) => Promise<ConversationAccess>): Promise<boolean> {
    const input = scope.value, previous = access.value;
    if (!input || !previous || busy.value || loading.value || !deps.writable()) return false;
    const generation = epoch;
    saving.value = true; error.value = ""; request++;
    try {
      const value = await action({ ...input }, previous);
      if (!current(generation, input)) return false;
      validate(value, input); freshAccess.value = true; access.value = value; disclosureAccepted.value = value.providerDisclosureAccepted; stagedDirectory.value = "";
      return true;
    } catch (failure) {
      if (current(generation, input)) {
        // Reconcile through a read, never replay an uncertain grant automatically.
        await refresh();
        if (current(generation, input)) error.value = String(failure);
      }
      return false;
    } finally { if (current(generation, input)) saving.value = false; }
  }
  async function setTools(change: Partial<Pick<ConversationAccess, "agentEnabled" | "readAssets" | "readScans" | "selectedSkillIds">>) {
    const value = access.value;
    if (!value) return;
    const desired = { agentEnabled: value.agentEnabled, readAssets: value.readAssets, readScans: value.readScans, selectedSkillIds: [...value.selectedSkillIds], ...change };
    const expanding = desired.agentEnabled && !value.agentEnabled || desired.readAssets && !value.readAssets || desired.readScans && !value.readScans || desired.selectedSkillIds.some(id => !value.selectedSkillIds.includes(id)) || disclosureAccepted.value && !value.providerDisclosureAccepted;
    if (expanding ? !canExpand.value : !canRevoke.value) return;
    if (expanding && (!disclosureAccepted.value || !value.providerIdentity)) { error.value = "Confirm that your configured model provider may receive the selected tool results before enabling access"; return; }
    await mutate((input, fresh) => {
      const payload: ConversationAccessInput = { ...input, expectedRevision: fresh.revision, ...desired, providerDisclosureAccepted: expanding ? disclosureAccepted.value : fresh.providerDisclosureAccepted, providerIdentity: fresh.providerIdentity, expectedSkillRevision: fresh.skillRevision };
      return deps.api.setConversationAccess(payload);
    });
  }
  async function setSkill(id: number, enabled: boolean) {
    const value = access.value;
    if (!value) return;
    const skill = skills.value.find(item => item.id === id);
    if (enabled && (!skill || !skill.enabled)) return;
    await setTools({ selectedSkillIds: enabled ? [...new Set([...value.selectedSkillIds, id])] : value.selectedSkillIds.filter(item => item !== id) });
  }
  async function chooseDirectory() {
    if (!canExpand.value || !scope.value || !access.value?.directorySupported) return;
    const input = { ...scope.value }, generation = epoch, revision = access.value.revision;
    choosing.value = true; error.value = "";
    try {
      const path = await deps.chooseDirectory();
      if (!current(generation, input) || access.value?.revision !== revision || !deps.writable() || deps.replyActive()) return;
      if (typeof path === "string" && path) stagedDirectory.value = path;
    } catch (failure) { if (current(generation, input)) error.value = String(failure); }
    finally { if (current(generation, input)) choosing.value = false; }
  }
  async function grantDirectory() {
    if (!canExpand.value || !access.value?.directorySupported || !stagedDirectory.value) return;
    if (!disclosureAccepted.value) { error.value = "Confirm that files read from this directory may be sent to your configured model provider"; return; }
    const path = stagedDirectory.value;
    await mutate((input, value) => deps.api.grantConversationDirectory({ ...input, expectedRevision: value.revision, path, providerDisclosureAccepted: true, providerIdentity: value.providerIdentity, expectedSkillRevision: value.skillRevision }));
  }
  async function revokeDirectory(grantId: string) {
    if (!canRevoke.value || !access.value?.directories.some(item => item.id === grantId)) return;
    await mutate((input, value) => deps.api.revokeConversationDirectory({ ...input, expectedRevision: value.revision, grantId }));
  }
  function clearStagedDirectory() { if (!busy.value) stagedDirectory.value = ""; }
  watch(() => `${scope.value?.projectId ?? ""}:${scope.value?.conversationId ?? ""}`, () => {
    epoch++; request++; freshAccess.value = false; access.value = undefined; skills.value = []; loading.value = false; saving.value = false; choosing.value = false;
    error.value = ""; skillsError.value = ""; stagedDirectory.value = ""; disclosureAccepted.value = false;
    void refresh(); void loadSkills();
  }, { immediate: true, flush: "sync" });
  onUnmounted(() => { disposed = true; epoch++; request++; });
  return { access, skills, loading, saving, choosing, busy, error, skillsError, disclosureAccepted, stagedDirectory, canExpand, canRevoke, refresh, loadSkills, setTools, setSkill, chooseDirectory, grantDirectory, revokeDirectory, clearStagedDirectory };
}
