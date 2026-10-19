//! 查询结果 spool：JSONL 行文件 + 偏移索引，支撑 `query/page` 随机读。
//!
//! 布局：`DBX_PLUGIN_DATA_DIR/queries/<connection-id>/<query-id>/`
//! - `rows.jsonl`：每行一个 JSON 数组（一行 = 一条数据行的 cells）
//! - `columns.json`：started 时的列描述
//! - `meta.json`：sql 预览 / startedAt / kind
//!
//! 约束：
//! - 页默认 500 行、上限 2000 行，单页目标 ≤ 1 MiB（UI 桥 2 MiB 上限）
//! - **不静默截断**：预算耗尽停在页边界并用 hasMore 指出还有数据；单行超
//!   预算照常返回（一行拆不开），由 8 MiB Sidecar 上限兜底
//! - `query/close` 删目录；disconnect 清理整个连接目录；启动清理崩溃遗留

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use dbx_plugin_sdk::PluginError;
use serde_json::Value;

use crate::session::QueryState;

/// spool 根目录。`DBX_PLUGIN_DATA_DIR` 由 DBX 宿主注入
/// （`<data>/plugin-data/<plugin-id>`）；缺失时退回系统临时目录
/// （开发/直跑 sidecar 的场景）。
pub fn data_root() -> PathBuf {
    std::env::var_os("DBX_PLUGIN_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("queries")
}

/// 连接级 spool 目录。
pub fn connection_dir(connection_id: &str) -> PathBuf {
    data_root().join(sanitize_segment(connection_id))
}

/// 路径段消毒：id 是我们生成/宿主给的字符串，但目录名防御到底——
/// 只留 `A-Za-z0-9._-`，其余替换 `_`；连续点折叠成 `_`（防 `..` 穿越）。
fn sanitize_segment(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .take(128)
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let mut cleaned = cleaned.replace("..", "_");
    if cleaned.contains('.') && (cleaned.starts_with('.') || cleaned.ends_with('.')) {
        cleaned = cleaned.trim_matches('.').to_string();
    }
    if cleaned.is_empty() { "_".to_string() } else { cleaned }
}

/// 单查询 spool 的写入端（只被查询 worker 线程持有，无并发）。
pub struct SpoolWriter {
    writer: BufWriter<File>,
    bytes_written: u64,
}

impl SpoolWriter {
    /// 建目录并打开 `rows.jsonl`。
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let file = File::create(dir.join("rows.jsonl"))?;
        Ok(Self { writer: BufWriter::new(file), bytes_written: 0 })
    }

    /// 追加一行（JSON 数组），返回该行的起始字节偏移。
    pub fn append_row(&mut self, row: &Value) -> std::io::Result<u64> {
        let offset = self.bytes_written;
        let mut line = serde_json::to_vec(row).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.bytes_written += line.len() as u64;
        Ok(offset)
    }

    /// chunk 边界刷盘：保证并发 page 能看到已收行。
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// 写列描述与 meta（started 前调用）。
pub fn write_columns_and_meta(dir: &Path, columns: &Value, sql_preview: &str, kind: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join("columns.json"), serde_json::to_vec(columns).map_err(std::io::Error::other)?)?;
    let meta = serde_json::json!({
        "sqlPreview": sql_preview.chars().take(500).collect::<String>(),
        "kind": kind,
        "startedAtMs": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    });
    fs::write(dir.join("meta.json"), serde_json::to_vec(&meta).map_err(std::io::Error::other)?)?;
    Ok(())
}

const DEFAULT_PAGE_LIMIT: u32 = 500;
const MAX_PAGE_LIMIT: u32 = 2000;
/// 单页字节预算（目标 ≤ 1 MiB；UI 桥 2 MiB 上限的一半）。
const PAGE_BYTE_BUDGET: usize = 1024 * 1024;

/// 页请求结果。
pub struct PageData {
    pub rows: Vec<Value>,
    pub total_rows: u64,
    pub done: bool,
    pub has_more: bool,
}

/// 从 spool 读取一页。游标提供偏移索引与完成标记（worker 持续更新）。
pub fn read_page(state: &QueryState, offset: u64, limit: Option<u32>) -> Result<PageData, PluginError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT) as u64;
    let (offsets, total_rows, done) = {
        let cursor = state
            .cursor
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (cursor.offsets.clone(), cursor.total_rows, cursor.done)
    };

    // offset 越界：返回空 rows + 当前 totalRows。
    if offset >= total_rows || offset as usize >= offsets.len() {
        return Ok(PageData { rows: Vec::new(), total_rows, done, has_more: !done });
    }

    let mut reader = BufReader::new(File::open(row_file(state)).map_err(|e| {
        crate::error::qsdb_error("SPOOL_READ_ERROR", None, false, format!("failed to open spool: {e}"))
    })?);
    reader
        .seek(SeekFrom::Start(offsets[offset as usize]))
        .map_err(|e| crate::error::qsdb_error("SPOOL_READ_ERROR", None, false, format!("seek failed: {e}")))?;

    let mut rows: Vec<Value> = Vec::new();
    let mut budget = PAGE_BYTE_BUDGET;
    while rows.len() as u64 <= limit {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // 文件尾（done=false 时表示 worker 尚未写到这）
            Ok(n) => {
                let value: Value = serde_json::from_str(line.trim_end()).map_err(|e| {
                    crate::error::qsdb_error("SPOOL_CORRUPT", None, false, format!("spool line is not JSON: {e}"))
                })?;
                rows.push(value);
                if let Some(b) = budget.checked_sub(n) {
                    budget = b;
                } else {
                    break; // 预算耗尽：停在页边界（至少已读 1 行）
                }
                if rows.len() as u64 >= limit || budget == 0 {
                    break;
                }
            }
            Err(e) => {
                return Err(crate::error::qsdb_error(
                    "SPOOL_READ_ERROR",
                    None,
                    false,
                    format!("failed to read spool: {e}"),
                ))
            }
        }
    }
    let end = offset + rows.len() as u64;
    let has_more = !done || end < total_rows;
    Ok(PageData { rows, total_rows, done, has_more })
}

fn row_file(state: &QueryState) -> PathBuf {
    state.dir.join("rows.jsonl")
}

/// 删除单个查询目录（幂等）。
pub fn remove_query_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// 删除整个连接目录（disconnect）。
pub fn remove_query_root(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// 启动时清理崩溃遗留：sidecar 进程启动时不可能有活跃查询（会话随进程
/// 消失），直接清掉整个 `queries/` 根即可。std::env::temp_dir 兜底分支同样成立。
pub fn cleanup_orphaned_runs() -> std::io::Result<()> {
    let root = data_root();
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{CursorMeta, QueryState};
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    fn temp_state(tag: &str) -> (QueryState, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "qsdb-spool-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let state = QueryState {
            query_id: format!("q-{tag}"),
            connection_id: "conn-test".into(),
            sql: "SELECT 1".into(),
            kind: crate::sqlclass::StatementKind::Read,
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            stream_slot: Mutex::new(None),
            abort_handle: qsdb_client::AbortHandle::new(),
            cursor: Mutex::new(CursorMeta { offsets: Vec::new(), total_rows: 0, done: false }),
            dir: dir.clone(),
        };
        (state, dir)
    }

    #[test]
    fn write_and_page_roundtrip() {
        let (state, dir) = temp_state("roundtrip");
        let mut writer = SpoolWriter::create(&dir).unwrap();
        for i in 0..1500u64 {
            let row = serde_json::json!([i, format!("row{i}")]);
            let offset = writer.append_row(&row).unwrap();
            let mut cursor = state.cursor.lock().unwrap();
            cursor.offsets.push(offset);
            cursor.total_rows += 1;
        }
        writer.flush().unwrap();
        state.cursor.lock().unwrap().done = true; // worker 退出时会置位；测试模拟

        // 默认 500 行页
        let page = read_page(&state, 0, None).unwrap();
        assert_eq!(page.rows.len(), 500);
        assert_eq!(page.total_rows, 1500);
        assert!(page.has_more);
        assert_eq!(page.rows[0], serde_json::json!([0, "row0"]));

        // 中段页
        let page = read_page(&state, 1200, Some(200)).unwrap();
        assert_eq!(page.rows.len(), 200);
        assert_eq!(page.rows[0], serde_json::json!([1200, "row1200"]));

        // 最后一页不满
        let page = read_page(&state, 1400, Some(500)).unwrap();
        assert_eq!(page.rows.len(), 100);
        assert!(!page.has_more);

        // 越界
        let page = read_page(&state, 9999, None).unwrap();
        assert!(page.rows.is_empty());
        assert_eq!(page.total_rows, 1500);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn growing_spool_pages_while_not_done() {
        let (state, dir) = temp_state("growing");
        let mut writer = SpoolWriter::create(&dir).unwrap();
        for i in 0..10u64 {
            let offset = writer.append_row(&serde_json::json!([i])).unwrap();
            let mut cursor = state.cursor.lock().unwrap();
            cursor.offsets.push(offset);
            cursor.total_rows += 1;
        }
        writer.flush().unwrap();
        // 未完成：读到最后也有 has_more（流未结束）
        let page = read_page(&state, 0, Some(100)).unwrap();
        assert_eq!(page.rows.len(), 10);
        assert!(!page.done);
        assert!(page.has_more);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_spool_offset_out_of_range() {
        let (state, dir) = temp_state("empty");
        SpoolWriter::create(&dir).unwrap();
        let page = read_page(&state, 0, None).unwrap();
        assert!(page.rows.is_empty());
        assert_eq!(page.total_rows, 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sanitize_blocks_traversal() {
        let d = connection_dir("../../evil");
        assert!(!d.to_string_lossy().contains(".."));
        assert_eq!(sanitize_segment(""), "_");
    }
}
