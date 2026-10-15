<script setup lang="ts">
import { computed } from "vue";
import { AlertTriangle, Check, LogIn, RefreshCw, ShieldCheck, X } from "@lucide/vue";
import type { BrowserAuthSession, SentinelScan } from "../types";

const props = defineProps<{
  scan: SentinelScan;
  sessions: BrowserAuthSession[];
  busy: string;
}>();
const emit = defineEmits<{
  reopen: [session: BrowserAuthSession];
  finish: [session: BrowserAuthSession];
  validate: [session: BrowserAuthSession];
  continue: [];
  close: [];
}>();

const ready = computed(
  () => props.sessions.length > 0 && props.sessions.every((session) => session.status === "valid"),
);
const statusLabel = (status: string) =>
  ({
    valid: "绿色有效",
    capturing: "登录窗口已打开",
    needs_check: "需要校验",
    invalid: "会话已失效",
    expired: "会话已过期",
  } as Record<string, string>)[status] || status;
</script>

<template>
  <section class="scan-auth-recovery">
    <header>
      <div class="scan-auth-recovery-title">
        <span class="scan-auth-warning"><AlertTriangle :size="18" /></span>
        <div>
          <span class="eyebrow">SESSION RECOVERY</span>
          <h3>原任务需要重新登录</h3>
          <p>任务「{{ scan.taskName || scan.projectName }}」仍保留全部历史、证据和累计 Token。只更新它绑定的会话，然后在同一个任务上继续执行。</p>
        </div>
      </div>
      <button class="icon-button" title="暂时关闭" @click="emit('close')"><X :size="15" /></button>
    </header>

    <div class="scan-auth-session-list">
      <article v-for="session in sessions" :key="session.id" :class="`status-${session.status}`">
        <div class="scan-auth-session-main">
          <span class="auth-status-dot"></span>
          <div>
            <strong>{{ session.name }}</strong>
            <small>{{ statusLabel(session.status) }} · {{ session.entryUrl }}</small>
          </div>
        </div>
        <div class="auth-session-metrics">
          <span><b>{{ session.cookieCount }}</b> Cookie</span>
          <span><b>{{ session.headerCount }}</b> 认证头</span>
          <span><b>{{ session.storageCount }}</b> Storage</span>
          <span><b>{{ session.capturedRequestCount }}</b> 请求</span>
        </div>
        <p v-if="session.lastError" class="auth-session-error">{{ session.lastError }}</p>
        <div class="auth-session-actions">
          <button v-if="session.status === 'capturing'" class="button primary compact" :disabled="Boolean(busy)" @click="emit('finish', session)"><Check :size="13" />我已登录，保存会话</button>
          <button :class="session.status === 'capturing' ? 'button ghost compact' : 'button primary compact'" :disabled="Boolean(busy)" @click="emit('reopen', session)"><LogIn :size="13" />{{ session.status === 'capturing' ? '打开登录窗口' : '重新登录' }}</button>
          <button v-if="session.status !== 'capturing'" class="button ghost compact" :disabled="Boolean(busy)" @click="emit('validate', session)"><ShieldCheck :size="13" />校验当前会话</button>
        </div>
      </article>
    </div>

    <footer>
      <span><ShieldCheck :size="14" />{{ ready ? "所有绑定身份均为绿色，可以继续原任务。" : "完成动态验证码或 SSO 后，回到这里保存会话；不创建新任务。" }}</span>
      <button class="button secondary" :disabled="!ready || Boolean(busy)" @click="emit('continue')"><RefreshCw :size="14" />会话有效，继续当前任务</button>
    </footer>
  </section>
</template>
