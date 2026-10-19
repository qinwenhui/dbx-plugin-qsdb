//! 测试辅助：`qsdb-rows` 线上格式的**最小编码器**（mock QSDB 服务用）。
//!
//! 布局对照 `qsdb-client` 的读侧解码器实现——只实现测试需要的
//! 类型子集（Utf8 / I64 + NULL）。

/// 列类型标签（与 wire 编号一致）。
#[derive(Clone, Copy)]
pub enum Tag {
    I64 = 3,
    Utf8 = 5,
}

/// 一个 cell（mock 只产这两类 + NULL）。
pub enum Cell {
    Null,
    Int(i64),
    Str(&'static str),
}

fn u16le(v: u16) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn u32le(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

/// 流头：`QSDBROWS` + version 1 + flags=1（带 schema）+ 列描述。
pub fn encode_header(cols: &[(String, Tag)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"QSDBROWS");
    out.extend_from_slice(&u16le(1));
    out.push(1); // flags: schema 随头
    out.extend_from_slice(&u16le(cols.len() as u16));
    for (name, tag) in cols {
        out.extend_from_slice(&u16le(name.len() as u16));
        out.extend_from_slice(name.as_bytes());
        out.push(*tag as u8);
        out.push(1); // col_flags: nullable（mock 全部可空）
        match tag {
            Tag::Utf8 => {
                // meta: max_chars u32（0 = 无上限）
                out.extend_from_slice(&u16le(4));
                out.extend_from_slice(&u32le(0));
            }
            _ => out.extend_from_slice(&u16le(0)),
        }
    }
    out
}

/// 一个数据块：row_count u32 + chunk_flags（有 NULL 时 bit0）+ 列位图 +
/// 行主序值区（NULL 也占位）。
pub fn encode_chunk(cols: &[(String, Tag)], rows: &[Vec<Cell>]) -> Vec<u8> {
    let n_cols = cols.len();
    let n_rows = rows.len();
    let has_null = rows.iter().any(|r| r.iter().any(|c| matches!(c, Cell::Null)));
    let mut out = Vec::new();
    out.extend_from_slice(&u32le(n_rows as u32));
    out.push(if has_null { 1 } else { 0 });
    if has_null {
        let nb = n_rows.div_ceil(8);
        for col in 0..n_cols {
            let mut bitmap = vec![0u8; nb];
            for (r, row) in rows.iter().enumerate() {
                if matches!(row[col], Cell::Null) {
                    bitmap[r / 8] |= 1 << (r % 8);
                }
            }
            out.extend_from_slice(&bitmap);
        }
    }
    for row in rows {
        for (col, cell) in row.iter().enumerate() {
            match (cols[col].1, cell) {
                (Tag::I64, Cell::Int(v)) => out.extend_from_slice(&v.to_le_bytes()),
                (Tag::I64, _) => out.extend_from_slice(&0i64.to_le_bytes()), // NULL 占位
                (Tag::Utf8, Cell::Str(s)) => {
                    out.extend_from_slice(&u32le(s.len() as u32));
                    out.extend_from_slice(s.as_bytes());
                }
                (Tag::Utf8, _) => out.extend_from_slice(&u32le(0)), // NULL 占位
            }
        }
    }
    out
}
