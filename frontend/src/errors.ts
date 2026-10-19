// 错误码说明：悬停提示用。只收录插件自身与 QSDB 常见错误码；
// 未收录的码返回 null（UI 不显示提示，原样透出）。
import type { Lang } from "./i18n";

interface Hint {
  zh: string;
  en: string;
}

const HINTS: Record<string, Hint> = {
  // ---- 插件侧 ----
  AUTH_REQUIRED: {
    zh: "该服务器要求认证。请在连接配置里填写 Basic（用户名/密码）或 Bearer Token，匿名无法访问。",
    en: "This server requires authentication. Configure Basic (username/password) or a Bearer token instead of anonymous.",
  },
  CONNECTION_NOT_FOUND: {
    zh: "连接已断开或不存在（可能连接被关闭、sidecar 重启）。请重新打开 QSDB 连接。",
    en: "The connection is closed or unknown (it may have been closed, or the sidecar restarted). Reopen the QSDB connection.",
  },
  READ_ONLY_CONNECTION: {
    zh: "只读连接只允许 SELECT / SHOW / EXPLAIN / VALUES / DESCRIBE 语句。",
    en: "Read-only connections only allow SELECT / SHOW / EXPLAIN / VALUES / DESCRIBE statements.",
  },
  QUERY_CANCELLED: {
    zh: "查询已被停止。两段式停止：先断开传输，再通知服务端取消；已接收的行仍可浏览。",
    en: "The query was stopped. Two-phase stop: the transfer is aborted first, then the server is asked to cancel. Rows already received stay browsable.",
  },
  QUERY_NOT_FOUND: {
    zh: "查询不存在（已关闭/被清理，或不属于当前连接）。",
    en: "Unknown query (already closed/cleaned up, or it belongs to another connection).",
  },
  NO_CURSOR: {
    zh: "该语句不返回行游标（DDL/DML 只有回执）。",
    en: "This statement has no row cursor (DDL/DML only returns a receipt).",
  },
  NETWORK_ERROR: {
    zh: "网络错误：服务器不可达、连接被重置或超时。检查地址/端口与服务器状态后重试。",
    en: "Network error: the server is unreachable, the connection was reset, or it timed out. Check host/port and server health, then retry.",
  },
  DECODE_ERROR: {
    zh: "结果解码失败：服务端返回的数据格式与插件预期不符（可能是内核版本差异）。",
    en: "Decode failed: the server returned data in an unexpected shape (possibly a kernel version difference).",
  },
  SPOOL_WRITE_ERROR: {
    zh: "本地结果缓存写入失败（磁盘满/权限/目录被清理）。",
    en: "Failed to write the local result spool (disk full / permissions / directory removed).",
  },
  SPOOL_READ_ERROR: {
    zh: "本地结果缓存读取失败（文件被移动或清理）。请重新运行查询。",
    en: "Failed to read the local result spool (file moved or cleaned up). Re-run the query.",
  },
  SPOOL_CORRUPT: {
    zh: "本地结果缓存损坏。请重新运行查询。",
    en: "The local result spool is corrupt. Re-run the query.",
  },
  EXPORT_WRITE_ERROR: {
    zh: "导出文件写入失败（磁盘满/权限）。导出位置在插件数据目录的 exports/ 下。",
    en: "Failed to write the export file (disk full / permissions). Exports live under the plugin data directory's exports/ folder.",
  },
  SCRIPT_WRITE_ERROR: {
    zh: "脚本保存失败（磁盘满/权限）。脚本保存在插件数据目录的 scripts/ 下。",
    en: "Failed to save the script (disk full / permissions). Scripts live under the plugin data directory's scripts/ folder.",
  },
  // ---- 服务端常见 ----
  UNAUTHORIZED: {
    zh: "认证失败（401）：用户名/密码或 Token 不正确。",
    en: "Authentication failed (401): wrong username/password or token.",
  },
  FORBIDDEN: {
    zh: "权限不足（403）：当前用户没有执行该操作的授权。会话列表等运维接口需要 admin。",
    en: "Forbidden (403): the current user lacks the grant. Admin endpoints (e.g. sessions) require admin.",
  },
  TABLE_NOT_FOUND: {
    zh: "表不存在：检查库名/表名限定（多挂载时用 db.table 形式）。",
    en: "Table not found: check the database/table qualification (use db.table when multiple catalogs are mounted).",
  },
  PARSE_ERROR: {
    zh: "SQL 解析失败：语法不符合 QSDB 方言。",
    en: "SQL parse error: the syntax does not match the QSDB dialect.",
  },
  QUERY_TIMEOUT: {
    zh: "查询超时。可加大超时参数或优化查询（缩小扫描范围/加分区谓词）。",
    en: "Query timed out. Increase the timeout or narrow the scan (partition predicates help).",
  },
  SESSION_NOT_FOUND: {
    zh: "服务端会话不存在或已过期，请重新连接。",
    en: "The server session is gone or expired. Reconnect.",
  },
};

/** 错误码 → 本地化提示；未收录返回 null。 */
export function errorHint(code: string, lang: Lang): string | null {
  const h = HINTS[code];
  if (!h) return null;
  return lang === "zh" ? h.zh : h.en;
}

/** 从错误文本（`CODE: message` 形态）里提取错误码；没有则 null。 */
export function leadingCode(text: string): string | null {
  const m = /^\s*([A-Z][A-Z0-9_]{2,})\s*:/.exec(text);
  return m ? m[1] : null;
}
