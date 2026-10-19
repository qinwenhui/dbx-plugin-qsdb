<script setup lang="ts">
// 行详情查看器：宽表的逐字段视图。Esc 关闭，←/→ 上一行/下一行，
// 每个字段可单独复制；row 由父层按全局行号响应式提供（分页缓存补页后自动填充）。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { state, type CellValue } from "../store";
import type { DbxColumn } from "../bridge";
import { copyText } from "../clipboard";

const props = defineProps<{
  index: number;
  row: CellValue[] | null;
  columns: DbxColumn[];
  total: number;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "nav", delta: number): void;
}>();

const t = computed(() => state.t);

function textOf(v: CellValue | undefined): string {
  if (v === null || v === undefined) return "";
  return typeof v === "object" ? JSON.stringify(v, null, 2) : String(v);
}
function isNull(v: CellValue | undefined): boolean {
  return v === null || v === undefined;
}

const copiedIdx = ref<number | null>(null);
async function copyField(i: number, v: CellValue | undefined) {
  if (isNull(v)) return;
  if (!(await copyText(textOf(v)))) return;
  copiedIdx.value = i;
  setTimeout(() => (copiedIdx.value = null), 700);
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
  else if (e.key === "ArrowLeft" && props.index > 0) emit("nav", -1);
  else if (e.key === "ArrowRight" && props.index < props.total - 1) emit("nav", 1);
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <div class="modal-card">
      <div class="modal-head">
        <span class="strong">{{ t.rowDetail }}</span>
        <span class="muted small">{{ t.rowCol }} {{ index + 1 }} / {{ total }}</span>
        <span class="spacer" style="flex: 1" />
        <button class="dbx-btn dbx-btn--ghost" style="height: 22px" :disabled="index === 0" :title="t.prevRow" @click="emit('nav', -1)">‹</button>
        <button class="dbx-btn dbx-btn--ghost" style="height: 22px" :disabled="index >= total - 1" :title="t.nextRow" @click="emit('nav', 1)">›</button>
        <button class="dbx-btn dbx-btn--ghost" style="height: 22px" :title="t.close" @click="emit('close')">✕</button>
      </div>
      <div class="modal-body">
        <table class="detail-table">
          <thead>
            <tr>
              <th class="dname">{{ t.field }}</th>
              <th class="dtype">{{ t.type }}</th>
              <th>{{ t.value }}</th>
              <th class="dact"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="!row">
              <td colspan="4" class="muted small">{{ t.loading }}</td>
            </tr>
            <template v-else>
              <tr v-for="(c, i) in columns" :key="i">
                <td class="dname">{{ c.name }}</td>
                <td class="dtype mono">{{ c.typeName }}</td>
                <td class="dval" :class="{ null: isNull(row[i]) }">
                  <pre>{{ isNull(row[i]) ? t.nullCell : textOf(row[i]) }}</pre>
                </td>
                <td class="dact">
                  <button class="dbx-btn dbx-btn--ghost" style="height: 20px; font-size: 11px" :disabled="isNull(row[i])" @click="copyField(i, row[i])">
                    {{ copiedIdx === i ? t.copied : t.copyValue }}
                  </button>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>
