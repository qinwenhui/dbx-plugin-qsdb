//! SQL 只读分类——**宁可保守**。
//!
//! 只有明确只读的语句走流式通道（`query_stream`，进 spool 分页）；
//! 其余一律按"执行回执"处理（`execute`，返回 command/rowsAffected）。
//! 把写语句误当读查询会静默丢弃影响行数，这是不能发生的错误。
//!
//! 这里不做 SQL 解析（不引 sqlparser）：一个剥注释/剥字符串的扫描器 +
//! 首关键字判断足够分类；分类错误的兜底是服务端语义（流式通道对
//! DDL/DML 会显式报"需要结果集"），不会静默错。

/// 语句分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    /// 只读：SELECT / WITH…SELECT / SHOW / EXPLAIN / VALUES / (DESC)RIBE。
    /// 走流式 + spool + 分页。
    Read,
    /// 其余（DDL/DML/USE/COPY…）：走执行回执，不进 spool。
    Exec,
}

/// 分类一条 SQL。多语句（非空段 >1）一律按 Exec 处理——流式通道不承接
/// 语句串，避免"首句只读、尾句写"被流式路径吞掉。
pub fn classify(sql: &str) -> StatementKind {
    let statements = split_statements(sql);
    let mut non_empty = statements.iter().filter(|s| !s.is_empty());
    let Some(first) = non_empty.next() else {
        return StatementKind::Exec; // 空语句让服务端报错，这里不猜
    };
    if non_empty.next().is_some() {
        return StatementKind::Exec;
    }
    match leading_keyword(first) {
        Some(kw) => match kw.to_ascii_uppercase().as_str() {
            "SELECT" | "SHOW" | "EXPLAIN" | "VALUES" | "DESCRIBE" | "DESC" => StatementKind::Read,
            "WITH" => {
                // WITH 只在读查询场景当作读；QSDB 的 CTE 只支持 SELECT。
                // 保守：WITH 后第一个词也必须是 SELECT 才算读。
                body_after_keyword(first, "WITH")
                    .map(rest_after_with)
                    .map(|rest| leading_keyword(rest).map(|k| k.eq_ignore_ascii_case("SELECT")).unwrap_or(false))
                    .unwrap_or(false)
                    .then_some(StatementKind::Read)
                    .unwrap_or(StatementKind::Exec)
            }
            _ => StatementKind::Exec,
        },
        None => StatementKind::Exec,
    }
}

/// WITH 之后跳过 `name AS ( … ) [, name AS ( … ) …]` 的 CTE 头，返回主体。
/// 判据：**最后一个顶层右括号**之后的内容——CTE 列表结束处。字符串里的
/// 括号已被上游剥掉，配平可靠；没有任何顶层括号时返回原文兜底。
fn rest_after_with(rest: &str) -> &str {
    let bytes = rest.as_bytes();
    let mut depth = 0i64;
    let mut after_last_top_paren = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    after_last_top_paren = i + 1;
                }
            }
            _ => {}
        }
    }
    if after_last_top_paren == 0 {
        rest
    } else {
        rest[after_last_top_paren..].trim_start()
    }
}

/// 按分号切语句（引号/注释外的分号才算）。
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = sql.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    while let Some(c) = chars.next() {
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                cur.push(c);
            }
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if in_single {
            cur.push(c);
            if c == '\'' {
                // '' 转义：吃掉第二个引号
                if chars.peek() == Some(&'\'') {
                    cur.push(chars.next().unwrap());
                } else {
                    in_single = false;
                }
            }
            continue;
        }
        if in_double {
            cur.push(c);
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push(chars.next().unwrap());
                } else {
                    in_double = false;
                }
            }
            continue;
        }
        match c {
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                in_line_comment = true;
            }
            '#' => in_line_comment = true,
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_block_comment = true;
            }
            '\'' => {
                in_single = true;
                cur.push(c);
            }
            '"' => {
                in_double = true;
                cur.push(c);
            }
            ';' => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// 剥掉前导空白与括号后的首个关键字（大写保留原文大小写由调用方处理）。
fn leading_keyword(statement: &str) -> Option<&str> {
    let mut rest = statement.trim_start();
    while rest.starts_with('(') {
        rest = rest[1..].trim_start();
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let kw = &rest[..end];
    if kw.is_empty() { None } else { Some(kw) }
}

fn body_after_keyword<'a>(statement: &'a str, kw: &str) -> Option<&'a str> {
    let start = statement.trim_start();
    let idx = start.find(kw)?; // 调用方已确认首词就是 kw
    Some(start[idx + kw.len()..].trim_start())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_read(sql: &str) -> bool {
        classify(sql) == StatementKind::Read
    }

    #[test]
    fn simple_reads() {
        for sql in [
            "SELECT 1",
            "  select * from t",
            "(SELECT a FROM t)",
            "((select 1))",
            "SHOW TABLES",
            "show databases",
            "EXPLAIN SELECT 1",
            "VALUES (1,2)",
            "DESCRIBE t",
            "DESC t",
        ] {
            assert!(is_read(sql), "{sql}");
        }
    }

    #[test]
    fn cte_read() {
        assert!(is_read("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(is_read("WITH a AS (SELECT 1), b AS (SELECT 2) SELECT * FROM b"));
    }

    #[test]
    fn writes_are_exec() {
        for sql in [
            "CREATE TABLE t (a BIGINT)",
            "INSERT INTO t VALUES (1)",
            "DROP TABLE t",
            "ALTER TABLE t ADD COLUMN b INT",
            "UPDATE t SET a = 1", // 服务端决定是否支持，分类保守
            "DELETE FROM t",
            "COPY t FROM 'x'",
            "USE settle",
            "SET x = 1",
        ] {
            assert!(!is_read(sql), "{sql}");
        }
    }

    #[test]
    fn write_in_string_literals_ignored() {
        assert!(is_read("SELECT 'DROP TABLE t' AS s"));
        assert!(is_read("SELECT \"insert\" FROM t"));
        assert!(!is_read("INSERT INTO t(a) VALUES ('; SELECT 1')"));
    }

    #[test]
    fn comments_stripped() {
        assert!(is_read("-- drop it\nSELECT 1"));
        assert!(is_read("/* insert */ SELECT 1"));
        assert!(!is_read("SELECT 1 -- trailing\n; DROP TABLE t"));
    }

    #[test]
    fn multi_statement_is_exec() {
        assert!(!is_read("SELECT 1; SELECT 2"));
        assert!(!is_read("SELECT 1; DROP TABLE t"));
    }

    #[test]
    fn empty_is_exec() {
        assert!(!is_read(""));
        assert!(!is_read("   "));
        assert!(!is_read("-- only a comment"));
    }

    #[test]
    fn with_non_select_is_exec() {
        assert!(!is_read("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x"));
    }
}
