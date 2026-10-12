<script setup>
import { ref } from 'vue';
import ToolApprovalPanel from '../../src/features/environment/ToolApprovalPanel.vue';
import { fixture, release, reset } from './transport.mjs';
const mountVersion = ref(0);
function restart() { reset(); mountVersion.value++; }
</script>
<template>
  <main>
    <h1>工具批准面板：实际浏览器 DOM / 模拟 IPC</h1>
    <p role="note">仅内存夹具，不连接 Tauri、用户应用、数据库或密钥。文件内容不做密码签名验证；成功状态只检验界面交互。</p>
    <section class="controls">
      <label>模拟回复 <select v-model="fixture.mode" aria-label="模拟回复"><option value="normal">正常不可执行库存</option><option value="fence">准备门禁仍在</option><option value="pending">等待手动释放</option><option value="saved-error">批准已保存、准备未完成错误</option><option value="revoke-error">撤销已保存、准备未完成错误</option><option value="unconfigured">未配置管理员信任</option></select></label>
      <button type="button" @click="release" :disabled="fixture.pending === 0">释放一个在途批准</button>
      <button type="button" @click="restart">重置夹具并重新挂载</button>
      <p>模拟 IPC 调用数：<output id="call-count">{{ fixture.calls.length }}</output> · 在途：<output id="pending-count">{{ fixture.pending }}</output></p>
      <pre id="mock-calls">{{ JSON.stringify(fixture.calls, null, 2) }}</pre>
    </section>
    <ToolApprovalPanel :key="mountVersion" />
  </main>
</template>
<style>
body { margin: 0; color: #192331; background: #f6f7fa; font: 14px system-ui, sans-serif; }
main { max-width: 960px; margin: 24px auto; padding: 24px; background: white; border-radius: 12px; }
h1 { font-size: 20px; } .form-stack { display: grid; gap: 12px; }.field { display: grid; gap: 6px; }
input, select, button { padding: 8px; border: 1px solid #bdc6d3; border-radius: 6px; font: inherit; }
button { cursor: pointer; } button:disabled { opacity: .5; cursor: default; }.helper { color: #536072; line-height: 1.5; }.form-error { color: #a3152d; }
.controls { background: #eef3fa; padding: 12px; border-radius: 8px; margin-bottom: 20px; }.controls button { margin: 6px; }pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 12px; }
</style>
