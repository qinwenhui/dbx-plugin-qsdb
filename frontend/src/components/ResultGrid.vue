<script setup lang="ts">
// 结果网格：虚拟滚动 + 分页缓存（store.rowsFor 按需取页）。
// 交互：行号槽点开行详情、点单元格复制、表头点击排序（前端快照）、
// 表头右缘拖拽调列宽（新查询重置）。排序激活时数据来自 sortView 快照。
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { state, rowsFor, toggleSort, type CellValue, type QueryStateT } from "../store";
import { copyText } from "../clipboard";
import RowDetail from "./RowDetail.vue";

const emit = defineEmits<{
  (e: "visible-rows", rows: CellValue[][]): void;
  (e: "range", range: { start: number; end: number }): void;
}>();

const wrap = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportH = ref(400);
const HEADER_H = 42;
const GUTTER_W = 64;
const OVERSCAN = 8;

/** 行高跟随密度设置（紧凑 22 / 舒适 26）。 */
const ROW_H = computed(() => (state.density === "compact" ? 22 : 26));

const q = computed(() => state.query as QueryStateT | null);
const columns = computed(() => q.value?.columns ?? []);

// ---- 列宽：按列名/抽样内容估宽（90–360px），拖拽覆盖，新查询重置 ----
const overrides = reactive(new Map<number, number>());
watch(
  () => q.value?.queryId,
  () => {
    overrides.clear();
    detailIndex.value = null;
  },
);

function baseWidth(ci: number): number {
  const c = columns.value[ci];
  if (!c) return 100;
  let w = Math.max(c.name.length, c.typeName.length) * 8 + 34;
  let sampled = 0;
  const query = q.value;
  if (query) {
    for (const rows of query.pages.values()) {
      for (const row of rows) {
        const v = row[ci];
        if (v !== null && v !== undefined) w = Math.max(w, String(v).length * 7.2 + 20);
        if (++sampled > 400) break;
      }
      if (sampled > 400) break;
    }
  }
  return Math.min(360, Math.max(90, w));
}

function colW(ci: number): number {
  return overrides.get(ci) ?? baseWidth(ci);
}
const totalWidth = computed(() => GUTTER_W + columns.value.reduce((a, _, i) => a + colW(i), 0));

// ---- 列宽拖拽 ----
let drag: { col: number; startX: number; startW: number } | null = null;
function onResizeDown(e: MouseEvent, col: number) {
  drag = { col, startX: e.clientX, startW: colW(col) };
  window.addEventListener("mousemove", onResizeMove);
  window.addEventListener("mouseup", onResizeUp);
  document.body.classList.add("col-resizing");
}
function onResizeMove(e: MouseEvent) {
  if (!drag) return;
  overrides.set(drag.col, Math.max(60, drag.startW + e.clientX - drag.startX));
}
function onResizeUp() {
  drag = null;
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", onResizeUp);
  document.body.classList.remove("col-resizing");
}
onBeforeUnmount(() => {
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", onResizeUp);
  document.body.classList.remove("col-resizing");
});

// ---- 排序（表头点击；状态与快照在 store） ----
function onHeaderClick(col: number) {
  void toggleSort(col);
}

const total = computed(() => {
  if (q.value?.sortView) return q.value.sortView.length;
  return q.value?.totalRows ?? 0;
});
const gridH = computed(() => Math.max(1, total.value * ROW_H.value));

const startIdx = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_H.value) - OVERSCAN));
const endIdx = computed(() => Math.min(total.value, Math.ceil((scrollTop.value + viewportH.value) / ROW_H.value) + OVERSCAN));

const visibleRows = computed(() => {
  const query = q.value;
  if (!query || query.receipt) return [];
  if (query.sortView) return query.sortView.slice(startIdx.value, endIdx.value);
  return rowsFor(query, startIdx.value, endIdx.value);
});

// 通知父层：当前可见行（复制用）+ 可见区间（状态栏页码）
let notifyTimer: ReturnType<typeof setTimeout> | undefined;
function notify() {
  clearTimeout(notifyTimer);
  notifyTimer = setTimeout(() => {
    emit("visible-rows", visibleRows.value.slice());
    emit("range", { start: startIdx.value, end: endIdx.value });
  }, 120);
}

function onScroll() {
  const el = wrap.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  viewportH.value = el.clientHeight;
  notify();
}

let ro: ResizeObserver | undefined;
onMounted(() => {
  const el = wrap.value;
  if (el) {
    viewportH.value = el.clientHeight;
    ro = new ResizeObserver(() => {
      if (wrap.value) viewportH.value = wrap.value.clientHeight;
    });
    ro.observe(el);
  }
  notify();
});
onBeforeUnmount(() => ro?.disconnect());

function fmtCell(v: CellValue): { text: string; isNull: boolean } {
  if (v === null || v === undefined) return { text: "", isNull: true };
  return { text: typeof v === "object" ? JSON.stringify(v) : String(v), isNull: false };
}

// ---- 点单元格复制（短暂高亮反馈） ----
const flash = reactive(new Set<string>());
async function onCellClick(ri: number, ci: number, v: CellValue) {
  if (v === null || v === undefined) return;
  if (!(await copyText(fmtCell(v).text))) return;
  const key = `${ri}:${ci}`;
  flash.add(key);
  setTimeout(() => flash.delete(key), 700);
}

// ---- 行详情 ----
const detailIndex = ref<number | null>(null);
/** 全局行号 i 的行数据；排序模式取快照，普通模式走分页缓存（响应式补页）。 */
const detailRow = computed<CellValue[] | null>(() => {
  const i = detailIndex.value;
  const query = q.value;
  if (i === null || !query) return null;
  if (query.sortView) return query.sortView[i] ?? null;
  const rows = rowsFor(query, i, i + 1);
  return rows[0] && rows[0].length ? rows[0] : null;
});

function emptyText(): string {
  if (!q.value) return "";
  if (q.value.receipt) return "";
  if (q.value.running) return "";
  return columns.value.length ? state.t.rowsEmpty : "";
}
</script>

<template>
  <div class="result-root" :class="state.density">
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

    <!-- 空态：未运行过查询给操作提示；跑完 0 行给结果说明 -->
    <div v-else-if="!q || (total === 0 && columns.length === 0)" class="center-note">
      <div>
        <div style="font-size: 28px; opacity: 0.5">{{ q && q.running ? "⏳" : q ? "∅" : "⌨" }}</div>
        <div style="margin-top: 8px">{{ q && q.running ? state.t.executing : emptyText() || state.t.emptyEditorHint }}</div>
      </div>
    </div>

    <!-- 虚拟网格 -->
    <div v-else ref="wrap" class="grid-wrap" @scroll.passive="onScroll">
      <div class="grid-inner" :style="{ height: `${gridH + HEADER_H}px`, width: `${totalWidth}px` }">
        <div class="grid-header" :style="{ width: `${totalWidth}px` }">
          <div class="hcell corner" :style="{ width: `${GUTTER_W}px` }">#</div>
          <div
            v-for="(c, i) in columns"
            :key="i"
            class="hcell sortable"
            :style="{ width: `${colW(i)}px` }"
            :title="`${c.name} · ${c.typeName} · ${state.t.sortHint}`"
            @click="onHeaderClick(i)"
          >
            <div class="hname">
              {{ c.name }}
              <span v-if="q?.sort?.col === i" class="sort-arrow">{{ q.sort.dir === 1 ? "▲" : "▼" }}</span>
            </div>
            <div class="htype">{{ c.typeName }}</div>
            <span class="col-resizer" @mousedown.stop.prevent="onResizeDown($event, i)" @click.stop @dblclick.stop />
          </div>
        </div>
        <div :style="{ position: 'absolute', top: `${startIdx * ROW_H + HEADER_H}px`, width: `${totalWidth}px` }">
          <div v-for="(row, ri) in visibleRows" :key="startIdx + ri" class="grid-row" :style="{ height: `${ROW_H}px` }">
            <div
              class="cell gutter"
              :style="{ width: `${GUTTER_W}px` }"
              :title="`${state.t.rowDetail} · ${startIdx + ri + 1}`"
              @click="detailIndex = startIdx + ri"
            >
              {{ startIdx + ri + 1 }}
            </div>
            <div
              v-for="(v, ci) in row"
              :key="ci"
              class="cell"
              :class="{ null: fmtCell(v).isNull, copied: flash.has(`${ri}:${ci}`) }"
              :style="{ width: `${colW(ci)}px` }"
              :title="fmtCell(v).text"
              @click="onCellClick(ri, ci, v)"
            >
              {{ fmtCell(v).isNull ? state.t.nullCell : fmtCell(v).text }}
            </div>
          </div>
        </div>
      </div>
    </div>

    <RowDetail
      v-if="detailIndex !== null"
      :index="detailIndex"
      :row="detailRow"
      :columns="columns"
      :total="total"
      @close="detailIndex = null"
      @nav="(d) => (detailIndex = Math.min(total - 1, Math.max(0, detailIndex! + d)))"
    />
  </div>
</template>
