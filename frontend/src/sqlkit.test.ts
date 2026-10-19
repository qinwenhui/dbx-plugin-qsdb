// sqlkit 单测：语句拆分（引号/注释/括号感知）与轻量格式化的稳定性。
import { describe, expect, it } from "vitest";
import {
  formatSql,
  formatStatement,
  generateDdl,
  isBlankSql,
  partitionPairs,
  partitionPreviewSql,
  partitionWhere,
  splitStatements,
  tokenize,
} from "./sqlkit";

describe("splitStatements", () => {
  it("splits on top-level semicolons", () => {
    expect(splitStatements("SELECT 1; SELECT 2;")).toEqual(["SELECT 1", "SELECT 2"]);
  });

  it("ignores semicolons in strings", () => {
    expect(splitStatements("SELECT 'a;b' AS x; SELECT 2")).toEqual(["SELECT 'a;b' AS x", "SELECT 2"]);
  });

  it("ignores semicolons in comments and quoted identifiers", () => {
    const sql = `-- note; with semicolon
SELECT "col;name" FROM t /* inline ; */ ; SELECT 2`;
    const parts = splitStatements(sql);
    expect(parts.length).toBe(2);
    expect(parts[0]).toContain('"col;name"');
  });

  it("ignores semicolons inside parens", () => {
    expect(splitStatements("SELECT f(a; b) FROM t")).toEqual(["SELECT f(a; b) FROM t"]);
  });

  it("drops comment-only fragments and empties", () => {
    expect(splitStatements(";; -- just a note ; ;")).toEqual([]);
    expect(splitStatements("")).toEqual([]);
  });

  it("keeps escaped quotes intact", () => {
    expect(splitStatements("SELECT 'it''s; fine'")).toEqual(["SELECT 'it''s; fine'"]);
  });
});

describe("tokenize", () => {
  it("keeps strings and comments as single tokens", () => {
    const toks = tokenize("SELECT 'a b' -- c d\nFROM t");
    const kinds = toks.map((t) => t.kind);
    expect(kinds).toContain("string");
    expect(kinds).toContain("comment");
    expect(toks.find((t) => t.kind === "string")?.text).toBe("'a b'");
  });
});

describe("formatStatement", () => {
  it("uppercases keywords and breaks major clauses", () => {
    const out = formatStatement("select a, b from t where x = 1 and y = 2 order by a desc limit 10");
    expect(out).toBe(
      ["SELECT a,", "  b", "FROM t", "WHERE x = 1", "  AND y = 2", "ORDER BY a DESC", "LIMIT 10"].join("\n"),
    );
  });

  it("does not break inside parens", () => {
    const out = formatStatement("select f(a, b) from t where x in (1, 2, 3)");
    expect(out).toBe("SELECT f(a, b)\nFROM t\nWHERE x IN (1, 2, 3)");
  });

  it("leaves string contents untouched", () => {
    const out = formatStatement("select 'select from where' as s from t");
    expect(out).toBe("SELECT 'select from where' AS s\nFROM t");
  });

  it("formats joins on their own lines", () => {
    const out = formatStatement("select * from a left join b on a.id = b.id where a.x = 1");
    expect(out).toBe("SELECT *\nFROM a\nLEFT JOIN b ON a.id = b.id\nWHERE a.x = 1");
  });
});

describe("formatSql", () => {
  it("formats each statement and joins with semicolons", () => {
    const out = formatSql("select 1;select 2;");
    expect(out).toBe("SELECT 1;\n\nSELECT 2");
  });

  it("returns blank sql untouched-ish", () => {
    expect(isBlankSql("  -- only comment\n")).toBe(true);
    expect(isBlankSql("select 1")).toBe(false);
  });
});

describe("partitionPairs / partitionWhere", () => {
  it("parses col=value names", () => {
    expect(partitionPairs("dt=2026-01-01")).toEqual([{ col: "dt", value: "2026-01-01" }]);
  });

  it("parses multi-column names separated by / or ,", () => {
    expect(partitionPairs("dt=2026-01-01/region=east")).toEqual([
      { col: "dt", value: "2026-01-01" },
      { col: "region", value: "east" },
    ]);
    expect(partitionPairs("dt=2026-01-01,region=east")).toEqual([
      { col: "dt", value: "2026-01-01" },
      { col: "region", value: "east" },
    ]);
  });

  it("treats bare names as value-only", () => {
    expect(partitionPairs("2026-01-01")).toEqual([{ col: "", value: "2026-01-01" }]);
  });

  it("builds WHERE with escaped quotes; null for no-column names", () => {
    expect(partitionWhere("dt=2026-01-01")).toBe("dt = '2026-01-01'");
    expect(partitionWhere("d=it's")).toBe("d = 'it''s'");
    expect(partitionWhere("2026-01-01")).toBeNull();
  });

  it("preview sql composes table + where + limit", () => {
    expect(partitionPreviewSql("default.sales", "dt=2026-01-01")).toBe(
      "SELECT * FROM default.sales WHERE dt = '2026-01-01' LIMIT 1000",
    );
  });
});

describe("generateDdl", () => {
  it("renders columns with NOT NULL and partition annotation as comment", () => {
    const out = generateDdl(
      "default.sales",
      [
        { name: "id", typeName: "Int64", nullable: false, partition: false },
        { name: "dt", typeName: "Date32", nullable: false, partition: true },
      ],
      "partition key",
    );
    expect(out).toBe(
      ["CREATE TABLE default.sales (", "  id INT64 NOT NULL,", "  -- ◀ partition key", "  dt DATE32 NOT NULL", ");"].join("\n"),
    );
  });

  it("uppercases type names", () => {
    const out = generateDdl("t", [{ name: "s", typeName: "Utf8", nullable: true, partition: false }], "pk");
    expect(out).toContain("s UTF8");
  });
});
