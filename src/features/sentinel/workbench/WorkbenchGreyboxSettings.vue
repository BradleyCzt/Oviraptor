<script setup lang="ts">
import { KeyRound } from "@lucide/vue";
import type { WorkbenchScanInput } from "../types";

defineProps<{
  form: Pick<WorkbenchScanInput, "environment" | "authProfileName" | "authType" | "authHeaderName" | "authValue">;
}>();
</script>

<template>
  <section class="workbench-mode-settings span-two">
    <header>
      <KeyRound :size="15" />
      <div>
        <strong>灰盒环境与手动认证备用</strong>
        <small>优先使用浏览器会话；这里只为无法打开浏览器登录的 API Token 场景保留。</small>
      </div>
    </header>
    <div class="workbench-fields nested-fields">
      <label class="field"><span>环境</span><input v-model="form.environment" placeholder="staging" /></label>
      <label class="field"><span>认证配置名称</span><input v-model="form.authProfileName" placeholder="测试管理员会话" /></label>
      <label class="field"><span>认证方式</span>
        <select v-model="form.authType">
          <option value="none">匿名</option>
          <option value="cookie">Cookie</option>
          <option value="bearer">Bearer Token</option>
          <option value="header">自定义 Header</option>
        </select>
      </label>
      <label v-if="form.authType === 'header'" class="field"><span>Header 名称</span><input v-model="form.authHeaderName" placeholder="X-API-Key" /></label>
      <label v-if="form.authType !== 'none'" class="field span-two"><span>临时会话值</span>
        <input v-model="form.authValue" type="password" autocomplete="off"
          :placeholder="form.authType === 'cookie' ? 'session=...' : form.authType === 'bearer' ? 'eyJ...' : 'Header value'" />
      </label>
    </div>
  </section>
</template>
