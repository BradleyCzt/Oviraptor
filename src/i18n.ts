import { computed, ref } from "vue";
import { assetMessages } from "./features/assets/messages";
import { configurationMessages } from "./features/configuration/messages";
import { workspaceMessages } from "./features/workspaces/messages";
import { commonMessages } from "./shared/i18n/messages";
import { shellMessages } from "./shell/messages";

export type Locale = "zh" | "en";
if (localStorage.getItem("oviraptor-locale") === null && localStorage.getItem("asset-atlas-locale") !== null) {
  localStorage.setItem("oviraptor-locale", localStorage.getItem("asset-atlas-locale")!);
}
localStorage.removeItem("asset-atlas-locale");
export const locale = ref<Locale>((localStorage.getItem("oviraptor-locale") as Locale) || "zh");

function messagesFor(value: Locale) {
  // Explicit precedence, from lowest to highest: shell, assets, workspaces,
  // configuration, common. Current resources have disjoint keys; later resources
  // win if a key is intentionally introduced in more than one owner.
  const merged = {
    ...shellMessages[value],
    ...assetMessages[value],
    ...workspaceMessages[value],
    ...configurationMessages[value],
    ...commonMessages[value],
  };
  // Preserve the public key order as well as each label. Callers keep the same t API.
  return {
    appName: merged.appName, dashboard: merged.dashboard, projects: merged.projects,
    query: merged.query, assets: merged.assets, quarantine: merged.quarantine,
    changes: merged.changes, tasks: merged.tasks, logs: merged.logs,
    settings: merged.settings, allProjects: merged.allProjects, search: merged.search,
    newProject: merged.newProject, newQuery: merged.newQuery, export: merged.export,
    refresh: merged.refresh, save: merged.save, cancel: merged.cancel,
    confirm: merged.confirm, close: merged.close, empty: merged.empty,
  } as const;
}

const messages = { zh: messagesFor("zh"), en: messagesFor("en") } as const;

export function useI18n() {
  const t = computed(() => messages[locale.value]);
  const tr = (zh: string, en: string) => locale.value === "zh" ? zh : en;
  const setLocale = (value: Locale) => {
    locale.value = value;
    localStorage.setItem("oviraptor-locale", value);
  };
  return { locale, t, tr, setLocale };
}
