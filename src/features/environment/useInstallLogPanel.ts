import { onMounted, onBeforeUnmount, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { environmentApi } from "./api";
import type { InstallLogPage, InstallLogRow } from "./types";

const streams = new Set(["status", "stdout", "stderr", "success", "error"]);
const PAGE_SIZE = 300;

function validPage(value: InstallLogPage, after?: number): value is InstallLogPage {
  if (!value || !Array.isArray(value.rows) || value.rows.length > PAGE_SIZE
    || typeof value.more !== "boolean" || !Number.isSafeInteger(value.olderRows)
    || value.olderRows < 0 || !Number.isSafeInteger(value.earliestId) || value.earliestId < 0) return false;
  if (after === undefined && value.more) return false;
  if (value.more && value.rows.length !== PAGE_SIZE) return false;
  let previous = after ?? 0;
  for (const row of value.rows) {
    if (!row || !Number.isSafeInteger(row.id) || row.id <= previous
      || typeof row.stage !== "string" || Array.from(row.stage).length > 120
      || typeof row.stream !== "string" || !streams.has(row.stream)
      || typeof row.message !== "string" || Array.from(row.message).length > 1200
      || typeof row.time !== "string") return false;
    previous = row.id;
  }
  return true;
}

// Events wake the reader; only committed, redacted journal rows enter the view.
export function useInstallLogPanel(changed: () => void = () => {}) {
  const logs = ref<InstallLogRow[]>([]), evicted = ref(0);
  const connectionUnavailable = ref(false), gapPossible = ref(false);
  let mounted = false, subscribing = false, reading = false, rerun = false;
  let initialized = false, cursor = 0, generation = 0, readFailed = false;
  let release: UnlistenFn | undefined, retry: number | undefined;

  function updateConnection() { connectionUnavailable.value = !release || readFailed; }

  async function refresh() {
    if (!mounted) return;
    if (reading) { rerun = true; return; }
    reading = true;
    const current = generation;
    try {
      do {
        rerun = false;
        const after = initialized ? cursor : undefined;
        const page = await environmentApi.listEnvironmentInstallLogs(after, PAGE_SIZE);
        if (!mounted || current !== generation) break;
        if (!validPage(page, after)) throw Error("Invalid install log page");
        if (after !== undefined && page.earliestId > after + 1) gapPossible.value = true;
        if (!initialized) { initialized = true; evicted.value = page.olderRows; }
        if (page.rows.length) {
          logs.value.push(...page.rows);
          cursor = page.rows[page.rows.length - 1].id;
          if (logs.value.length > PAGE_SIZE)
            evicted.value += logs.value.splice(0, logs.value.length - PAGE_SIZE).length;
          changed();
        }
        readFailed = false;
        updateConnection();
        if (page.more) rerun = true;
      } while (rerun && mounted && current === generation);
    } catch {
      if (mounted && current === generation) { readFailed = true; updateConnection(); }
    } finally {
      reading = false;
      if (rerun && mounted) { rerun = false; void refresh(); }
    }
  }

  // A durable high-water mark prevents prior rows and in-flight reads from
  // reappearing in the next installation's console.
  async function clear() {
    ++generation;
    const current = generation;
    try {
      const page = await environmentApi.listEnvironmentInstallLogs(undefined, PAGE_SIZE);
      if (!validPage(page)) throw Error("Invalid install log snapshot");
      if (!mounted || current !== generation) throw Error("Install log view unavailable");
      cursor = Math.max(cursor, page.rows[page.rows.length - 1]?.id ?? 0);
      initialized = true;
      logs.value = [];
      evicted.value = 0;
      gapPossible.value = false;
      readFailed = false;
      updateConnection();
      if (reading) rerun = true;
    } catch {
      if (mounted && current === generation) { readFailed = true; updateConnection(); }
      throw Error("无法确认安装日志起点，请稍后重试");
    }
  }

  async function subscribe() {
    if (!mounted || release || subscribing) return;
    subscribing = true;
    try {
      const unlisten = await listen("environment-install-log", () => { void refresh(); });
      if (!mounted) unlisten();
      else { release = unlisten; updateConnection(); void refresh(); }
    } catch {
      if (mounted) updateConnection();
    } finally { subscribing = false; }
  }
  onMounted(() => {
    mounted = true;
    updateConnection();
    void subscribe();
    void refresh();
    retry = window.setInterval(() => { void subscribe(); void refresh(); }, 15000);
  });
  onBeforeUnmount(() => {
    mounted = false;
    ++generation;
    if (retry !== undefined) window.clearInterval(retry);
    release?.();
    release = undefined;
  });
  return { logs, evicted, connectionUnavailable, gapPossible, clear, refresh };
}
