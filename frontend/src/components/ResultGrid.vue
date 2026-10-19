<script setup lang="ts">
// 结果网格：虚拟滚动 + 分页缓存（store.rowsFor 按需取页）。
// 只读展示：NULL 斜体、列头带类型；exec 回执显示影响行数。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { state, rowsFor, type CellValue, type QueryStateT } from "../store";

const emit = defineEmits<{ (e: "visible-rows", rows: CellValue[][]): void }>();

const wrap = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportH = ref(400);
const ROW_H = 26;
const OVERSCAN = 8;

const q = computed(() => state.query as QueryStateT | null);
const columns = computed(() => q.value?.columns ?? []);

/** 列宽：按列名/抽样内容估宽（80–360px）。 */
const colWidths = computed(() => {
  const cols = columns.value;
  if (!q.value) return cols.map(() => 100);
  return cols.map((c, ci) => {
    let w = c.name.length * 8 + 40;
    let sampled = 0;
    for (const rows of q.value!.pages.values()) {
      for (const row of rows) {
        const v = row[ci];
        if (v !== null && v !== undefined) w = Math.max(w, String(v).length * 7.2 + 20);
        if (++sampled > 400) break;
      }
      if (sampled > 400) break;
    }
    return Math.min(360, Math.max(80, w));
  });
});

const totalWidth = computed(() => colWidths.value.reduce((a, b) => a + b, 0));

const total = computed(() => q.value?.totalRows ?? 0);
const gridH = computed(() => Math.max(1, total.value * ROW_H));

const startIdx = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_H) - OVERSCAN));
const endIdx = computed(() => Math.min(total.value, Math.ceil((scrollTop.value + viewportH.value) / ROW_H) + OVERSCAN));

const visibleRows = computed(() => {
  const query = q.value;
  if (!query || query.receipt) return [];
  return rowsFor(query, startIdx.value, endIdx.value);
});

// 通知父层当前可见行（复制用）
let notifyTimer: ReturnType<typeof setTimeout> | undefined;
function notify() {
  clearTimeout(notifyTimer);
  notifyTimer = setTimeout(() => emit("visible-rows", visibleRows.value.slice()), 120);
}

function onScroll() {
  const el = wrap.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  viewportH.value = el.clientHeight;
  notify();
}

onMounted(() => {
  const el = wrap.value;
  if (el) {
    viewportH.value = el.clientHeight;
    const ro = new ResizeObserver(() => {
      if (wrap.value) viewportH.value = wrap.value.clientHeight;
    });
    ro.observe(el);
    onBeforeUnmount(() => ro.disconnect());
  }
  notify();
});

function fmtCell(v: CellValue): { text: string; isNull: boolean } {
  if (v === null || v === undefined) return { text: "", isNull: true };
  return { text: typeof v === "object" ? JSON.stringify(v) : String(v), isNull: false };
}

function emptyText(): string {
  if (!q.value) return "";
  if (q.value.receipt) return "";
  if (q.value.running) return "";
  return columns.value.length ? "0 rows" : "";
}
</script>

<template>
  <div>
    <!-- exec 回执 -->
    <div v-if="q?.receipt" class="center-note">
      <div>
        <div style="font-size: 28px">✅</div>
        <div style="margin-top: 8px">
          <strong>{{ q.receipt.command }}</strong>
          <template v-if="q.receipt.message"> — {{ q.receipt.message }}</template>
          <div>{{ state.t.rowsAffected(String(q.receipt.rowsAffected)) }}</div>
        </div>
      </div>
    </div>

    <!-- 空态 -->
    <div v-else-if="!q || (total === 0 && columns.length === 0)" class="center-note">
      <div>{{ emptyText() || state.t.rowsEmpty }}</div>
    </div>

    <!-- 虚拟网格 -->
    <div v-else ref="wrap" class="grid-wrap" @scroll.passive="onScroll">
      <div class="grid-inner" :style="{ height: `${gridH + ROW_H}px`, width: `${totalWidth}px` }">
        <div class="grid-header" :style="{ width: `${totalWidth}px` }">
          <div v-for="(c, i) in columns" :key="i" class="cell" :style="{ width: `${colWidths[i]}px` }" :title="`${c.name} · ${c.typeName}`">
            {{ c.name }}<span style="opacity: 0.6"> · {{ c.typeName }}</span>
          </div>
        </div>
        <div :style="{ position: 'absolute', top: `${startIdx * ROW_H + ROW_H}px`, width: `${totalWidth}px` }">
          <div v-for="(row, ri) in visibleRows" :key="startIdx + ri" class="grid-row" :style="{ height: `${ROW_H}px` }">
            <div
              v-for="(v, ci) in row"
              :key="ci"
              class="cell"
              :class="{ null: fmtCell(v).isNull }"
              :style="{ width: `${colWidths[ci]}px` }"
              :title="fmtCell(v).text"
            >
              {{ fmtCell(v).isNull ? state.t.nullCell : fmtCell(v).text }}
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
