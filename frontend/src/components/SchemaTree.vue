<script setup lang="ts">
// Schema 树：库 → 表 → 列。过滤、刷新；表行操作（预览/分区）。
import { computed, ref } from "vue";
import { state, refresh, loadMeta, draftChanged } from "../store";

defineProps<{ filterPlaceholder: string; noTables: string; refreshLabel: string }>();
const emit = defineEmits<{ (e: "table-action", action: { kind: "partitions" | "preview"; table: string }): void }>();

const filter = ref("");
const openDbs = ref(new Set<string>());
const openTables = ref(new Set<string>());

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

function toggleDb(db: string) {
  const s = new Set(openDbs.value);
  if (s.has(db)) s.delete(db);
  else s.add(db);
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

async function onSelectDb(db: string) {
  state.currentDb = db;
  await loadMeta();
}

function preview(table: string) {
  draftChanged(`SELECT * FROM ${table} LIMIT 100`);
  emit("table-action", { kind: "preview", table });
}
</script>

<template>
  <div class="sidebar-tools">
    <input v-model="filter" class="dbx-input" type="search" :placeholder="filterPlaceholder" />
    <button class="dbx-btn dbx-btn--ghost" style="height: 26px; flex: none" :title="refreshLabel" @click="doRefresh">⟳</button>
  </div>
  <div class="tree">
    <div v-if="filteredTables.length === 0 && state.tables.length > 0" class="grid-empty">{{ noTables }}</div>
    <div v-for="db in dbsWithTables" :key="db.name" class="db-group">
      <div class="db-head" @click="toggleDb(db.name)">
        <span class="chev">{{ openDbs.has(db.name) ? "▾" : "▸" }}</span>
        <span class="name" :title="db.name">{{ db.name }}<span v-if="db.readOnly" title="read-only"> 🔒</span></span>
        <span class="spacer" style="flex: 1" />
        <button
          v-if="state.currentDb !== db.name"
          class="dbx-btn dbx-btn--ghost"
          style="height: 20px; font-size: 11px; flex: none"
          title="use"
          @click.stop="onSelectDb(db.name)"
        >
          use
        </button>
      </div>
      <template v-if="openDbs.has(db.name) || filter">
        <div v-for="table in db.tables" :key="table.name">
          <div class="table-head" :class="{ active: state.partitionsTable === table.name }" @click="toggleTable(table.name)">
            <span class="chev">{{ openTables.has(table.name) ? "▾" : "▸" }}</span>
            <span class="name" :title="table.name">{{ table.name }}</span>
            <span class="spacer" style="flex: 1" />
            <button class="dbx-btn dbx-btn--ghost" style="height: 20px; font-size: 11px; flex: none" @click.stop="preview(table.name)">▦</button>
            <button
              class="dbx-btn dbx-btn--ghost"
              style="height: 20px; font-size: 11px; flex: none"
              title="partitions"
              @click.stop="emit('table-action', { kind: 'partitions', table: table.name })"
            >
              ⛭
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
    <div v-if="state.tables.length === 0 && filter" class="grid-empty">{{ noTables }}</div>
  </div>
</template>
