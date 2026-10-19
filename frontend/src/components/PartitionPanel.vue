<script setup lang="ts">
// 分区面板：generation / rows / segments / bytes + 表大小汇总。
// bytes 未知（旧内核）显示"未知"，绝不显示伪造的 0 B。
// 交互：最高 generation 标"最新"；每行可生成谓词预览 SQL（跑/复制）。
import { computed } from "vue";
import { state, showPartitions, runSqlText, draftChanged, type PartitionRow } from "../store";
import { humanBytes } from "../i18n";
import { partitionPairs, partitionPreviewSql } from "../sqlkit";
import { copyText } from "../clipboard";

const t = computed(() => state.t);
const rowsTotal = computed(() => state.partitions.reduce((a, p) => a + p.rows, 0));
const maxGeneration = computed(() => state.partitions.reduce((m, p) => Math.max(m, p.generation), 0));

function fmtNum(n: number): string {
  return n.toLocaleString(state.lang === "zh" ? "zh-CN" : "en-US");
}

function previewSql(p: PartitionRow): string {
  return partitionPreviewSql(state.partitionsTable, p.name);
}

function runPartition(p: PartitionRow) {
  const sqlText = previewSql(p);
  draftChanged(sqlText);
  void runSqlText(sqlText);
}

async function copyPartitionSql(p: PartitionRow) {
  await copyText(previewSql(p));
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
          <div class="stat-card">
            <div class="label">{{ t.generation }}</div>
            <div class="value">{{ fmtNum(maxGeneration) }}</div>
          </div>
        </div>

        <table class="dbx-table parts-table">
          <thead>
            <tr>
              <th>{{ t.partitionName }}</th>
              <th class="num">{{ t.generation }}</th>
              <th class="num">{{ t.rows }}</th>
              <th class="num">{{ t.segments }}</th>
              <th class="num">{{ t.bytes }}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="p in state.partitions" :key="p.name">
              <td>
                <span class="pchip" v-for="(pair, i) in partitionPairs(p.name)" :key="i" :title="pair.col ? `${pair.col} = ${pair.value}` : pair.value">
                  <template v-if="pair.col"><b>{{ pair.col }}</b>={{ pair.value }}</template>
                  <template v-else>{{ pair.value }}</template>
                </span>
              </td>
              <td class="num">
                {{ p.generation }}
                <span v-if="p.generation === maxGeneration && state.partitions.length > 1" class="badge" style="height: 15px">{{ t.latestBadge }}</span>
              </td>
              <td class="num">{{ fmtNum(p.rows) }}</td>
              <td class="num">{{ fmtNum(p.segments) }}</td>
              <td class="num" :style="p.bytesKnown ? '' : 'color: var(--color-muted-foreground)'">
                {{ p.bytesKnown ? humanBytes(p.bytes, state.lang) : humanBytes(null, state.lang) }}
              </td>
              <td style="white-space: nowrap; text-align: right">
                <button class="dbx-btn dbx-btn--ghost" style="height: 20px; font-size: 11px" :title="t.titlePartitionPreview" @click="runPartition(p)">
                  {{ t.preview }}
                </button>
                <button class="dbx-btn dbx-btn--ghost" style="height: 20px; font-size: 11px" :title="previewSql(p)" @click="copyPartitionSql(p)">
                  {{ t.copySql }}
                </button>
              </td>
            </tr>
            <tr v-if="state.partitions.length === 0">
              <td colspan="6" style="color: var(--color-muted-foreground)">—</td>
            </tr>
          </tbody>
        </table>
      </template>
    </template>
  </div>
</template>
