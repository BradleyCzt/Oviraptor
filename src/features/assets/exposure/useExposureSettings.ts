// §E2 — the exposure collection settings editor. It only reads and writes the exposure
// surface's own settings; nothing here starts a collection, and a save that is refused stays
// unsaved instead of being reported as applied.

import { computed, ref } from "vue";
import {
  exposureErrorText,
  isUnregisteredCommand,
  formatDocumentUrls,
  parseDocumentUrls,
  parseOwnerList,
  readDocumentUrls,
  readSourceCapabilities,
  type JsonLike,
  type SourceCapability,
} from "./contracts";
import type { ExposureSettingsView, ExposureSettingVersionPage } from "./api";

export interface ExposureSettingsApi {
  getExposureSettings?: (projectId: number) => Promise<unknown>;
  saveExposureSettings?: (projectId: number, expectedVersion: number, settings: JsonLike) => Promise<number>;
  listExposureSettingVersions?: (projectId: number, limit: number) => Promise<unknown>;
}

export interface ExposureSettingsDeps {
  projectId: () => number | undefined;
  api: ExposureSettingsApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
}

export type SettingsNumberKey =
  | "exposureMaxItems"
  | "exposureMaxRequests"
  | "exposureMaxResponseBytes"
  | "exposureMaxTotalBytes"
  | "exposureTimeoutSeconds"
  | "exposureCtPageSize"
  | "exposureArchivePageSize"
  | "exposureAuthorizationDays";

const EDITABLE: readonly SettingsNumberKey[] = [
  "exposureMaxRequests",
  "exposureMaxResponseBytes",
  "exposureMaxTotalBytes",
  "exposureMaxItems",
  "exposureTimeoutSeconds",
  "exposureCtPageSize",
  "exposureArchivePageSize",
  "exposureAuthorizationDays",
] as const;

export function useExposureSettings(deps: ExposureSettingsDeps) {
  const current = ref<ExposureSettingsView | null>(null);
  const draft = ref<JsonLike>({});
  const history = ref<ExposureSettingVersionPage | null>(null);
  const loading = ref(false);
  const saving = ref(false);
  const error = ref("");
  const conflict = ref(false);
  const available = ref(true);
  const dirty = ref(false);

  let generation = 0;
  /// FIX4 P1／R4 草稿属于哪个项目（读取成功时绑定）。加载中、切换后尚未读完或读取失败时，草稿不属于当前项目，禁止保存。
  const draftProject = ref(0);

  const bounds = computed<JsonLike>(() => current.value?.bounds ?? {});
  const isDefault = computed(() => current.value?.saved === false);
  const versionLabel = computed(() =>
    current.value
      ? isDefault.value
        ? `默认配置（未保存，服务器版本 v${current.value.storedVersion}）`
        : `已保存 v${current.value.version}`
      : "尚未读取配置",
  );
  const sources = computed<string[]>(() => current.value?.exposureSources ?? []);
  const allowedSources = computed<string[]>(
    () => ((bounds.value.sources as string[] | undefined) ?? []),
  );
  /** G2: `null` = the backend reports no capability list (shown as integration_pending). */
  const capabilities = computed<SourceCapability[] | null>(() =>
    readSourceCapabilities(current.value?.sourceCapabilities),
  );
  const ownerText = ref("");
  const ownerInvalid = ref<string[]>([]);
  const documentText = ref("");
  const documentInvalid = ref<string[]>([]);
  const credentialRef = ref("");

  function discarded(claimed: number, projectId: number): boolean {
    return claimed !== generation || deps.projectId() !== projectId;
  }

  function readSettings(value: unknown): ExposureSettingsView {
    if (typeof value !== "object" || value === null) throw new Error("配置返回结构无效");
    const row = value as Record<string, unknown>;
    if (typeof row.exposureMaxRequests !== "number" || !Array.isArray(row.exposureSources)) {
      throw new Error("配置缺少预算或来源字段");
    }
    if (typeof row.version !== "number") throw new Error("配置缺少版本号");
    return value as ExposureSettingsView;
  }

  async function load(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) return;
    if (!deps.api.getExposureSettings) {
      available.value = false;
      error.value = "";
      return;
    }
    const claimed = ++generation;
    loading.value = true;
    if (draftProject.value !== projectId) draftProject.value = 0;
    try {
      const view = readSettings(await deps.api.getExposureSettings(projectId));
      if (discarded(claimed, projectId)) return;
      current.value = view;
      draftProject.value = projectId;
      available.value = true;
      conflict.value = false;
      error.value = "";
      dirty.value = false;
      draft.value = {
        exposureSources: [...view.exposureSources],
        exposureAllowPrivateDestinations: view.exposureAllowPrivateDestinations,
        exposureGithubOfficialOwners: Array.isArray(view.exposureGithubOfficialOwners)
          ? [...(view.exposureGithubOfficialOwners as string[])]
          : [],
        exposureDocumentUrls: readDocumentUrls(view.exposureDocumentUrls) as unknown as JsonLike,
      };
      documentText.value = formatDocumentUrls(readDocumentUrls(view.exposureDocumentUrls));
      documentInvalid.value = [];
      ownerText.value = (draft.value.exposureGithubOfficialOwners as string[]).join("\n");
      ownerInvalid.value = [];
      credentialRef.value = typeof view.exposureGithubCredentialRef === "string" ? view.exposureGithubCredentialRef : "";
      draft.value.exposureGithubCredentialRef = credentialRef.value;
      for (const key of EDITABLE) draft.value[key] = view[key];
    } catch (reason) {
      if (discarded(claimed, projectId)) return;
      if (isUnregisteredCommand(reason)) available.value = false;
      else error.value = exposureErrorText(reason);
    } finally {
      if (!discarded(claimed, projectId)) loading.value = false;
    }
  }

  function setNumber(key: SettingsNumberKey, raw: unknown): void {
    const next = Number(raw);
    draft.value[key] = Number.isFinite(next) ? next : raw;
    dirty.value = true;
  }

  function toggleSource(name: string): void {
    const rows = Array.isArray(draft.value.exposureSources)
      ? (draft.value.exposureSources as string[])
      : [];
    draft.value.exposureSources = rows.includes(name)
      ? rows.filter(row => row !== name)
      : [...rows, name];
    dirty.value = true;
  }

  /** G2: the human-maintained list of official open-source GitHub owners. */
  function setOfficialOwners(text: string): void {
    ownerText.value = text;
    const parsed = parseOwnerList(text);
    ownerInvalid.value = parsed.invalid;
    draft.value.exposureGithubOfficialOwners = parsed.owners;
    dirty.value = true;
  }

  /** G3: explicit public document URLs, one per line; `URL | 理由` declares public material. */
  function setDocumentUrls(text: string): void {
    documentText.value = text;
    const parsed = parseDocumentUrls(text);
    documentInvalid.value = parsed.invalid;
    draft.value.exposureDocumentUrls = parsed.entries as unknown as JsonLike;
    dirty.value = true;
  }

  function setPrivateEgress(on: boolean): void {
    draft.value.exposureAllowPrivateDestinations = on;
    dirty.value = true;
  }

  function setCredentialRef(text: string): void {
    credentialRef.value = text;
    draft.value.exposureGithubCredentialRef = text.trim();
    dirty.value = true;
  }

  async function save(): Promise<boolean> {
    const projectId = deps.projectId();
    const view = current.value;
    if (!projectId || !view || saving.value) return false;
    if (loading.value || draftProject.value !== projectId) {
      error.value = "配置仍在读取，或草稿不属于当前项目；未保存，请等读取完成后再编辑保存";
      return false;
    }
    if (ownerInvalid.value.length) {
      error.value = `正式开源账号格式不合规：${ownerInvalid.value.join("、")}，未保存`;
      return false;
    }
    if (documentInvalid.value.length) {
      error.value = `文档 URL 不合规：${documentInvalid.value.join("、")}，未保存`;
      return false;
    }
    if (!deps.api.saveExposureSettings) {
      error.value = "配置保存接口尚未注册，未做任何修改";
      return false;
    }
    saving.value = true;
    try {
      const next = await deps.api.saveExposureSettings(
        projectId,
        view.storedVersion,
        { ...draft.value, version: view.version },
      );
      if (deps.projectId() !== projectId) return false;
      conflict.value = false;
      error.value = "";
      deps.notify("success", `采集配置已保存为 v${next}，只对之后新建的轮次生效`);
      await load();
      return true;
    } catch (reason) {
      if (deps.projectId() !== projectId) return false;
      const text = exposureErrorText(reason);
      // A version clash is not a validation failure: keep the editor content and say why.
      conflict.value = reason instanceof Error && reason.message.includes("exposure_setting_conflict");
      error.value = conflict.value
        ? "配置已被别处改动，本次未保存。请重新读取后再编辑。"
        : text || "配置校验未通过，未保存";
      deps.notify("error", conflict.value ? "配置版本冲突，未保存" : "配置保存失败，未做任何修改");
      return false;
    } finally {
      saving.value = false;
    }
  }

  async function loadHistory(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !deps.api.listExposureSettingVersions) return;
    try {
      const raw = await deps.api.listExposureSettingVersions(projectId, 20);
      if (typeof raw !== "object" || raw === null || !Array.isArray((raw as JsonLike).items)) {
        throw new Error("配置历史返回结构无效");
      }
      history.value = raw as unknown as ExposureSettingVersionPage;
    } catch (reason) {
      error.value = exposureErrorText(reason);
    }
  }

  /// G1-0 the executor is serial: the saved concurrency is kept but has no effect.
  const concurrencyNotice = computed(() => {
    const effective = current.value?.effective as Record<string, unknown> | undefined;
    if (!effective) return "并行度：后端未报告实际执行方式";
    return `并行度已从采集合同移除，执行器串行（${String(effective.concurrencyNote ?? "同一时刻只处理一个工作项")}）`;
  });

  const notice = computed(() => {
    if (!deps.projectId()) return "请先选择工作空间";
    if (!available.value) return "配置接口尚未注册，页面显示的不是可保存的真实值";
    if (error.value) return error.value;
    return isDefault.value
      ? "当前是代码默认值；保存后才会成为该工作空间的设定，且只影响之后新建的轮次"
      : "改动只会冻结到之后新建的轮次，进行中和已中断的轮次仍按各自冻结配置执行";
  });

  return {
    current,
    draft,
    history,
    loading,
    draftProject,
    saving,
    error,
    conflict,
    available,
    dirty,
    bounds,
    isDefault,
    versionLabel,
    sources,
    allowedSources,
    capabilities,
    ownerText,
    ownerInvalid,
    documentText,
    documentInvalid,
    notice,
    concurrencyNotice,
    load,
    loadHistory,
    save,
    setNumber,
    setPrivateEgress,
    setOfficialOwners,
    setDocumentUrls,
    toggleSource,
    credentialRef,
    setCredentialRef,
  };
}
