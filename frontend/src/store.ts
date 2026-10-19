// 工作台状态：连接、元数据、查询生命周期、分页缓存、历史。
// 单例 reactive store；组件直接 import 使用。
import { reactive, readonly } from "vue";
import { api, invoke, InvokeError, onQueryEvent, onEnvChange, type DbxColumn, type DbxContext } from "./bridge";
import { langFor, t, type Lang, type Messages } from "./i18n";

export type CellValue = null | boolean | number | string;

export interface ServerInfo {
  version: string;
  user: string | null;
  role: string | null;
  defaultDb: string;
  readOnly: boolean;
}

export interface DbInfo {
  name: string;
  readOnly: boolean;
}

export interface TableInfo {
  name: string; // 展示名（多挂载为 db.table 限定形态）
  database: string;
  bareName: string;
  columns: { name: string; typeName: string; nullable: boolean; partition: boolean }[];
}

export interface PartitionRow {
  name: string;
  generation: number;
  rows: number;
  segments: number;
  bytes: number | null; // null = 未知（旧内核），UI 显示"未知"，不伪造 0 B
  bytesKnown: boolean;
}

export interface ExecReceipt {
  command: string;
  message: string | null;
  rowsAffected: number;
}

export interface QueryState {
  queryId: string;
  sql: string;
  columns: DbxColumn[];
  totalRows: number;
  receivedRows: number;
  done: boolean; // 流结束（finished 或 error/cancel）
  finishedOk: boolean;
  cancelled: boolean;
  error: { code: string; message: string } | null;
  running: boolean;
  elapsedMs: number | null;
  serverElapsedMs: number | null;
  receipt: ExecReceipt | null;
  pages: Map<number, CellValue[][]>; // offset → 实际返回的行
  pending: Set<number>;
}

interface HistoryItem {
  sql: string;
  at: number;
}

const PAGE_SIZE = 500;
const MAX_CACHED_PAGES = 80;
const MAX_HISTORY = 50;

const state = reactive({
  locale: "en",
  lang: "en" as Lang,
  t: t("en") as Messages,
  connectionId: "",
  connectionName: "",
  ready: false,
  gone: false, // 连接已断开（CONNECTION_NOT_FOUND）
  loadError: "" as string,
  info: null as ServerInfo | null,
  databases: [] as DbInfo[],
  tables: [] as TableInfo[],
  currentDb: "",
  sql: "",
  query: null as QueryState | null,
  activeTab: "results" as "results" | "partitions" | "messages",
  messages: [] as { kind: "info" | "error" | "receipt"; text: string; at: number }[],
  history: [] as HistoryItem[],
  partitionsTable: "",
  partitions: [] as PartitionRow[],
  partitionsSize: null as { rows: number; partitionCount: number; bytes: number | null; bytesKnown: boolean } | null,
  partitionsLoading: false,
  partitionsError: "",
});

function setLocale(locale: string) {
  state.locale = locale;
  state.lang = langFor(locale);
  state.t = t(locale);
}

function pushMessage(kind: "info" | "error" | "receipt", text: string) {
  state.messages.unshift({ kind, text, at: Date.now() });
  if (state.messages.length > 200) state.messages.pop();
}

function qsdbCodeOf(e: unknown): string {
  return e instanceof InvokeError ? (e.qsdbCode ?? "ERROR") : "ERROR";
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

async function loadHistory() {
  try {
    const raw = await api().storage.get("qsdb.history");
    if (Array.isArray(raw)) state.history = raw as HistoryItem[];
  } catch {
    /* storage 不可用（旧宿主）就不持久化 */
  }
}

async function saveHistory(sql: string) {
  state.history = [{ sql, at: Date.now() }, ...state.history.filter((h) => h.sql !== sql)].slice(0, MAX_HISTORY);
  try {
    await api().storage.set("qsdb.history", state.history);
  } catch {
    /* 忽略：历史是非关键数据 */
  }
}

async function loadDraft() {
  try {
    const draft = await api().storage.get("qsdb.draft");
    if (typeof draft === "string" && draft) state.sql = draft;
  } catch {
    /* 同上 */
  }
}

let draftTimer: ReturnType<typeof setTimeout> | undefined;
export function draftChanged(sql: string) {
  state.sql = sql;
  clearTimeout(draftTimer);
  draftTimer = setTimeout(() => {
    api().storage.set("qsdb.draft", sql).catch(() => undefined);
  }, 400);
}

export async function init() {
  const a = api();
  setLocale(a.locale);
  onEnvChange((env) => {
    if (env.locale) setLocale(env.locale);
  });
  subscribeEvents();
  const ctx: DbxContext = await a.ready;
  a.onContext((c) => {
    if (c.connectionId && c.connectionId !== state.connectionId) {
      state.connectionId = c.connectionId;
      void bootstrapConnection();
    }
  });
  state.connectionId = ctx.connectionId ?? "";
  if (!state.connectionId) {
    state.ready = true;
    return;
  }
  await loadHistory();
  await loadDraft();
  await bootstrapConnection();
}

async function bootstrapConnection() {
  state.gone = false;
  state.loadError = "";
  try {
    state.info = await invoke<ServerInfo>("qsdb/server/info", { connectionId: state.connectionId });
    state.currentDb = state.info.defaultDb;
    await loadMeta();
    state.ready = true;
  } catch (e) {
    if (e instanceof InvokeError && e.isConnectionGone) {
      state.gone = true;
    } else {
      state.loadError = messageOf(e);
    }
    state.ready = true;
  }
}

export async function loadMeta() {
  const meta = await invoke<{ databases: DbInfo[]; tables: TableInfo[] }>("qsdb/metadata", {
    connectionId: state.connectionId,
    database: state.currentDb || undefined,
  });
  state.databases = meta.databases;
  state.tables = meta.tables;
}

export async function refresh() {
  try {
    state.info = await invoke<ServerInfo>("qsdb/server/info", { connectionId: state.connectionId });
    await loadMeta();
  } catch (e) {
    handleConnectionLoss(e);
  }
}

function handleConnectionLoss(e: unknown): boolean {
  if (e instanceof InvokeError && e.isConnectionGone) {
    state.gone = true;
    return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// 查询
// ---------------------------------------------------------------------------

function newQuery(queryId: string, sql: string): QueryState {
  return {
    queryId,
    sql,
    columns: [],
    totalRows: 0,
    receivedRows: 0,
    done: false,
    finishedOk: false,
    cancelled: false,
    error: null,
    running: true,
    elapsedMs: null,
    serverElapsedMs: null,
    receipt: null,
    pages: new Map(),
    pending: new Set(),
  };
}

/** UI 层只读拦截（Sidecar 层是第二道；服务端授权是最终边界）。 */
export function isReadOnlySql(sql: string): boolean {
  return /^(select|with|show|explain|values|describe|desc)\b/i.test(sql.trim().replace(/^(--.*\n|#[^\n]*\n|\/\*[\s\S]*?\*\/)+/s, "").trimStart());
}

export async function runSql() {
  const sql = state.sql.trim();
  if (!sql) return;
  if (state.query?.running) return;
  if (state.info?.readOnly && !isReadOnlySql(sql)) {
    pushMessage("error", state.t.readOnlyBlocked);
    return;
  }
  const q = newQuery("", sql);
  state.query = q;
  state.activeTab = "results";
  try {
    const r = await invoke<{ queryId: string }>("qsdb/query/start", {
      connectionId: state.connectionId,
      sql,
      database: state.currentDb || undefined,
    });
    q.queryId = r.queryId;
    void saveHistory(sql);
  } catch (e) {
    q.running = false;
    q.done = true;
    q.error = { code: qsdbCodeOf(e), message: messageOf(e) };
    pushMessage("error", `${q.error.code}: ${q.error.message}`);
    handleConnectionLoss(e);
  }
}

export async function stopQuery() {
  const q = state.query;
  if (!q?.running || !q.queryId) return;
  try {
    await invoke("qsdb/query/stop", { connectionId: state.connectionId, queryId: q.queryId });
  } catch (e) {
    pushMessage("error", messageOf(e));
  }
}

function subscribeEvents() {
  onQueryEvent((method, p) => {
    const q = state.query;
    if (!q || p.connectionId !== state.connectionId || p.queryId !== q.queryId) return;
    if (method === "qsdb/query/started") {
      q.columns = (p.columns as DbxColumn[]) ?? [];
    } else if (method === "qsdb/query/progress") {
      q.receivedRows = (p.receivedRows as number) ?? q.receivedRows;
      q.totalRows = q.receivedRows;
    } else if (method === "qsdb/query/finished") {
      q.running = false;
      q.done = true;
      q.finishedOk = true;
      if (p.kind === "exec") {
        q.receipt = {
          command: (p.command as string) ?? "EXEC",
          message: (p.message as string) ?? null,
          rowsAffected: (p.rowsAffected as number) ?? 0,
        };
        const r = q.receipt;
        pushMessage("receipt", `${r.command}: ${state.t.rowsAffected(String(r.rowsAffected))}${r.message ? ` (${r.message})` : ""}`);
      } else {
        q.totalRows = (p.totalRows as number) ?? q.receivedRows;
        q.elapsedMs = (p.elapsedMs as number) ?? null;
        q.serverElapsedMs = (p.serverElapsedMs as number) ?? null;
      }
    } else if (method === "qsdb/query/error") {
      q.running = false;
      q.done = true;
      q.receivedRows = (p.receivedRows as number) ?? q.receivedRows;
      q.totalRows = q.totalRows || q.receivedRows;
      const code = (p.code as string) ?? "ERROR";
      const message = (p.message as string) ?? "";
      q.error = { code, message };
      if (code === "QUERY_CANCELLED") {
        q.cancelled = true;
        pushMessage("info", `${state.t.cancelled}: ${state.t.stoppedByUser}`);
      } else {
        pushMessage("error", `${code}: ${message}`);
      }
    }
  });
}

// ---------------------------------------------------------------------------
// 分页（虚拟网格按需取页 + 预取）
// ---------------------------------------------------------------------------

export function rowsFor(q: QueryState, start: number, end: number): CellValue[][] {
  const out: CellValue[][] = [];
  let i = start;
  while (i < end) {
    const pageOffset = Math.floor(i / PAGE_SIZE) * PAGE_SIZE;
    const rows = q.pages.get(pageOffset);
    if (!rows) {
      out.push([]);
      void fetchPage(q, pageOffset);
      i = pageOffset + PAGE_SIZE;
      continue;
    }
    const take = rows.slice(i - pageOffset, end - pageOffset);
    out.push(...take);
    i = pageOffset + rows.length;
  }
  return out;
}

async function fetchPage(q: QueryState, offset: number) {
  if (q.pending.has(offset) || q.pages.has(offset)) return;
  if (q.totalRows > 0 && offset >= q.totalRows && q.done) return;
  q.pending.add(offset);
  try {
    const page = await invoke<{ offset: number; rows: CellValue[][]; totalRows: number; done: boolean; hasMore: boolean }>(
      "qsdb/query/page",
      { connectionId: state.connectionId, queryId: q.queryId, offset, limit: PAGE_SIZE },
      { timeoutMs: 30000 },
    );
    q.pages.set(page.offset, page.rows);
    q.totalRows = Math.max(q.totalRows, page.totalRows);
    // 缓存上限：FIFO 淘汰最早的页（当前视图附近的页会被 rowsFor 重新拉取）
    while (q.pages.size > MAX_CACHED_PAGES) {
      const oldest = q.pages.keys().next().value;
      if (oldest === undefined) break;
      q.pages.delete(oldest);
    }
  } catch (e) {
    if (!handleConnectionLoss(e)) {
      pushMessage("error", messageOf(e));
    }
  } finally {
    q.pending.delete(offset);
  }
}

// ---------------------------------------------------------------------------
// 分区面板
// ---------------------------------------------------------------------------

export async function showPartitions(table: string) {
  state.partitionsTable = table;
  state.partitionsLoading = true;
  state.partitionsError = "";
  state.partitions = [];
  state.partitionsSize = null;
  state.activeTab = "partitions";
  try {
    const params = { connectionId: state.connectionId, table };
    const [parts, size] = await Promise.all([
      invoke<PartitionRow[]>("qsdb/partitions", params),
      invoke<{ rows: number; partitionCount: number; bytes: number | null; bytesKnown: boolean }>("qsdb/table/size", params),
    ]);
    state.partitions = parts;
    state.partitionsSize = size;
  } catch (e) {
    state.partitionsError = `${qsdbCodeOf(e)}: ${messageOf(e)}`;
  } finally {
    state.partitionsLoading = false;
  }
}

export function closePartitions() {
  state.partitionsTable = "";
  state.partitions = [];
  state.partitionsSize = null;
}

export { state, readonly };
export type { QueryState as QueryStateT };
export const pageSize = PAGE_SIZE;
