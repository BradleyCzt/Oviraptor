import { computed, onUnmounted, ref, watch } from "vue";
import type { BuiltinPluginHost, PluginLinkCandidate, PluginLinkCursor, PluginNavigationIntent } from "../../plugins";
import type { ConversationLink, ConversationLinksInput, ConversationSnapshot } from "./types";

// Human-operated navigation and typed references only. The model never receives
// plugin operations, candidate data, or permission to execute a scan.
export function useConversationPlugins(deps: {
  host: () => BuiltinPluginHost;
  snapshot: () => ConversationSnapshot | undefined;
  projectId: () => number | undefined;
  projectActive: () => boolean;
  blocked: () => boolean;
  save: (input: ConversationLinksInput) => Promise<boolean>;
}) {
  const expanded = ref(false);
  const editing = ref(false), pluginId = ref<"asset" | "sentinel">("asset"), query = ref("");
  const candidates = ref<PluginLinkCandidate[]>([]), draftLinks = ref<ConversationLink[]>([]);
  const nextCursor = ref<PluginLinkCursor>(), loading = ref(false), saving = ref(false), error = ref("");
  const anchor = ref<{ projectId: number; conversationId: string; expectedRevision: number }>();
  let request = 0, epoch = 0, disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const key = (link: { kind: string; targetId: string }) => `${link.kind}:${link.targetId}`;
  const scope = () => {
    const value = deps.snapshot()?.conversation;
    return value && value.projectId === deps.projectId() ? `${value.projectId}:${value.id}` : "";
  };
  const canEdit = computed(() => Boolean(scope()) && deps.projectActive() && !deps.snapshot()?.conversation.archived && !deps.blocked() && !saving.value);
  const stale = computed(() => Boolean(anchor.value && deps.snapshot()?.conversation.revision !== anchor.value.expectedRevision));
  const hasUnavailable = computed(() => draftLinks.value.some(link => !link.available));
  const changed = computed(() => {
    const existing = (deps.snapshot()?.links ?? []).map(key).sort();
    return JSON.stringify(existing) !== JSON.stringify(draftLinks.value.map(key).sort());
  });
  const disabledNewLinks = computed(() => draftLinks.value.some(link =>
    !(deps.snapshot()?.links ?? []).some(existing => key(existing) === key(link)) &&
    !deps.host().isEnabled(link.kind === "asset" ? "asset" : "sentinel")));
  const canSave = computed(() => editing.value && canEdit.value && !stale.value && changed.value && !hasUnavailable.value && !disabledNewLinks.value && draftLinks.value.length <= 50);
  function cancelLookup() {
    request++;
    if (timer) clearTimeout(timer);
    timer = undefined;
    loading.value = false;
  }
  function clearCandidates() { cancelLookup(); candidates.value = []; nextCursor.value = undefined; }
  function current(capturedEpoch: number, capturedRequest: number, capturedScope: string, plugin: "asset" | "sentinel", search: string) {
    return !disposed && editing.value && epoch === capturedEpoch && request === capturedRequest && scope() === capturedScope && pluginId.value === plugin && query.value.trim() === search && deps.host().isEnabled(plugin);
  }
  async function loadCandidates(append = false) {
    const saved = anchor.value, plugin = pluginId.value;
    if (!editing.value || !saved || saved.projectId !== deps.projectId() || !deps.host().isEnabled(plugin) || saving.value || (append && (!nextCursor.value || loading.value))) return;
    const capturedScope = scope(), capturedEpoch = epoch, capturedRequest = ++request, search = query.value.trim();
    const cursor = append ? nextCursor.value : undefined;
    if (!append) { candidates.value = []; nextCursor.value = undefined; }
    loading.value = true; error.value = "";
    try {
      const page = await deps.host().listLinkCandidates({ pluginId: plugin, projectId: saved.projectId, search, cursor });
      if (!current(capturedEpoch, capturedRequest, capturedScope, plugin, search)) return;
      const kind = plugin === "asset" ? "asset" : "scan";
      if (page.items.some(item => item.projectId !== saved.projectId || item.kind !== kind || !item.targetId)) throw Error("Plugin records do not match this workspace");
      const combined = append ? [...candidates.value, ...page.items] : page.items;
      candidates.value = combined.filter((item, index) => combined.findIndex(other => key(other) === key(item)) === index);
      nextCursor.value = page.nextCursor;
    } catch (failure) {
      if (current(capturedEpoch, capturedRequest, capturedScope, plugin, search)) error.value = String(failure);
    } finally { if (current(capturedEpoch, capturedRequest, capturedScope, plugin, search)) loading.value = false; }
  }
  function togglePanel() { expanded.value = !expanded.value; }
  function openEditor(plugin?: "asset" | "sentinel") {
    if (!canEdit.value) return;
    const value = deps.snapshot()!;
    expanded.value = true;
    editing.value = false; epoch++; clearCandidates();
    anchor.value = { projectId: value.conversation.projectId, conversationId: value.conversation.id, expectedRevision: value.conversation.revision };
    draftLinks.value = value.links.map(link => ({ ...link }));
    pluginId.value = plugin ?? (deps.host().isEnabled("asset") ? "asset" : "sentinel");
    query.value = ""; error.value = ""; editing.value = true;
    void loadCandidates();
  }
  function closeEditor() {
    if (saving.value) return;
    editing.value = false; epoch++; clearCandidates(); anchor.value = undefined; draftLinks.value = []; error.value = "";
  }
  function selectPlugin(plugin: "asset" | "sentinel") {
    if (plugin === pluginId.value || !deps.host().isEnabled(plugin) || saving.value) return;
    clearCandidates(); pluginId.value = plugin; query.value = ""; error.value = "";
    if (timer) clearTimeout(timer); timer = undefined;
    void loadCandidates();
  }
  function isSelected(item: { kind: string; targetId: string }) { return draftLinks.value.some(link => key(link) === key(item)); }
  function addCandidate(item: PluginLinkCandidate) {
    if (!editing.value || !canEdit.value || stale.value || !deps.host().isEnabled(pluginId.value) || item.projectId !== anchor.value?.projectId || item.kind !== (pluginId.value === "asset" ? "asset" : "scan")) return;
    if (!candidates.value.some(candidate => key(candidate) === key(item)) || isSelected(item)) return;
    if (draftLinks.value.length >= 50) { error.value = "A conversation can link at most 50 records"; return; }
    draftLinks.value = [...draftLinks.value, { kind: item.kind, targetId: item.targetId, label: item.label, available: true }];
  }
  function removeLink(item: { kind: string; targetId: string }) {
    if (!editing.value || !canEdit.value || stale.value) return;
    draftLinks.value = draftLinks.value.filter(link => key(link) !== key(item));
  }
  async function save() {
    if (!canSave.value || !anchor.value) return;
    const capturedEpoch = epoch, capturedScope = scope();
    const input = { ...anchor.value, links: draftLinks.value.map(({ kind, targetId }) => ({ kind, targetId })) };
    saving.value = true; error.value = "";
    try {
      const saved = await deps.save(input);
      if (disposed || epoch !== capturedEpoch || scope() !== capturedScope) return;
      if (saved) { saving.value = false; closeEditor(); }
      else error.value = "Links were not saved. Refresh the conversation and review your selection before trying again";
    } catch (failure) { if (!disposed && epoch === capturedEpoch && scope() === capturedScope) error.value = String(failure); }
    finally { if (!disposed && epoch === capturedEpoch && scope() === capturedScope) saving.value = false; }
  }
  function openWorkspace(plugin: "asset" | "sentinel") {
    const projectId = deps.projectId();
    if (!projectId || !deps.host().isEnabled(plugin)) return;
    const intent: PluginNavigationIntent = plugin === "asset" ? { pluginId: "asset", projectId, page: "assets" } : { pluginId: "sentinel", projectId, page: "sentinel", section: "queue" };
    try { deps.host().openWorkspace(intent); }
    catch (failure) { error.value = String(failure); }
  }
  watch(() => `${deps.projectId() ?? ""}:${scope()}`, () => {
    expanded.value = false;
    epoch++; cancelLookup(); editing.value = false; saving.value = false; anchor.value = undefined; candidates.value = []; draftLinks.value = []; nextCursor.value = undefined; error.value = "";
  }, { flush: "sync" });
  watch(() => [deps.host().isEnabled("asset"), deps.host().isEnabled("sentinel")], () => {
    if (!deps.host().isEnabled(pluginId.value)) clearCandidates();
  }, { flush: "sync" });
  watch(query, () => {
    if (!editing.value) return;
    clearCandidates(); error.value = "";
    timer = setTimeout(() => { timer = undefined; void loadCandidates(); }, 250);
  }, { flush: "sync" });
  onUnmounted(() => { disposed = true; epoch++; cancelLookup(); });
  return { expanded, togglePanel, editing, pluginId, query, candidates, draftLinks, nextCursor, loading, saving, error, stale, hasUnavailable, disabledNewLinks, changed, canEdit, canSave, isSelected, openEditor, closeEditor, selectPlugin, loadCandidates, addCandidate, removeLink, save, openWorkspace };
}
