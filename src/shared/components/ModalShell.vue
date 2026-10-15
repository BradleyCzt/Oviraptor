<script setup lang="ts">
import { X } from "@lucide/vue";

defineProps<{ title: string; wide?: boolean }>();
defineEmits<{ close: [] }>();
</script>

<template>
  <div class="modal-backdrop" @mousedown.self="$emit('close')">
    <section class="modal-card" :class="{ wide }">
      <header class="modal-header">
        <div>
          <slot name="eyebrow" />
          <h2>{{ title }}</h2>
        </div>
        <button class="icon-button" @click="$emit('close')"><X :size="18" /></button>
      </header>
      <div class="modal-body"><slot /></div>
      <footer v-if="$slots.footer" class="modal-footer"><slot name="footer" /></footer>
    </section>
  </div>
</template>
