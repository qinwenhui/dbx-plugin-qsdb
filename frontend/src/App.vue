<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { state, init, runSql, stopQuery, refresh, showPartitions, loadMeta, isReadOnlySql } from "./store";
import { invoke } from "./bridge";
import SchemaTree from "./components/SchemaTree.vue";
import SqlEditor from "./components/SqlEditor.vue";
import ResultGrid from "./components/ResultGrid.vue";
import PartitionPanel from "./components/PartitionPanel.vue";

const showHistory = ref(false);
const t = computed(() => state.t);

const running = computed(() => state.query?.running === true);
const runDisabled = computed(() => !state.connectionId || running.value);
const stopDisabled = computed(() => !running.value);
const uiBlocked = computed(() => state.info?.readOnly === true && !isReadOnlySql(state.sql));

const headerMeta = computed(() => {
  const i = state.info;
  if (!i) return t.value.connecting;
  const who = i.user ? `${i.user}` : "anonymous";
  const role = i.role ? ` · ${i.role}` : "";
  return `${i.version} · ${who}@${i.defaultDb}${role}`;
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

onMounted(() => {
  void init();
});

function run() {
  void runSql();
}

function stop() {
  void stopQuery();
}

function restoreHistory(sql: string) {
  showHistory.value = false;
  void import("./store").then((m) => m.draftChanged(sql));
}

async function changeDb(db: string) {
  state.currentDb = db;
  try {
    await loadMeta();
  } catch {
    /* 错误已在 store 里处理 */
  }
}

function onTableAction(action: { kind: "partitions" | "preview"; table: string }) {
  if (action.kind === "partitions") void showPartitions(action.table);
  if (action.kind === "preview") {
    void import("./store").then(async (m) => {
      m.draftChanged(`SELECT * FROM ${action.table} LIMIT 100`);
      void runSql();
    });
  }
}

async function copyText(text: string) {
  try {
    await invoke("host.copy", { text });
  } catch {
    /* 剪贴板不可用（旧宿主）——静默 */
  }
}

const copyTimer = ref<ReturnType<typeof setTimeout> | undefined>();
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
  void copyText(text);
  clearTimeout(copyTimer.value);
  copyTimer.value = setTimeout(() => undefined, 800);
}

const visibleRows = ref<unknown[][]>([]);
function onVisibleRows(rows: unknown[][]) {
  visibleRows.value = rows;
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
        <SchemaTree :filter-placeholder="t.filterPlaceholder" :no-tables="t.noTables" :refresh-label="t.refresh" @table-action="onTableAction" />
      </aside>

      <section class="qsdb-content">
        <div class="qsdb-editor-zone">
          <div class="qsdb-toolbar" style="position: relative">
            <button class="dbx-btn dbx-btn--primary" :disabled="runDisabled || uiBlocked" @click="run">
              {{ running ? t.executing : t.run }}
            </button>
            <button class="dbx-btn dbx-btn--danger" :disabled="stopDisabled" @click="stop">
              {{ state.query?.cancelled ? t.cancelled : t.stop }}
            </button>
            <select class="dbx-select" :value="state.currentDb" aria-label="database" @change="changeDb(($event.target as HTMLSelectElement).value)">
              <option v-for="d in state.databases" :key="d.name" :value="d.name">
                {{ d.name }}{{ d.readOnly ? " 🔒" : "" }}
              </option>
            </select>
            <span v-if="state.query?.running" class="badge">{{ state.query.cancelled ? t.cancelling : t.executing }}</span>
            <span class="spacer" style="flex: 1" />
            <button class="dbx-btn dbx-btn--ghost" @click="showHistory = !showHistory">{{ t.history }}</button>
            <div v-if="showHistory" class="history-pop">
              <div v-if="state.history.length === 0" class="grid-empty">{{ t.noHistory }}</div>
              <div v-for="h in state.history" :key="h.at" class="history-item" :title="h.sql" @click="restoreHistory(h.sql)">
                {{ h.sql }}
              </div>
            </div>
          </div>
          <SqlEditor :placeholder="t.editorPlaceholder" @run="run" />
        </div>

        <div v-if="errorBanner" class="error-banner">
          <span class="code">{{ errorBanner.code }}</span>
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
          <ResultGrid @visible-rows="onVisibleRows" />
        </div>
        <div v-show="state.activeTab === 'partitions'" class="qsdb-panel">
          <PartitionPanel />
        </div>
        <div v-show="state.activeTab === 'messages'" class="qsdb-panel">
          <div class="msg-list">
            <div v-if="state.messages.length === 0" class="grid-empty">{{ t.messagesEmpty }}</div>
            <div v-for="m in state.messages" :key="m.at" class="msg" :class="m.kind">{{ m.text }}</div>
          </div>
        </div>

        <footer class="qsdb-statusbar">
          <span>{{ receivedLabel }}</span>
          <span>{{ elapsedLabel }}</span>
          <span v-if="state.query?.cancelled" class="badge warn">{{ t.cancelled }}</span>
          <span class="spacer" style="flex: 1" />
          <template v-if="state.query && !state.query.receipt && state.query.columns.length">
            <button class="dbx-btn dbx-btn--ghost" style="height: 22px; font-size: 11px" @click="copyGrid('tsv')">{{ t.copyTsv }}</button>
            <button class="dbx-btn dbx-btn--ghost" style="height: 22px; font-size: 11px" @click="copyGrid('csv')">{{ t.copyCsv }}</button>
          </template>
        </footer>
      </section>
    </div>
  </div>
</template>
