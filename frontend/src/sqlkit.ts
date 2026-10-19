// SQL 工具：语句拆分与轻量格式化。零依赖（单文件包不放运行时依赖）。
// 拆分规则：只按**顶层**（引号/注释/括号之外）的分号切；格式化是保守的
// "关键字大写 + 主句换行 + 顶层逗号缩进"，不追求完整 SQL 语义。

export interface Token {
  kind: "word" | "string" | "comment" | "ws" | "other";
  text: string;
}

/** 词法切分：字符串/注释/括号感知。word = [A-Za-z0-9_$#]+。 */
export function tokenize(sql: string): Token[] {
  const out: Token[] = [];
  let i = 0;
  const n = sql.length;
  const push = (kind: Token["kind"], text: string) => {
    if (!text) return;
    const last = out[out.length - 1];
    if (last && last.kind === kind && (kind === "ws" || kind === "word")) last.text += text;
    else out.push({ kind, text });
  };
  while (i < n) {
    const c = sql[i];
    // 行注释
    if (c === "-" && sql[i + 1] === "-") {
      const end = sql.indexOf("\n", i);
      const stop = end === -1 ? n : end;
      push("comment", sql.slice(i, stop));
      i = stop;
      continue;
    }
    // 块注释
    if (c === "/" && sql[i + 1] === "*") {
      const end = sql.indexOf("*/", i + 2);
      const stop = end === -1 ? n : end + 2;
      push("comment", sql.slice(i, stop));
      i = stop;
      continue;
    }
    // 字符串：'…''…' 与双引号标识符 "…""…"
    if (c === "'" || c === '"') {
      let j = i + 1;
      while (j < n) {
        if (sql[j] === c) {
          if (sql[j + 1] === c) {
            j += 2; // 转义（双写）
            continue;
          }
          j += 1;
          break;
        }
        j += 1;
      }
      push("string", sql.slice(i, j));
      i = j;
      continue;
    }
    if (/\s/.test(c)) {
      let j = i;
      while (j < n && /\s/.test(sql[j])) j += 1;
      push("ws", sql.slice(i, j));
      i = j;
      continue;
    }
    if (/[A-Za-z0-9_$#]/.test(c)) {
      let j = i;
      while (j < n && /[A-Za-z0-9_$#]/.test(sql[j])) j += 1;
      push("word", sql.slice(i, j));
      i = j;
      continue;
    }
    push("other", c);
    i += 1;
  }
  return out;
}

/** 按顶层分号拆语句（去空白、去纯注释片段；语句文本原样保留）。 */
export function splitStatements(sql: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  let i = 0;
  const n = sql.length;
  while (i < n) {
    const c = sql[i];
    if (c === "'" || c === '"') {
      i += 1;
      while (i < n) {
        if (sql[i] === c) {
          if (sql[i + 1] === c) {
            i += 2;
            continue;
          }
          i += 1;
          break;
        }
        i += 1;
      }
      continue;
    }
    if (c === "-" && sql[i + 1] === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? n : end;
      continue;
    }
    if (c === "/" && sql[i + 1] === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? n : end + 2;
      continue;
    }
    if (c === "(") depth += 1;
    else if (c === ")") depth = Math.max(0, depth - 1);
    else if (c === ";" && depth === 0) {
      out.push(sql.slice(start, i));
      start = i + 1;
    }
    i += 1;
  }
  out.push(sql.slice(start));
  return out.map((s) => s.trim()).filter((s) => {
    // 纯注释/空白的片段不算语句
    const stripped = s
      .replace(/--[^\n]*/g, "")
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .trim();
    return stripped.length > 0;
  });
}

// ---------------------------------------------------------------------------
// 轻量格式化
// ---------------------------------------------------------------------------

/** 主句关键字：前面换行（顶层）。 */
const MAJOR = new Set([
  "select", "from", "where", "group", "having", "order", "limit", "offset",
  "union", "except", "intersect", "values", "set",
  "insert", "update", "delete", "create", "drop", "alter", "with", "truncate",
]);
/** JOIN 家族：前面换行。 */
const JOINS = new Set(["join", "inner", "left", "right", "full", "cross"]);
/** 需要大写的关键字（保守集合，函数名不处理）。 */
const KEYWORDS = new Set([
  "select", "distinct", "from", "where", "group", "by", "having", "order", "limit", "offset",
  "as", "on", "and", "or", "not", "in", "is", "null", "like", "between", "exists",
  "case", "when", "then", "else", "end", "asc", "desc",
  "join", "inner", "left", "right", "full", "outer", "cross", "union", "all", "any", "some",
  "insert", "into", "values", "update", "set", "delete",
  "create", "table", "drop", "alter", "add", "column", "view", "index",
  "primary", "key", "foreign", "references", "constraint", "default", "unique",
  "with", "cast", "partition", "over", "using", "truncate", "if", "returns",
]);

const isKw = (tok: Token) => tok.kind === "word" && KEYWORDS.has(tok.text.toLowerCase());
const lower = (tok: Token) => tok.text.toLowerCase();

/** 单条语句格式化。行为保守：只动空白与大小写，不改任何字面量。
 * 层级约定：主句关键字 0 缩进，顶层逗号/AND/OR 续行 2 缩进；括号内不折行。 */
export function formatStatement(sql: string): string {
  const tokens = tokenize(sql).filter((t) => t.kind !== "ws");
  if (tokens.length === 0) return sql.trim();
  let out = "";
  let depth = 0;
  let atLineStart = true;
  let tightNext = false; // `(`/`.` 之后第一个 token 贴紧，不加空格

  const newline = (spaces: number) => {
    out = out.replace(/\s+$/, "");
    out += `\n${" ".repeat(spaces)}`;
    atLineStart = true;
  };
  const stripTail = () => {
    out = out.replace(/\s+$/, "");
  };
  const space = () => {
    if (tightNext) {
      tightNext = false;
      return;
    }
    if (out && !atLineStart && !/\s$/.test(out)) out += " ";
  };

  for (let i = 0; i < tokens.length; i += 1) {
    const tok = tokens[i];
    if (tok.kind === "comment") {
      if (!atLineStart) newline(0);
      out += tok.text.trim();
      newline(0);
      continue;
    }
    if (tok.kind === "other") {
      if (tok.text === "(") {
        // 函数调用（前一个词不是关键字）贴紧；关键字/右括号后留空格
        const prev = tokens[i - 1];
        const fnCall = prev && prev.kind === "word" && !KEYWORDS.has(lower(prev));
        if (fnCall) stripTail();
        else space();
        out += "(";
        depth += 1;
        tightNext = true;
        continue;
      }
      if (tok.text === ")") {
        depth = Math.max(0, depth - 1);
        stripTail();
        out += ")";
        continue;
      }
      if (tok.text === ".") {
        stripTail(); // 限定名 a.b 两侧贴紧
        out += ".";
        tightNext = true;
        continue;
      }
      if (tok.text === ",") {
        if (depth === 0) {
          stripTail();
          out += ",";
          newline(2);
        } else {
          stripTail();
          out += ", ";
        }
        continue;
      }
      space();
      out += tok.text;
      continue;
    }
    // word / string
    if (tok.kind === "word") {
      const lw = lower(tok);
      // 主句/JOIN：顶层换行（group/order/partition 的 by 不再换；
      // join 前的 left/right/full/inner/cross 同理——它们自己已经换过行了）
      if (depth === 0 && (MAJOR.has(lw) || JOINS.has(lw))) {
        const prev = tokens[i - 1];
        const prevW = prev && prev.kind === "word" ? lower(prev) : "";
        const prevIsMod = ["group", "order", "partition"].includes(prevW) || (lw === "join" && JOINS.has(prevW));
        if (!prevIsMod && !atLineStart) newline(0);
      }
      // 顶层 AND/OR 换行缩进
      if (depth === 0 && (lw === "and" || lw === "or") && !atLineStart) newline(2);
      space();
      out += isKw(tok) ? tok.text.toUpperCase() : tok.text;
      atLineStart = false;
      continue;
    }
    space();
    out += tok.text;
    atLineStart = false;
  }
  return out.trim();
}

/** 格式化整段（多语句时逐条处理，`;` 连接；末条不带分号）。 */
export function formatSql(sql: string): string {
  const parts = splitStatements(sql);
  if (parts.length === 0) return sql;
  return parts.map((p) => formatStatement(p)).join(";\n\n");
}

/** 去注释后是否空白（无实际语句）。 */
export function isBlankSql(sql: string): boolean {
  return sql.replace(/--[^\n]*/g, "").replace(/\/\*[\s\S]*?\*\//g, "").trim().length === 0;
}

// ---------------------------------------------------------------------------
// 分区谓词 / DDL 生成
// ---------------------------------------------------------------------------

export interface PartitionPair {
  col: string;
  value: string;
}

/** 分区名（目录名形态，如 `dt=2026-01-01` 或多列 `dt=2026-01-01/region=east`）
 * → k/v 对。无 `=` 的名字整体作为 value（col 为空）。 */
export function partitionPairs(name: string): PartitionPair[] {
  return name
    .split(/[/,]/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => {
      const i = s.indexOf("=");
      return i === -1 ? { col: "", value: s } : { col: s.slice(0, i).trim(), value: s.slice(i + 1).trim() };
    });
}

/** 分区名 → WHERE 谓词文本。值统一按字符串字面量引用（' 按标准翻倍转义）；
 * 无列名的名字不产生条件（调用方回退整表查询）。 */
export function partitionWhere(name: string): string | null {
  const pairs = partitionPairs(name).filter((p) => p.col);
  if (pairs.length === 0) return null;
  return pairs.map((p) => `${p.col} = '${p.value.replaceAll("'", "''")}'`).join(" AND ");
}

/** 分区预览 SQL：`SELECT * FROM <table> WHERE <谓词> LIMIT n`。 */
export function partitionPreviewSql(table: string, partitionName: string, limit = 1000): string {
  const where = partitionWhere(partitionName);
  return `SELECT * FROM ${table}${where ? ` WHERE ${where}` : ""} LIMIT ${limit}`;
}

export interface DdlColumn {
  name: string;
  typeName: string;
  nullable: boolean;
  partition: boolean;
}

/** 由服务端 schema 生成参考 DDL。QSDB 的 DDL 语法细节（分区子句等）未在
 * 接口契约中定义，因此分区列用**独立注释行**标注而非猜测语法——生成物
 * 明确标记为参考，执行前需按服务端语法调整。 */
export function generateDdl(name: string, columns: DdlColumn[], note: string): string {
  const body = columns
    .map((c) => {
      const line = `  ${c.name} ${c.typeName.toUpperCase()}${c.nullable ? "" : " NOT NULL"}`;
      return c.partition ? `  -- ◀ ${note}\n${line}` : line;
    })
    .join(",\n");
  return `CREATE TABLE ${name} (\n${body}\n);`;
}
