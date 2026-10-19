// 工作台状态：连接、元数据、查询生命周期、分页缓存、历史。
// 单例 reactive store；组件直接 import 使用。
import { reactive, readonly } from "vue";
import { api, invoke, InvokeError, onQueryEvent, onEnvChange, type DbxColumn, type DbxContext } from "./bridge";
import { langFor, t, humanBytes, type Lang, type Messages } from "./i18n";

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
  /** 前端列排序：激活时全量拉页 + 排序快照（sortView 取代 spool 翻页）。 */
  sort: { col: number; dir: 1 | -1 } | null;
  sortView: CellValue[][] | null;
  sortPreparing: boolean;
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
  server: {
    loading: false,
    queries: [],
    sessions: [],
    sessionsError: "",
    stats: null,
    error: "",
  } as ServerPanel,
  lastFile: "", // 最近一次落盘文件（结果导出 / 脚本保存）绝对路径
  density: "cozy" as "cozy" | "compact", // 结果网格行高
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
export { pushMessage };

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

async function loadDensity() {
  try {
    const d = await api().storage.get("qsdb.density");
    if (d === "cozy" || d === "compact") state.density = d;
  } catch {
    /* 非关键偏好 */
  }
}

/** 切换结果网格密度（紧凑/舒适）并持久化。 */
export function toggleDensity() {
  state.density = state.density === "cozy" ? "compact" : "cozy";
  api()
    .storage.set("qsdb.density", state.density)
    .catch(() => undefined);
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
  void loadDensity();
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
  // 始终取全量（不带 database 过滤）：schema 树按库分组展示全部挂载；
  // currentDb 只影响查询的默认库，不裁剪树。
  const meta = await invoke<{ databases: DbInfo[]; tables: TableInfo[] }>("qsdb/metadata", {
    connectionId: state.connectionId,
  });
  state.databases = meta.databases;
  state.tables = meta.tables;
}

export async function refresh() {
  try {
    state.info = await invoke<ServerInfo>("qsdb/server/info", { connectionId: state.connectionId });
    await loadMeta();
    state.loadError = "";
  } catch (e) {
    if (!handleConnectionLoss(e)) {
      state.loadError = messageOf(e);
    }
  }
}

/** 元数据加载失败后的手动重试入口（UI 横幅按钮）。 */
export function retryLoad() {
  return bootstrapConnection();
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
    sort: null,
    sortView: null,
    sortPreparing: false,
  };
}

/** UI 层只读拦截（Sidecar 层是第二道；服务端授权是最终边界）。 */
export function isReadOnlySql(sql: string): boolean {
  return /^(select|with|show|explain|values|describe|desc)\b/i.test(sql.trim().replace(/^(--.*\n|#[^\n]*\n|\/\*[\s\S]*?\*\/)+/s, "").trimStart());
}

/** 创建查询态并让 sidecar 启动；启动失败时错误落在 q.error（done=true）。 */
async function startQuery(sqlText: string): Promise<QueryState> {
  const q = newQuery("", sqlText);
  state.query = q;
  state.activeTab = "results";
  try {
    const r = await invoke<{ queryId: string }>("qsdb/query/start", {
      connectionId: state.connectionId,
      sql: sqlText,
      database: state.currentDb || undefined,
    });
    q.queryId = r.queryId;
    void saveHistory(sqlText);
  } catch (e) {
    q.running = false;
    q.done = true;
    q.error = { code: qsdbCodeOf(e), message: messageOf(e) };
    pushMessage("error", `${q.error.code}: ${q.error.message}`);
    handleConnectionLoss(e);
  }
  return q;
}

/** 运行一段 SQL（单条或选中片段）。 */
export async function runSqlText(text: string) {
  const sqlText = text.trim();
  if (!sqlText || state.query?.running) return;
  if (state.info?.readOnly && !isReadOnlySql(sqlText)) {
    pushMessage("error", state.t.readOnlyBlocked);
    return;
  }
  state.lastFile = "";
  await startQuery(sqlText);
}

/** 整篇编辑器内容。 */
export async function runSql() {
  await runSqlText(state.sql);
}

function waitDone(q: QueryState): Promise<void> {
  return new Promise((resolve) => {
    const iv = setInterval(() => {
      if (q.done) {
        clearInterval(iv);
        resolve();
      }
    }, 50);
  });
}

/** 多语句顺序执行：逐条启动，上一条完成后才发下一条；失败即停。
 * 每条的结果/回执照常进结果区与消息面板，这里只追加进度摘要。 */
export async function runStatements(sqls: string[]) {
  if (state.query?.running) return;
  for (let i = 0; i < sqls.length; i += 1) {
    const text = sqls[i];
    if (state.info?.readOnly && !isReadOnlySql(text)) {
      pushMessage("error", `${state.t.stmtFailed(i + 1, sqls.length)}: ${state.t.readOnlyBlocked}`);
      return;
    }
    state.lastFile = "";
    const q = await startQuery(text);
    if (!q.queryId) return; // 启动失败，错误已入消息面板
    await waitDone(q);
    if (q.error && !q.cancelled) {
      pushMessage("error", state.t.stmtStopped(i + 1, sqls.length));
      return;
    }
    const detail = q.receipt
      ? `${q.receipt.command} · ${state.t.rowsAffected(String(q.receipt.rowsAffected))}`
      : `${q.totalRows} ${state.lang === "zh" ? "行" : "rows"}`;
    pushMessage("receipt", `▸ ${state.t.stmtDone(i + 1, sqls.length)} · ${detail}`);
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
// 前端列排序 / 导出
// ---------------------------------------------------------------------------

/** 前端排序只对可全量拉取的结果集开放（更大的结果请用 SQL ORDER BY）。 */
export const SORT_ROW_LIMIT = 20_000;

function compareCells(a: CellValue, b: CellValue): number {
  // null 恒排最后（与方向无关）；number 数值比，其余按字符串比
  if (a === null || a === undefined) return 1;
  if (b === null || b === undefined) return -1;
  if (typeof a === "number" && typeof b === "number") return a - b;
  const sa = typeof a === "object" ? JSON.stringify(a) : String(a);
  const sb = typeof b === "object" ? JSON.stringify(b) : String(b);
  return sa < sb ? -1 : sa > sb ? 1 : 0;
}

/** 表头点击：asc → desc → off 循环。仅在结果已收完时可排序（快照才完整）。 */
export async function toggleSort(col: number) {
  const q = state.query;
  if (!q || q.receipt || q.sortPreparing) return;
  if (q.sort && q.sort.col === col && q.sort.dir === -1) {
    q.sort = null;
    q.sortView = null;
    return;
  }
  if (!q.done) {
    pushMessage("info", state.t.sortNeedsDone);
    return;
  }
  if (q.totalRows > SORT_ROW_LIMIT) {
    pushMessage("info", `${state.t.sortTooLarge}（${q.totalRows} > ${SORT_ROW_LIMIT}）`);
    return;
  }
  const dir: 1 | -1 = q.sort?.col === col ? -1 : 1;
  q.sortPreparing = true;
  try {
    // 全量拉页（已缓存的跳过；顺序 await 避免瞬时打满桥）
    for (let off = 0; off < q.totalRows; off += PAGE_SIZE) {
      if (!q.pages.has(off)) await fetchPage(q, off);
    }
    const all: CellValue[][] = [];
    for (let off = 0; off < q.totalRows; off += PAGE_SIZE) {
      const page = q.pages.get(off);
      if (page) all.push(...page);
    }
    all.sort((r1, r2) => compareCells(r1[col] ?? null, r2[col] ?? null) * dir);
    q.sort = { col, dir };
    q.sortView = all;
  } finally {
    q.sortPreparing = false;
  }
}

/** 退出排序快照（状态栏排序徽章的 ✕）。 */
export function clearSort() {
  const q = state.query;
  if (!q) return;
  q.sort = null;
  q.sortView = null;
}

export interface ExportResult {
  path: string;
  rows: number;
  bytes: number;
  finished: boolean;
}

/** 导出结果到 `<DATA>/exports/`（sidecar 流式落盘）；路径进消息面板。 */
export async function exportQuery(format: "csv" | "tsv"): Promise<ExportResult | null> {
  const q = state.query;
  if (!q?.queryId || q.receipt) return null;
  const r = await invoke<ExportResult>("qsdb/query/export", {
    connectionId: state.connectionId,
    queryId: q.queryId,
    format,
  });
  const size = humanBytes(r.bytes, state.lang);
  const unit = state.lang === "zh" ? "行" : "rows";
  pushMessage(
    "receipt",
    r.finished ? `${state.t.exportDone}: ${r.path} · ${r.rows} ${unit} · ${size}` : `${state.t.exportPartial}: ${r.path} · ${r.rows} ${unit}`,
  );
  state.lastFile = r.path;
  return r;
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

// ---------------------------------------------------------------------------
// 服务器概览（侧栏"服务器"页）
// ---------------------------------------------------------------------------

export interface RunningQuery {
  id: string;
  sql: string;
  elapsedMs: number;
  memoryBytes: number;
  cancelled: boolean;
  sessionId: string | null;
}

export interface AdminSessionRow {
  id: string;
  user: string;
  role: string | null;
  client: string;
  createdAtMs: number;
  lastActiveMs: number;
  idleMs: number;
  defaultDb: string;
  runningQueries: string[];
  queriesTotal: number;
  rowsWritten: number;
}

export interface StatsInfo {
  tables: number;
  storeBytes: number;
  writeBuffer: { table: string; batches: number; bytes: number }[];
}

export interface ServerPanel {
  loading: boolean;
  queries: RunningQuery[];
  sessions: AdminSessionRow[];
  /** 会话是 admin 专属端点；非 admin 拿到 403 时给出提示而不是报错横幅 */
  sessionsError: string;
  stats: StatsInfo | null;
  error: string;
}

export async function loadServerPanel() {
  const p = state.server;
  p.loading = true;
  p.error = "";
  p.sessionsError = "";
  const conn = { connectionId: state.connectionId };
  const [queries, stats, sessions] = await Promise.allSettled([
    invoke<RunningQuery[]>("qsdb/queries/list", conn),
    invoke<StatsInfo>("qsdb/server/stats", conn),
    invoke<AdminSessionRow[]>("qsdb/sessions/list", conn),
  ]);
  p.queries = queries.status === "fulfilled" ? queries.value : p.queries;
  p.stats = stats.status === "fulfilled" ? stats.value : p.stats;
  if (sessions.status === "fulfilled") {
    p.sessions = sessions.value;
  } else {
    p.sessions = [];
    p.sessionsError = sessions.reason instanceof Error ? sessions.reason.message : String(sessions.reason);
  }
  if (queries.status === "rejected") p.error = messageOf(queries.reason);
  if (stats.status === "rejected") p.error = p.error || messageOf(stats.reason);
  p.loading = false;
}

export async function cancelServerQuery(queryId: string) {
  await invoke("qsdb/queries/cancel", { connectionId: state.connectionId, queryId });
  pushMessage("info", `${state.t.cancelled}: ${queryId}`);
  await loadServerPanel();
}

export async function kickSession(sessionId: string) {
  await invoke("qsdb/sessions/kick", { connectionId: state.connectionId, sessionId });
  pushMessage("info", `${state.t.sessionKicked}: ${sessionId}`);
  await loadServerPanel();
}

export { state, readonly };
export type { QueryState as QueryStateT };
export const pageSize = PAGE_SIZE;
