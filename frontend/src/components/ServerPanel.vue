<script setup lang="ts">
// 服务器概览：身份、存储统计、运行中的查询（可取消）、会话（admin 可踢）。
import { computed, onMounted } from "vue";
import {
  state,
  loadServerPanel,
  cancelServerQuery,
  kickSession,
  type RunningQuery,
  type AdminSessionRow,
} from "../store";
import { humanBytes } from "../i18n";

const t = computed(() => state.t);

onMounted(() => {
  void loadServerPanel();
});

function fmtNum(n: number): string {
  return n.toLocaleString(state.lang === "zh" ? "zh-CN" : "en-US");
}

function fmtMs(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

function fmtIdle(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  return `${Math.floor(s / 3600)}h`;
}

async function cancel(q: RunningQuery) {
  if (!window.confirm(t.value.confirmCancelQuery)) return;
  try {
    await cancelServerQuery(q.id);
  } catch {
    /* 错误已入消息面板 */
  }
}

async function kick(s: AdminSessionRow) {
  if (!window.confirm(t.value.confirmKick)) return;
  try {
    await kickSession(s.id);
  } catch {
    /* 错误已入消息面板 */
  }
}
</script>

<template>
  <div class="server-panel">
    <div class="server-section">
      <div class="stat-card full">
        <div class="label">{{ t.serverPanel }}</div>
        <div class="identity">
          <span class="badge">{{ state.info?.version ?? "?" }}</span>
          <span class="strong">{{ state.info?.user ?? t.anonymous }}</span>
          <span v-if="state.info?.role" class="muted">· {{ state.info.role }}</span>
          <span class="muted">@ {{ state.info?.defaultDb }}</span>
          <span v-if="state.info?.readOnly" class="badge warn">{{ t.readOnlyBadge }}</span>
        </div>
      </div>
    </div>

    <div class="server-section" v-if="state.server.stats">
      <div class="section-title">{{ t.storageStats }}</div>
      <div class="stat-row">
        <div class="stat-card grow">
          <div class="label">{{ t.storeBytes }}</div>
          <div class="value">{{ humanBytes(state.server.stats.storeBytes, state.lang) }}</div>
        </div>
        <div class="stat-card grow">
          <div class="label">{{ t.tables }}</div>
          <div class="value">{{ fmtNum(state.server.stats.tables) }}</div>
        </div>
      </div>
      <div v-if="state.server.stats.writeBuffer.length" class="write-buffer">
        <div class="muted small">{{ t.writeBuffer }}</div>
        <div v-for="w in state.server.stats.writeBuffer" :key="w.table" class="wb-row">
          <span class="name" :title="w.table">{{ w.table }}</span>
          <span class="mono">{{ fmtNum(w.batches) }} · {{ humanBytes(w.bytes, state.lang) }}</span>
        </div>
      </div>
    </div>

    <div class="server-section">
      <div class="section-title-row">
        <span class="section-title">{{ t.runningQueries }}</span>
        <span class="spacer" style="flex: 1" />
        <button class="dbx-btn dbx-btn--ghost" style="height: 20px; font-size: 11px" :disabled="state.server.loading" @click="loadServerPanel()">
          ⟳
        </button>
      </div>
      <div v-if="state.server.error" class="muted small">{{ state.server.error }}</div>
      <div v-else-if="state.server.queries.length === 0" class="muted small">{{ t.noRunningQueries }}</div>
      <div v-for="q in state.server.queries" :key="q.id" class="query-row">
        <div class="query-meta">
          <span class="mono small" :title="q.id">{{ q.id }}</span>
          <span class="badge" style="height: 16px">{{ fmtMs(q.elapsedMs) }}</span>
          <span class="muted small">{{ t.memory }} {{ humanBytes(q.memoryBytes, state.lang) }}</span>
          <span class="spacer" style="flex: 1" />
          <button class="dbx-btn dbx-btn--danger" style="height: 20px; font-size: 11px" @click="cancel(q)">
            {{ t.cancelQuery }}
          </button>
        </div>
        <div class="query-sql mono" :title="q.sql">{{ q.sql }}</div>
      </div>
    </div>

    <div class="server-section">
      <div class="section-title">{{ t.sessions }}</div>
      <div v-if="state.server.sessionsError" class="muted small">{{ t.sessionsAdminOnly }}</div>
      <template v-else>
        <div v-if="state.server.sessions.length === 0" class="muted small">—</div>
        <div v-for="s in state.server.sessions" :key="s.id" class="session-row">
          <div class="query-meta">
            <span class="strong">{{ s.user }}</span>
            <span v-if="s.role" class="badge" style="height: 16px">{{ s.role }}</span>
            <span class="muted small">@{{ s.defaultDb }}</span>
            <span class="muted small">{{ t.idle }} {{ fmtIdle(s.idleMs) }}</span>
            <span class="muted small">{{ t.running }} {{ s.runningQueries.length }}</span>
            <span class="spacer" style="flex: 1" />
            <button class="dbx-btn dbx-btn--danger" style="height: 20px; font-size: 11px" @click="kick(s)">
              {{ t.kick }}
            </button>
          </div>
          <div class="muted small mono">{{ s.client }} · {{ s.queriesTotal }}q · {{ fmtNum(s.rowsWritten) }}w</div>
        </div>
      </template>
    </div>
  </div>
</template>
