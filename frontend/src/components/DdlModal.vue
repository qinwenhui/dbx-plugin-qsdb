<script setup lang="ts">
// 表 DDL 视图：由服务端 schema 生成参考 CREATE TABLE（分区列以注释标注，
// 不猜测服务端分区子句语法）。可复制全文。Esc 关闭。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { state } from "../store";
import type { TableInfo } from "../store";
import { generateDdl } from "../sqlkit";
import { copyText } from "../clipboard";

const props = defineProps<{ table: TableInfo }>();
const emit = defineEmits<{ (e: "close"): void }>();

const t = computed(() => state.t);
const ddl = computed(() => generateDdl(props.table.name, props.table.columns, t.value.ddlPartitionNote));

const copied = ref(false);
async function copyDdl() {
  if (!(await copyText(ddl.value))) return;
  copied.value = true;
  setTimeout(() => (copied.value = false), 900);
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <div class="modal-card ddl-card">
      <div class="modal-head">
        <span class="strong">DDL · {{ table.name }}</span>
        <span class="badge" style="height: 16px">{{ t.colsCount(table.columns.length) }}</span>
        <span class="muted small ddl-note">{{ t.ddlNote }}</span>
        <span class="spacer" style="flex: 1" />
        <button class="dbx-btn dbx-btn--ghost" style="height: 22px" @click="copyDdl">{{ copied ? t.copied : t.copyDdl }}</button>
        <button class="dbx-btn dbx-btn--ghost" style="height: 22px" :title="t.close" @click="emit('close')">✕</button>
      </div>
      <div class="modal-body">
        <pre class="ddl-pre">{{ ddl }}</pre>
      </div>
    </div>
  </div>
</template>
