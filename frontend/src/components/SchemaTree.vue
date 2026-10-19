<script setup lang="ts">
// Schema 树：库 → 表 → 列。点库名 = 选中该库（后续查询的默认库）并展开；
// 首次加载自动展开当前库。表行操作（预览/分区/DDL）；表行可拖进编辑器
// 自动生成预览 SQL。
import { computed, ref, watch } from "vue";
import { state, refresh, draftChanged } from "../store";

const props = defineProps<{ filterPlaceholder: string; noTables: string; refreshLabel: string }>();
const t = computed(() => state.t);
const emit = defineEmits<{
  (e: "table-action", action: { kind: "partitions" | "preview" | "ddl"; table: string }): void;
}>();

const filter = ref("");
const openDbs = ref(new Set<string>());
const openTables = ref(new Set<string>());

/** 拖动表 → 编辑器。MIME 与 SqlEditor 的 drop 处理约定一致。 */
function onTableDragStart(event: DragEvent, table: string) {
  event.dataTransfer?.setData("application/x-qsdb-table", table);
  event.dataTransfer?.setData("text/plain", `SELECT * FROM ${table} LIMIT 100`);
  if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy";
}

const filteredTables = computed(() => {
  const q = filter.value.trim().toLowerCase();
  const list = q ? state.tables.filter((t) => t.name.toLowerCase().includes(q) || t.columns.some((c) => c.name.toLowerCase().includes(q))) : state.tables;
  return list;
});

const dbsWithTables = computed(() => {
  const map = new Map<string, typeof filteredTables.value>();
  for (const db of state.databases) map.set(db.name, []);
  for (const t of filteredTables.value) {
    if (!map.has(t.database)) map.set(t.database, []);
    map.get(t.database)!.push(t);
  }
  return [...map.entries()].map(([name, tables]) => ({
    name,
    readOnly: state.databases.find((d) => d.name === name)?.readOnly ?? false,
    tables,
  }));
});

// 首次有数据时自动展开当前库（以及无库归属时的第一个分组）
watch(
  dbsWithTables,
  (groups) => {
    if (groups.length === 0 || openDbs.value.size > 0) return;
    const target = groups.find((g) => g.name === state.currentDb) ?? groups[0];
    if (target) openDbs.value = new Set([target.name]);
  },
  { immediate: true },
);

/** 点库名：展开/收起；展开的同时把它设为当前库。 */
function clickDb(db: string) {
  const s = new Set(openDbs.value);
  if (s.has(db)) {
    s.delete(db);
  } else {
    s.add(db);
    state.currentDb = db;
  }
  openDbs.value = s;
}

function toggleTable(name: string) {
  const s = new Set(openTables.value);
  if (s.has(name)) s.delete(name);
  else s.add(name);
  openTables.value = s;
}

async function doRefresh() {
  await refresh();
}

function preview(table: string) {
  draftChanged(`SELECT * FROM ${table} LIMIT 100`);
  emit("table-action", { kind: "preview", table });
}

const showEmpty = computed(() => props.noTables !== "" && state.tables.length === 0);
</script>

<template>
  <div class="sidebar-tools">
    <input v-model="filter" class="dbx-input" type="search" :placeholder="filterPlaceholder" />
    <button class="dbx-btn dbx-btn--ghost" style="height: 26px; flex: none" :title="refreshLabel" @click="doRefresh">⟳</button>
  </div>
  <div class="tree">
    <div v-if="showEmpty" class="grid-empty">{{ noTables }}</div>
    <div v-for="db in dbsWithTables" :key="db.name" class="db-group">
      <div class="db-head" :class="{ current: state.currentDb === db.name }" @click="clickDb(db.name)">
        <span class="chev">{{ openDbs.has(db.name) ? "▾" : "▸" }}</span>
        <span class="name" :title="db.name">{{ db.name }}<span v-if="db.readOnly" :title="t.readOnlyBadge"> 🔒</span></span>
        <span class="spacer" style="flex: 1" />
        <span v-if="state.currentDb === db.name" class="badge" style="flex: none">use</span>
      </div>
      <template v-if="openDbs.has(db.name) || filter">
        <div v-for="table in db.tables" :key="table.name">
          <div
            class="table-head"
            :class="{ active: state.partitionsTable === table.name }"
            draggable="true"
            @dragstart="onTableDragStart($event, table.name)"
            @click="toggleTable(table.name)"
          >
            <span class="chev">{{ openTables.has(table.name) ? "▾" : "▸" }}</span>
            <span class="name" :title="table.name">{{ table.name }}</span>
            <span class="spacer" style="flex: 1" />
            <button class="dbx-btn dbx-btn--ghost" style="height: 20px; font-size: 11px; flex: none" :title="t.preview" @click.stop="preview(table.name)">▦</button>
            <button
              class="dbx-btn dbx-btn--ghost"
              style="height: 20px; font-size: 11px; flex: none"
              :title="t.viewPartitions"
              @click.stop="emit('table-action', { kind: 'partitions', table: table.name })"
            >
              ⛭
            </button>
            <button
              class="dbx-btn dbx-btn--ghost"
              style="height: 20px; font-size: 11px; flex: none"
              :title="t.titleDdl"
              @click.stop="emit('table-action', { kind: 'ddl', table: table.name })"
            >
              { }
            </button>
          </div>
          <div v-if="openTables.has(table.name)" class="cols">
            <div v-for="col in table.columns" :key="col.name" class="col">
              <span>{{ col.name }}</span>
              <span class="ty">{{ col.typeName }}</span>
              <span v-if="col.partition" class="pk">P</span>
            </div>
          </div>
        </div>
      </template>
    </div>
    <div v-if="!showEmpty && filter && filteredTables.length === 0 && state.tables.length > 0" class="grid-empty">{{ noTables }}</div>
  </div>
</template>
