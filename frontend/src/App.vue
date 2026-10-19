<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  state,
  init,
  runSqlText,
  runStatements,
  stopQuery,
  refresh,
  showPartitions,
  retryLoad,
  isReadOnlySql,
  exportQuery,
  clearSort,
  pushMessage,
  draftChanged,
  toggleDensity,
} from "./store";
import { splitStatements } from "./sqlkit";
import { invoke } from "./bridge";
import { copyText } from "./clipboard";
import { errorHint, leadingCode } from "./errors";
import SchemaTree from "./components/SchemaTree.vue";
import SqlEditor from "./components/SqlEditor.vue";
import ResultGrid from "./components/ResultGrid.vue";
import PartitionPanel from "./components/PartitionPanel.vue";
import ServerPanel from "./components/ServerPanel.vue";
import DdlModal from "./components/DdlModal.vue";
import type { TableInfo } from "./store";

const showHistory = ref(false);
const sidebarTab = ref<"objects" | "server">("objects");
const t = computed(() => state.t);

const running = computed(() => state.query?.running === true);
const runDisabled = computed(() => !state.connectionId || running.value);
const stopDisabled = computed(() => !running.value);
const uiBlocked = computed(() => state.info?.readOnly === true && !isReadOnlySql(state.sql));

const headerMeta = computed(() => {
  const i = state.info;
  if (!i) return t.value.connecting;
  const who = i.user ?? t.value.anonymous;
  const role = i.role ? ` · ${i.role}` : "";
  return `${i.version} · ${who}@${i.defaultDb}${role}`;
});

const visibleRange = ref<{ start: number; end: number }>({ start: 0, end: 0 });
const pageLabel = computed(() => {
  const n = state.query?.totalRows ?? 0;
  if (n === 0) return "";
  const page = Math.floor(visibleRange.value.start / 500) + 1;
  const pages = Math.max(1, Math.ceil(n / 500));
  return `${t.value.page} ${page} / ${pages}`;
});

const receivedLabel = computed(() => {
  const q = state.query;
  if (!q || q.receipt) return "";
  const n = q.totalRows || q.receivedRows;
  if (!q.done) return t.value.rowsReceiving(String(n));
  return t.value.rowsLoaded(String(n), String(q.totalRows));
});

const elapsedLabel = computed(() => {
  const q = state.query;
  if (!q?.elapsedMs) return "";
  return `${t.value.elapsed} ${(q.elapsedMs / 1000).toFixed(2)}s${
    q.serverElapsedMs ? ` · ${t.value.serverElapsed} ${(q.serverElapsedMs / 1000).toFixed(2)}s` : ""
  }`;
});

const errorBanner = computed(() => {
  const q = state.query;
  if (q?.error && !q.cancelled) return { code: q.error.code, message: q.error.message };
  return null;
});

/** 错误码悬停说明（横幅与消息面板共用）。 */
function hintFor(code: string | null): string | undefined {
  return (code && errorHint(code, state.lang)) || undefined;
}
function msgHint(text: string): string | undefined {
  return hintFor(leadingCode(text));
}

onMounted(() => {
  void init();
});

const editor = ref<InstanceType<typeof SqlEditor> | null>(null);

function run() {
  // 有选中只运行选中；整篇按顶层分号拆分，多语句顺序执行
  const target = editor.value?.runTarget();
  if (!target?.text.trim()) return;
  const parts = splitStatements(target.text);
  if (parts.length > 1) void runStatements(parts);
  else void runSqlText(target.text);
}

function formatEditor() {
  editor.value?.format();
}

const savingScript = ref(false);
async function saveScript() {
  if (savingScript.value) return;
  const text = editor.value?.docText() ?? "";
  if (!text.trim()) return;
  savingScript.value = true;
  try {
    const r = await invoke<{ path: string; bytes: number }>("qsdb/script/save", { sql: text });
    state.lastFile = r.path;
    pushMessage("receipt", `${state.t.scriptSaved}: ${r.path}`);
  } catch (e) {
    pushMessage("error", e instanceof Error ? e.message : String(e));
  } finally {
    savingScript.value = false;
  }
}

const fileInput = ref<HTMLInputElement | null>(null);
function pickScript() {
  fileInput.value?.click();
}
async function onFilePicked(ev: Event) {
  const inp = ev.target as HTMLInputElement;
  const file = inp.files?.[0];
  if (!file) return;
  try {
    editor.value?.setDoc(await file.text());
  } catch (e) {
    pushMessage("error", e instanceof Error ? e.message : String(e));
  }
  inp.value = "";
}

function stop() {
  void stopQuery();
}

function restoreHistory(sql: string) {
  showHistory.value = false;
  draftChanged(sql);
}

function changeDb(db: string) {
  // currentDb 只影响新查询的默认库；树是全量的，不需要重载元数据。
  state.currentDb = db;
}

const ddlTable = ref<TableInfo | null>(null);

function onTableAction(action: { kind: "partitions" | "preview" | "ddl"; table: string }) {
  if (action.kind === "partitions") void showPartitions(action.table);
  if (action.kind === "preview") {
    const sqlText = `SELECT * FROM ${action.table} LIMIT 100`;
    draftChanged(sqlText);
    void runSqlText(sqlText);
  }
  if (action.kind === "ddl") {
    ddlTable.value = state.tables.find((tb) => tb.name === action.table) ?? null;
  }
}

const copyTimer = ref<ReturnType<typeof setTimeout> | undefined>();
const copiedFlash = ref(false);
function copyGrid(fmt: "tsv" | "csv") {
  const q = state.query;
  if (!q) return;
  const rows: string[][] = [
    q.columns.map((c) => c.name),
    ...visibleRows.value.map((r) => r.map((v) => (v === null ? "" : String(v)))),
  ];
  const sep = fmt === "tsv" ? "\t" : ",";
  const quote = (s: string) => (fmt === "csv" && /[",\n]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s);
  const text = rows.map((r) => r.map(quote).join(sep)).join("\n");
  void copyText(text).then((ok) => {
    if (!ok) return;
    copiedFlash.value = true;
    clearTimeout(copyTimer.value);
    copyTimer.value = setTimeout(() => (copiedFlash.value = false), 800);
  });
}

const exporting = ref(false);
async function doExport(fmt: "csv" | "tsv") {
  if (exporting.value || !state.query?.queryId || state.query.receipt) return;
  exporting.value = true;
  try {
    await exportQuery(fmt);
  } catch (e) {
    pushMessage("error", e instanceof Error ? e.message : String(e));
  } finally {
    exporting.value = false;
  }
}

async function copyLastFilePath() {
  await copyText(state.lastFile);
}

const sortColName = computed(() => {
  const s = state.query?.sort;
  if (!s) return "";
  return state.query?.columns[s.col]?.name ?? `#${s.col}`;
});
const lastFileName = computed(() => state.lastFile.split(/[\\/]/).pop() ?? state.lastFile);

const visibleRows = ref<unknown[][]>([]);
function onVisibleRows(rows: unknown[][]) {
  visibleRows.value = rows;
}
function onRange(range: { start: number; end: number }) {
  visibleRange.value = range;
}
</script>

<template>
  <div class="qsdb-app">
    <header class="qsdb-header">
      <span class="brand">{{ t.appName }}</span>
      <span class="meta">{{ headerMeta }}</span>
      <span v-if="state.info?.readOnly" class="badge warn">{{ t.readOnlyBadge }}</span>
      <span class="spacer" />
      <button class="dbx-btn" :title="t.refresh" @click="refresh()">{{ t.refresh }}</button>
    </header>

    <div v-if="!state.connectionId || state.gone" class="center-note">
      <div>{{ state.gone ? t.connectionGone : t.noConnection }}</div>
    </div>

    <div v-else class="qsdb-main">
      <aside class="qsdb-sidebar">
        <div class="sidebar-tabs">
          <button class="sidebar-tab" :class="{ active: sidebarTab === 'objects' }" @click="sidebarTab = 'objects'">
            {{ t.tabObjects }}
          </button>
          <button class="sidebar-tab" :class="{ active: sidebarTab === 'server' }" @click="sidebarTab = 'server'">
            {{ t.tabServer }}
          </button>
        </div>
        <SchemaTree
          v-show="sidebarTab === 'objects'"
          :filter-placeholder="t.filterPlaceholder"
          :no-tables="t.noTables"
          :refresh-label="t.refresh"
          @table-action="onTableAction"
        />
        <ServerPanel v-show="sidebarTab === 'server'" />
      </aside>

      <section class="qsdb-content">
        <div v-if="state.loadError" class="error-banner">
          <span class="code help" :title="hintFor(leadingCode(state.loadError))">{{ leadingCode(state.loadError) ?? "METADATA" }}</span>
          <span>{{ state.loadError }}</span>
          <button class="dbx-btn" @click="retryLoad()">{{ t.refresh }}</button>
        </div>
        <div class="qsdb-editor-zone">
          <div class="qsdb-toolbar" style="position: relative">
            <button class="dbx-btn dbx-btn--primary" :disabled="runDisabled || uiBlocked" :title="t.titleRun" @click="run">
              {{ running ? t.executing : t.run }}
            </button>
            <button class="dbx-btn dbx-btn--danger" :disabled="stopDisabled" :title="t.titleStop" @click="stop">
              {{ state.query?.cancelled ? t.cancelled : t.stop }}
            </button>
            <select class="dbx-select" :value="state.currentDb" aria-label="database" @change="changeDb(($event.target as HTMLSelectElement).value)">
              <option v-for="d in state.databases" :key="d.name" :value="d.name">
                {{ d.name }}{{ d.readOnly ? " 🔒" : "" }}
              </option>
            </select>
            <span v-if="state.query?.running" class="badge">{{ state.query.cancelled ? t.cancelling : t.executing }}</span>
            <span class="spacer" style="flex: 1" />
            <button class="dbx-btn dbx-btn--ghost" :title="t.titleFormat" @click="formatEditor">{{ t.formatSqlBtn }}</button>
            <button class="dbx-btn dbx-btn--ghost" :title="t.titleOpen" @click="pickScript">{{ t.openScript }}</button>
            <button class="dbx-btn dbx-btn--ghost" :disabled="savingScript" :title="t.titleSave" @click="saveScript">{{ t.saveScript }}</button>
            <button class="dbx-btn dbx-btn--ghost" @click="showHistory = !showHistory">{{ t.history }}</button>
            <input ref="fileInput" type="file" accept=".sql,.txt" style="display: none" @change="onFilePicked" />
            <div v-if="showHistory" class="history-pop">
              <div v-if="state.history.length === 0" class="grid-empty">{{ t.noHistory }}</div>
              <div v-for="h in state.history" :key="h.at" class="history-item" :title="h.sql" @click="restoreHistory(h.sql)">
                {{ h.sql }}
              </div>
            </div>
          </div>
          <SqlEditor ref="editor" :placeholder="t.editorPlaceholder" @run="run" @format="formatEditor" @save="saveScript" />
        </div>

        <div v-if="errorBanner" class="error-banner">
          <span class="code help" :title="hintFor(errorBanner.code)">{{ errorBanner.code }}</span>
          <span>{{ errorBanner.message }}</span>
          <button class="dbx-btn" @click="state.query!.error = null">✕</button>
        </div>

        <div class="qsdb-tabs">
          <button class="qsdb-tab" :class="{ active: state.activeTab === 'results' }" @click="state.activeTab = 'results'">
            {{ t.tabResults }}<span v-if="state.query && !state.query.receipt" class="count">{{ state.query.totalRows }}</span>
          </button>
          <button class="qsdb-tab" :class="{ active: state.activeTab === 'partitions' }" @click="state.activeTab = 'partitions'">
            {{ t.tabPartitions }}
          </button>
          <button class="qsdb-tab" :class="{ active: state.activeTab === 'messages' }" @click="state.activeTab = 'messages'">
            {{ t.tabMessages }}<span v-if="state.messages.length" class="count">{{ state.messages.length }}</span>
          </button>
        </div>

        <div v-show="state.activeTab === 'results'" class="qsdb-panel">
          <ResultGrid @visible-rows="onVisibleRows" @range="onRange" />
        </div>
        <div v-show="state.activeTab === 'partitions'" class="qsdb-panel">
          <PartitionPanel />
        </div>
        <div v-show="state.activeTab === 'messages'" class="qsdb-panel">
          <div class="msg-list">
            <div v-if="state.messages.length === 0" class="grid-empty">{{ t.messagesEmpty }}</div>
            <div v-for="m in state.messages" :key="m.at" class="msg" :class="m.kind" :title="m.kind === 'error' ? msgHint(m.text) : undefined">
            {{ m.text }}
          </div>
          </div>
        </div>

        <footer class="qsdb-statusbar">
          <span>{{ receivedLabel }}</span>
          <span>{{ pageLabel }}</span>
          <span>{{ elapsedLabel }}</span>
          <span v-if="state.query?.cancelled" class="badge warn">{{ t.cancelled }}</span>
          <span v-if="state.query?.sort" class="badge clickable" :title="t.clearSort" @click="clearSort()">
            {{ sortColName }} {{ state.query.sort.dir === 1 ? "▲" : "▼" }} ✕
          </span>
          <span v-if="state.query?.sortPreparing" class="badge">{{ t.sortHint }}…</span>
          <span class="spacer" style="flex: 1" />
          <span v-if="copiedFlash" class="muted small">{{ t.copied }}</span>
          <button class="dbx-btn dbx-btn--ghost" style="height: 22px; font-size: 11px" :title="t.density" @click="toggleDensity()">
            {{ state.density === "compact" ? "≡" : "☰" }}
          </button>
          <template v-if="state.query && !state.query.receipt && state.query.columns.length">
            <button class="dbx-btn dbx-btn--ghost" style="height: 22px; font-size: 11px" @click="copyGrid('tsv')">{{ t.copyTsv }}</button>
            <button class="dbx-btn dbx-btn--ghost" style="height: 22px; font-size: 11px" @click="copyGrid('csv')">{{ t.copyCsv }}</button>
            <button
              class="dbx-btn dbx-btn--ghost"
              style="height: 22px; font-size: 11px"
              :disabled="exporting || !state.query.queryId"
              @click="doExport('csv')"
            >
              {{ t.exportCsv }}
            </button>
            <button
              class="dbx-btn dbx-btn--ghost"
              style="height: 22px; font-size: 11px"
              :disabled="exporting || !state.query.queryId"
              @click="doExport('tsv')"
            >
              {{ t.exportTsv }}
            </button>
          </template>
          <span v-if="state.lastFile" class="badge clickable" :title="state.lastFile" @click="copyLastFilePath()">
            {{ lastFileName }} · {{ t.copyPath }}
          </span>
        </footer>
      </section>
    </div>

    <DdlModal v-if="ddlTable" :table="ddlTable" @close="ddlTable = null" />
  </div>
</template>
