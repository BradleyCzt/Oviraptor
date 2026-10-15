<script setup lang="ts">
import { reactive, ref } from "vue";
import { AppWindow, ImageUp, RotateCcw } from "@lucide/vue";
import ModalShell from "../../shared/components/ModalShell.vue";
import { shellApi } from "../api";
import { useI18n } from "../../i18n";
import type { AppSettings } from "../types";
import ToolApprovalPanel from "../../features/environment/ToolApprovalPanel.vue";

const props = defineProps<{ settings: AppSettings }>();
const emit = defineEmits<{ close: []; saved: []; iconChanged: [] }>();
const { tr } = useI18n();
const form = reactive({ reminderDays: props.settings.reminderDays || 7 });
const busy = ref(false);
const iconBusy = ref(false);
const error = ref("");
const iconState = ref(props.settings.customIcon);

async function save() {
  busy.value = true; error.value = "";
  try { await shellApi.saveAppSettings(form.reminderDays); emit("saved"); }
  catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}

async function uploadIcon(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  iconBusy.value = true; error.value = "";
  try {
    if (file.type !== "image/png") throw new Error(tr("请选择 PNG 图片", "Select a PNG image"));
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    await shellApi.saveAppIcon(bytes); iconState.value = true; emit("iconChanged");
  } catch (reason) { error.value = String(reason); }
  finally { iconBusy.value = false; input.value = ""; }
}

async function resetIcon() {
  iconBusy.value = true; error.value = "";
  try { await shellApi.resetAppIcon(); iconState.value = false; emit("iconChanged"); }
  catch (reason) { error.value = String(reason); }
  finally { iconBusy.value = false; }
}
</script>

<template>
  <ModalShell :title="tr('应用设置','App settings')" @close="$emit('close')">
    <template #eyebrow><span class="eyebrow"><AppWindow :size="14" /> APPLICATION</span></template>
    <div class="form-stack">
      <label class="field">
        <span>{{tr('项目更新提醒（天）','Project update reminder (days)')}}</span>
        <input v-model.number="form.reminderDays" type="number" min="1" max="365" />
      </label>
      <p class="helper">{{tr('打开应用时，如果项目超过这个天数没有完成更新，会显示提醒。定时任务需要电脑和应用保持运行。','When the app opens, projects older than this are shown as due. Scheduled work requires the computer and app to stay running.')}}</p>
      <div class="icon-setting-card">
        <div class="icon-setting-preview"><AppWindow :size="24" /></div>
        <div><strong>{{tr('界面品牌图标','Interface brand icon')}}</strong><span>{{iconState?tr('正在使用自定义 PNG（不影响桌面和菜单栏）','Using a custom PNG (does not affect the desktop or menu bar)'):tr('正在使用默认品牌图标','Using the default brand icon')}}</span></div>
        <label class="button secondary compact file-button"><ImageUp :size="15" /> {{iconBusy?tr('处理中…','Working…'):tr('上传 PNG','Upload PNG')}}<input type="file" accept="image/png,.png" :disabled="iconBusy" @change="uploadIcon" /></label>
        <button class="button ghost compact" :disabled="iconBusy||!iconState" @click="resetIcon"><RotateCcw :size="14" /> {{tr('恢复默认','Reset')}}</button>
      </div>
      <p class="helper">{{tr('建议使用透明背景的正方形 PNG，尺寸至少 32×32，最大 5MB。','Use a square transparent PNG, at least 32×32 and no larger than 5MB.')}}</p>
      <p v-if="error" class="form-error">{{error}}</p>
      <ToolApprovalPanel />
    </div>
    <template #footer>
      <button class="button ghost" @click="$emit('close')">{{tr('取消','Cancel')}}</button>
      <button class="button primary" :disabled="busy" @click="save">{{busy?tr('保存中…','Saving…'):tr('保存设置','Save settings')}}</button>
    </template>
  </ModalShell>
</template>
