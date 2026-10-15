import { onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sentinelApi } from "../api";
import type { BrowserAuthSession } from "../types";

type AuthForm = {
  projectId: number;
  urls: string;
  authSessionId: string;
  authSessionIds: string[];
  authLoginUrl: string;
  authSessionName: string;
};
type Notice = (type: "success" | "error" | "info", text: string) => void;

export const newAuthSessionScope = () => globalThis.crypto.randomUUID();

export function useBrowserAuthSessions(options: {
  form: AuthForm;
  tr: (zh: string, en: string) => string;
  notify: Notice;
  isDisposed: () => boolean;
}) {
  const { form, tr, notify, isDisposed } = options;
  const authBusy = ref("");
  const authSessions = ref<BrowserAuthSession[]>([]);
  const authSessionScopeId = ref(newAuthSessionScope());
  const showAuthSessionPicker = ref(false);
  const authCaptureTimers = new Map<string, number>();
  let unlistenAuthSession: UnlistenFn | undefined;
  let disposed = false;

  function stopAuthCapturePolling(sessionId: string) {
    const timer = authCaptureTimers.get(sessionId);
    if (timer !== undefined) window.clearInterval(timer);
    authCaptureTimers.delete(sessionId);
  }
  function resetIdentitySelection() {
    for (const sessionId of authCaptureTimers.keys()) stopAuthCapturePolling(sessionId);
    authSessionScopeId.value = newAuthSessionScope();
    form.authSessionId = "";
    form.authSessionIds = [];
    authSessions.value = [];
    showAuthSessionPicker.value = false;
  }
  function startAuthCapturePolling(session: BrowserAuthSession) {
    stopAuthCapturePolling(session.id);
    const timer = window.setInterval(async () => {
      if (disposed || isDisposed() || (authBusy.value && authBusy.value !== session.id)) return;
      authBusy.value = session.id;
      try {
        const updated = await sentinelApi.finishBrowserAuthSession(session.id);
        await loadAuthSessions();
        if (updated.status === "valid") stopAuthCapturePolling(session.id);
      } catch {
        stopAuthCapturePolling(session.id);
      } finally {
        if (authBusy.value === session.id) authBusy.value = "";
      }
    }, 2500);
    authCaptureTimers.set(session.id, timer);
  }
  async function loadAuthSessions() {
    if (!form.projectId) {
      authSessions.value = [];
      return;
    }
    const projectId = form.projectId;
    const scopeId = authSessionScopeId.value;
    try {
      const sessions = await sentinelApi.listBrowserAuthSessions(projectId, scopeId);
      if (disposed || isDisposed() || projectId !== form.projectId || scopeId !== authSessionScopeId.value) return;
      authSessions.value = sessions;
      const validIds = new Set(sessions.filter((session) => session.status === "valid").map((session) => session.id));
      form.authSessionIds = form.authSessionIds.filter((id) => validIds.has(id));
      form.authSessionId = form.authSessionIds[0] || "";
    } catch (error) {
      if (disposed || isDisposed() || projectId !== form.projectId || scopeId !== authSessionScopeId.value) return;
      notify("error", String(error));
    }
  }
  function onAuthSessionToggle(sessionId: string, event: Event) {
    const selected = (event.target as HTMLInputElement | null)?.checked === true;
    if (selected) {
      if (form.authSessionIds.length >= 5) {
        notify("info", tr("单个任务最多比较 5 个独立登录身份", "A task can compare up to five isolated identities"));
        return;
      }
      if (!form.authSessionIds.includes(sessionId)) form.authSessionIds.push(sessionId);
    } else {
      form.authSessionIds = form.authSessionIds.filter((id) => id !== sessionId);
    }
    form.authSessionId = form.authSessionIds[0] || "";
  }
  function firstUrl() {
    return form.urls.split(/\r?\n|,/).map((value) => value.trim()).find(Boolean) || "";
  }
  function authStatusLabel(status: string) {
    return ({
      valid: tr("会话有效", "Session active"), capturing: tr("等待完成登录", "Waiting for login"),
      needs_check: tr("需要确认", "Needs check"), invalid: tr("会话失效", "Session invalid"),
      expired: tr("会话过期", "Session expired"),
    } as Record<string, string>)[status] || status;
  }
  function shortTime(value: string) {
    if (!value) return tr("尚未校验", "Not validated");
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
  }
  function currentScope() {
    const projectId = form.projectId;
    const scopeId = authSessionScopeId.value;
    return () => !disposed && !isDisposed() && form.projectId === projectId && authSessionScopeId.value === scopeId;
  }
  async function openLogin(session?: BrowserAuthSession) {
    if (!form.projectId) {
      notify("info", tr("请先选择工作空间", "Select a workspace first"));
      return;
    }
    const entryUrl = (form.authLoginUrl || session?.entryUrl || firstUrl() || "").trim();
    if (!/^https?:\/\//i.test(entryUrl)) {
      notify("info", tr("请先填写登录 URL 或授权 URL", "Enter a login or authorized URL first"));
      return;
    }
    const current = currentScope();
    const busyId = session?.id || "new";
    authBusy.value = busyId;
    try {
      const opened = await sentinelApi.openBrowserAuthSession({
        id: session?.id, projectId: form.projectId, name: form.authSessionName || session?.name || "",
        entryUrl, draftScopeId: authSessionScopeId.value,
      });
      if (!current()) return;
      startAuthCapturePolling(opened);
      await loadAuthSessions();
      if (current()) notify("info", tr("登录窗口已打开：登录成功并访问后台功能后，会自动保存身份、显示右上角成功提示并关闭窗口", "Login window opened. After a successful login and an authenticated feature request, the identity is saved automatically and the window closes."));
    } catch (error) {
      if (current()) notify("error", String(error));
    } finally {
      if (authBusy.value === busyId) authBusy.value = "";
    }
  }
  async function finishLogin(session: BrowserAuthSession) {
    const current = currentScope();
    authBusy.value = session.id;
    try {
      const updated = await sentinelApi.finishBrowserAuthSession(session.id);
      if (!current()) return;
      if (!form.authSessionIds.includes(updated.id)) form.authSessionIds.push(updated.id);
      form.authSessionId = form.authSessionIds[0] || updated.id;
      form.authSessionName = "";
      await loadAuthSessions();
      if (current() && updated.status !== "valid") notify("info", updated.lastError || tr("已捕获，但需要再访问一个登录后功能", "Captured, but visit an authenticated feature first"));
    } catch (error) {
      if (current()) notify("error", String(error));
    } finally {
      if (authBusy.value === session.id) authBusy.value = "";
    }
  }
  async function validateLogin(session: BrowserAuthSession) {
    const current = currentScope();
    authBusy.value = session.id;
    try {
      const updated = await sentinelApi.validateBrowserAuthSession(session.id);
      if (!current()) return;
      await loadAuthSessions();
      if (current()) notify(updated.status === "valid" ? "success" : "info", updated.status === "valid"
        ? tr("会话校验通过", "Session validation passed")
        : updated.lastError || tr("会话需要重新确认", "Session needs confirmation"));
    } catch (error) {
      if (current()) notify("error", String(error));
    } finally {
      if (authBusy.value === session.id) authBusy.value = "";
    }
  }
  async function removeLogin(session: BrowserAuthSession) {
    if (!window.confirm(tr(`删除登录会话“${session.name}”？之后需要重新登录。`, `Delete session “${session.name}”? You will need to sign in again.`))) return;
    const current = currentScope();
    authBusy.value = session.id;
    try {
      await sentinelApi.deleteBrowserAuthSession(session.id);
      if (!current()) return;
      form.authSessionIds = form.authSessionIds.filter((id) => id !== session.id);
      form.authSessionId = form.authSessionIds[0] || "";
      await loadAuthSessions();
      if (current()) notify("success", tr("登录会话已删除", "Login session deleted"));
    } catch (error) {
      if (current()) notify("error", String(error));
    } finally {
      if (authBusy.value === session.id) authBusy.value = "";
    }
  }

  onMounted(async () => {
    const unlisten = await listen<BrowserAuthSession>("browser-auth-session-updated", async (event) => {
      const session = event.payload;
      if (!session || disposed || isDisposed() || session.projectId !== form.projectId || session.draftScopeId !== authSessionScopeId.value) return;
      await loadAuthSessions();
      if (disposed || isDisposed() || session.projectId !== form.projectId || session.draftScopeId !== authSessionScopeId.value) return;
      if (session.status === "valid") {
        stopAuthCapturePolling(session.id);
        if (!form.authSessionIds.includes(session.id)) form.authSessionIds.push(session.id);
        form.authSessionId = session.id;
        form.authSessionName = "";
        showAuthSessionPicker.value = false;
        notify("success", tr(`登录成功，${session.name} 已保存身份`, `Login succeeded; ${session.name} identity saved`));
      }
    });
    if (disposed || isDisposed()) unlisten();
    else unlistenAuthSession = unlisten;
    await loadAuthSessions();
  });
  onBeforeUnmount(() => {
    disposed = true;
    for (const sessionId of authCaptureTimers.keys()) stopAuthCapturePolling(sessionId);
    unlistenAuthSession?.();
    unlistenAuthSession = undefined;
  });
  return { authBusy, authSessions, authSessionScopeId, showAuthSessionPicker,
    resetIdentitySelection, loadAuthSessions, onAuthSessionToggle, firstUrl, authStatusLabel,
    shortTime, openLogin, finishLogin, validateLogin, removeLogin };
}
