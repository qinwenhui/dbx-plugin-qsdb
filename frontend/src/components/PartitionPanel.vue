<script setup lang="ts">
// 分区面板：generation / rows / segments / bytes + 表大小汇总。
// bytes 未知（旧内核）显示"未知"，绝不显示伪造的 0 B。
import { computed } from "vue";
import { state, showPartitions } from "../store";
import { humanBytes } from "../i18n";

const t = computed(() => state.t);
const rowsTotal = computed(() => state.partitions.reduce((a, p) => a + p.rows, 0));

function fmtNum(n: number): string {
  return n.toLocaleString(state.lang === "zh" ? "zh-CN" : "en-US");
}
</script>

<template>
  <div class="parts-panel">
    <div v-if="!state.partitionsTable" class="center-note">
      <div>{{ t.selectTable }}</div>
    </div>
    <template v-else>
      <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 10px; flex-wrap: wrap">
        <strong>{{ t.partitionsFor(state.partitionsTable) }}</strong>
        <span class="badge">{{ t.partitionsCount(String(state.partitions.length)) }}</span>
        <span class="spacer" style="flex: 1" />
        <button class="dbx-btn" @click="showPartitions(state.partitionsTable)">{{ t.refresh }}</button>
      </div>

      <div v-if="state.partitionsLoading" class="grid-empty">{{ t.loading }}</div>
      <div v-else-if="state.partitionsError" class="error-banner">
        <span>{{ state.partitionsError }}</span>
      </div>
      <template v-else>
        <div class="parts-summary">
          <div class="stat-card">
            <div class="label">{{ t.rows }}</div>
            <div class="value">{{ fmtNum(rowsTotal) }}</div>
          </div>
          <div class="stat-card">
            <div class="label">{{ t.bytes }}</div>
            <div class="value">
              {{ state.partitionsSize?.bytesKnown ? humanBytes(state.partitionsSize?.bytes, state.lang) : humanBytes(null, state.lang) }}
            </div>
          </div>
          <div class="stat-card">
            <div class="label">{{ t.partitions }}</div>
            <div class="value">{{ fmtNum(state.partitionsSize?.partitionCount ?? state.partitions.length) }}</div>
          </div>
        </div>

        <table class="dbx-table">
          <thead>
            <tr>
              <th>{{ t.partitionName }}</th>
              <th class="num">{{ t.generation }}</th>
              <th class="num">{{ t.rows }}</th>
              <th class="num">{{ t.segments }}</th>
              <th class="num">{{ t.bytes }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="p in state.partitions" :key="p.name">
              <td>{{ p.name }}</td>
              <td class="num">{{ p.generation }}</td>
              <td class="num">{{ fmtNum(p.rows) }}</td>
              <td class="num">{{ fmtNum(p.segments) }}</td>
              <td class="num" :style="p.bytesKnown ? '' : 'color: var(--color-muted-foreground)'">
                {{ p.bytesKnown ? humanBytes(p.bytes, state.lang) : humanBytes(null, state.lang) }}
              </td>
            </tr>
            <tr v-if="state.partitions.length === 0">
              <td colspan="5" style="color: var(--color-muted-foreground)">—</td>
            </tr>
          </tbody>
        </table>
      </template>
    </template>
  </div>
</template>
