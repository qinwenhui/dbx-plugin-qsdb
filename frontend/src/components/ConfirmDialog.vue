<script setup lang="ts">
// 确认对话框：沙箱 iframe 里原生 confirm/alert/prompt 不可依赖（官方文档
// 明令禁止）——破坏性操作（取消查询/踢会话等）一律走这里。Esc/取消关闭。
import { computed, onBeforeUnmount, onMounted } from "vue";
import { state } from "../store";

const props = defineProps<{
  message: string;
  confirmLabel?: string;
  danger?: boolean;
}>();
const emit = defineEmits<{ (e: "ok"): void; (e: "cancel"): void }>();

const t = computed(() => state.t);

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("cancel");
  else if (e.key === "Enter") emit("ok");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="modal-overlay confirm-overlay" @click.self="emit('cancel')">
    <div class="modal-card confirm-card">
      <div class="modal-body confirm-body">
        <div class="confirm-icon" :class="{ danger: props.danger }">{{ props.danger ? "⚠" : "?" }}</div>
        <div>{{ message }}</div>
      </div>
      <div class="confirm-actions">
        <button class="dbx-btn" @click="emit('cancel')">{{ t.cancel }}</button>
        <button class="dbx-btn" :class="props.danger ? 'dbx-btn--danger' : 'dbx-btn--primary'" @click="emit('ok')">
          {{ props.confirmLabel ?? t.confirm }}
        </button>
      </div>
    </div>
  </div>
</template>
