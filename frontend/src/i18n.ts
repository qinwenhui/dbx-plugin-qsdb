// 中英文案。语言跟随宿主（dbxPlugin.locale；'zh' 开头 → 中文，其余英文）。

export type Lang = "en" | "zh";

const messages = {
  en: {
    appName: "QSDB",
    connecting: "Connecting…",
    noConnection: "No open connection. Open this workbench from a saved QSDB connection.",
    connectionGone: "Connection is closed. Reopen the QSDB connection to continue.",
    disconnected: "Disconnected",
    refresh: "Refresh",
    filterPlaceholder: "Filter tables",
    noTables: "No tables",
    run: "Run",
    stop: "Stop",
    executing: "Running…",
    cancelling: "Cancelling…",
    cancelled: "Cancelled",
    history: "History",
    noHistory: "No history yet",
    readOnlyBadge: "read-only",
    tabResults: "Results",
    tabPartitions: "Partitions",
    tabMessages: "Messages",
    messagesEmpty: "No messages yet",
    rowsLoaded: (loaded: string, total: string) => `Loaded ${loaded} of ${total} rows`,
    rowsReceiving: (loaded: string) => `${loaded} rows · receiving…`,
    rowsEmpty: "0 rows",
    elapsed: "Elapsed",
    serverElapsed: "Server",
    page: "Page",
    copyTsv: "Copy TSV",
    copyCsv: "Copy CSV",
    copied: "Copied",
    nullCell: "NULL",
    rowsAffected: (n: string) => `${n} row(s) affected`,
    errorTitle: "Error",
    stoppedByUser: "Query stopped — rows already received stay browsable",
    selectTable: "Select a table to inspect partitions and size",
    partitionsFor: (t: string) => `Partitions of ${t}`,
    partitionName: "Partition",
    generation: "Generation",
    rows: "Rows",
    segments: "Segments",
    bytes: "Bytes",
    bytesUnknown: "unknown",
    tableSize: "Table size",
    partitionsCount: (n: string) => `${n} partitions`,
    preview: "Preview",
    viewPartitions: "Partitions",
    copyName: "Copy name",
    loading: "Loading…",
    loadFailed: (msg: string) => `Load failed: ${msg}`,
    editorPlaceholder: "-- SQL here. Ctrl/Cmd+Enter to run",
    readOnlyBlocked: "Read-only connection: only SELECT / SHOW / EXPLAIN / VALUES statements can run.",
    bytesHuman: (s: string) => s,
    partitions: "Partitions",
  },
  zh: {
    appName: "QSDB",
    connecting: "连接中…",
    noConnection: "尚未打开连接。请从已保存的 QSDB 连接打开此工作台。",
    connectionGone: "连接已断开。请重新打开 QSDB 连接后继续。",
    disconnected: "已断开",
    refresh: "刷新",
    filterPlaceholder: "过滤表名",
    noTables: "暂无表",
    run: "运行",
    stop: "停止",
    executing: "执行中…",
    cancelling: "正在停止…",
    cancelled: "已取消",
    history: "历史",
    noHistory: "暂无历史",
    readOnlyBadge: "只读",
    tabResults: "结果",
    tabPartitions: "分区",
    tabMessages: "消息",
    messagesEmpty: "暂无消息",
    rowsLoaded: (loaded: string, total: string) => `已加载 ${loaded} / 共 ${total} 行`,
    rowsReceiving: (loaded: string) => `${loaded} 行 · 接收中…`,
    rowsEmpty: "0 行",
    elapsed: "耗时",
    serverElapsed: "服务端",
    page: "页",
    copyTsv: "复制 TSV",
    copyCsv: "复制 CSV",
    copied: "已复制",
    nullCell: "NULL",
    rowsAffected: (n: string) => `影响 ${n} 行`,
    errorTitle: "错误",
    stoppedByUser: "查询已停止——已收到的行仍可浏览",
    selectTable: "选择一张表查看分区与大小",
    partitionsFor: (t: string) => `${t} 的分区`,
    partitionName: "分区",
    generation: "generation",
    rows: "行数",
    segments: "段数",
    bytes: "字节",
    bytesUnknown: "未知",
    tableSize: "表大小",
    partitionsCount: (n: string) => `${n} 个分区`,
    preview: "预览",
    viewPartitions: "分区",
    copyName: "复制表名",
    loading: "加载中…",
    loadFailed: (msg: string) => `加载失败：${msg}`,
    editorPlaceholder: "-- 在这里输入 SQL，Ctrl/Cmd+Enter 运行",
    readOnlyBlocked: "只读连接：仅允许 SELECT / SHOW / EXPLAIN / VALUES 语句。",
    bytesHuman: (s: string) => s,
    partitions: "分区",
  },
}

export type Messages = (typeof messages)["en"];

export function langFor(locale: string | undefined): Lang {
  return (locale ?? "en").toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function t(locale: string | undefined): Messages {
  return messages[langFor(locale)];
}

/** 人类可读字节数；null/undefined → 未知标注（不伪造 0 B）。 */
export function humanBytes(n: number | null | undefined, lang: Lang): string {
  if (n === null || n === undefined) {
    return lang === "zh" ? "未知" : "unknown";
  }
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return `${u === 0 ? v : v.toFixed(1)} ${units[u]}`;
}
