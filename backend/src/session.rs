//! 会话注册表：以 `connection.id` 为 key（Sidecar 是**插件级共享进程**，
//! 多个 Tab 的 UI 调用都落到同一张表上）。
//!
//! 每连接两个 `qsdb_client::Client`：
//! - `query_client`：流式读查询独占（一条 keep-alive 连接被流占住时，
//!   另一条仍可工作）；
//! - `control_client`：metadata / cancel / health / 写语句执行。
//!
//! `Client` 是 `Send + Sync`（内部锁保护连接），`Arc` 共享即可。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dbx_plugin_sdk::PluginError;
use serde_json::Value;

use crate::spool;

/// 一条在跑/已完成查询的共享状态（`query/start` 注册，UI 经 page/stop/close 访问）。
pub struct QueryState {
    pub query_id: String,
    pub connection_id: String,
    pub sql: String,
    pub kind: crate::sqlclass::StatementKind,
    /// 停止标记：stop 与 worker 都读写，决定结束事件是 finished / cancelled。
    pub stop: AtomicBool,
    /// worker 已退出（成功/失败/取消都算）——close/disconnect 等它之后才能
    /// 安全删 spool 目录（Windows 上打开中的文件删不掉）。
    pub finished: AtomicBool,
    /// 流式查询的**停车位**：worker 在两次取块之间把 `RowStream` 放回这里，
    /// 让 stop 能立刻 drop（= abort，关 socket）。worker 正在取块时为 None，
    /// 那个窗口由"块边界轮询 stop 标记"兜底。
    pub stream_slot: Mutex<Option<qsdb_client::RowStream>>,
    /// bulk/exec 查询的传输段中止句柄（`query_with_abort` 用）。
    pub abort_handle: qsdb_client::AbortHandle,
    /// spool 游标元数据（行偏移索引 + 总行数 + 完成标记）。
    pub cursor: Mutex<CursorMeta>,
    /// spool 目录：`<data>/queries/<connection>/<queryId>/`。
    pub dir: PathBuf,
}

/// spool 游标：`offsets[i]` = 第 i 行在 rows.jsonl 里的字节偏移。
pub struct CursorMeta {
    pub offsets: Vec<u64>,
    pub total_rows: u64,
    pub done: bool,
}

impl QueryState {
    pub fn received_rows(&self) -> u64 {
        self.cursor.lock().map(|c| c.total_rows).unwrap_or(0)
    }
}

/// 一个已连接的 QSDB 会话。
pub struct Session {
    pub connection_id: String,
    pub query_client: Arc<qsdb_client::Client>,
    pub control_client: Arc<qsdb_client::Client>,
    pub version: String,
    /// `None` = 匿名/无会话。
    pub user: Option<String>,
    pub role: Option<String>,
    pub default_db: Option<String>,
    pub read_only: bool,
    /// 本连接的查询表；disconnect 时整体清空。
    pub queries: Mutex<HashMap<String, Arc<QueryState>>>,
    /// spool 根：`<DATA>/queries/<connection-id>/`。
    pub spool_dir: PathBuf,
}

impl Session {
    /// 会话是否存在且至少还能用（version 在 connect 时拿到）。
    pub fn server_info(&self) -> Value {
        serde_json::json!({
            "version": self.version,
            "user": self.user,
            "role": self.role,
            "defaultDb": self.default_db.clone().unwrap_or_else(|| "default".to_string()),
            "readOnly": self.read_only,
        })
    }

    /// 注册一条新查询（start 幂等性不要求，但 queryId 冲突要防——重挂载）。
    pub fn register_query(
        self: &Arc<Self>,
        query_id: String,
        sql: String,
        kind: crate::sqlclass::StatementKind,
    ) -> Result<Arc<QueryState>, PluginError> {
        let dir = self.spool_dir.join(&query_id);
        let mut queries = self.queries.lock().map_err(|_| poisoned("query registry"))?;
        if queries.contains_key(&query_id) {
            return Err(crate::error::internal("duplicate query id"));
        }
        let state = Arc::new(QueryState {
            query_id,
            connection_id: self.connection_id.clone(),
            sql,
            kind,
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            stream_slot: Mutex::new(None),
            abort_handle: qsdb_client::AbortHandle::new(),
            cursor: Mutex::new(CursorMeta { offsets: Vec::new(), total_rows: 0, done: false }),
            dir,
        });
        queries.insert(state.query_id.clone(), state.clone());
        Ok(state)
    }

    /// 停止本连接全部在跑查询（disconnect 用）。
    pub fn stop_all_queries(&self) {
        let queries = self.queries.lock().unwrap_or_else(|e| e.into_inner());
        for state in queries.values() {
            state.stop.store(true, Ordering::SeqCst);
            // bulk/exec：立刻关 socket（解除 worker 的阻塞读）；流式：drop
            // 停车位上的流（= abort）。worker 正在取块的窗口由块边界检查兜底。
            state.abort_handle.abort();
            if let Ok(mut slot) = state.stream_slot.lock() {
                slot.take(); // drop = abort
            }
        }
    }
}

fn poisoned(what: &str) -> PluginError {
    crate::error::internal(format!("{what} lock is poisoned"))
}

/// 清理线程等待 worker 退出的上限（stop 已发，正常毫秒级；兜底防泄漏）。
const CLEANUP_WAIT: Duration = Duration::from_secs(300);

/// 等待 worker 退出（轮询 `finished`；到时即返）。
pub fn wait_finished(state: &QueryState, limit: Duration) {
    let deadline = std::time::Instant::now() + limit;
    while !state.finished.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 全局会话表。
#[derive(Default)]
pub struct Registry {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, connection_id: &str) -> Result<Arc<Session>, PluginError> {
        self.sessions
            .read()
            .map_err(|_| poisoned("session registry"))?
            .get(connection_id)
            .cloned()
            .ok_or_else(|| {
                // 自定义方法对未知连接的统一错误：所有方法都校验
                // connectionId 对应会话仍存在。
                crate::error::qsdb_error("CONNECTION_NOT_FOUND", None, false, "connection is not open; connect first")
            })
    }

    pub fn insert(&self, session: Session) {
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session.connection_id.clone(), Arc::new(session));
    }

    pub fn contains(&self, connection_id: &str) -> bool {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(connection_id)
    }

    /// 移除会话并清理其全部 spool 目录（幂等：不存在也返回成功）。
    pub fn remove(&self, connection_id: &str) {
        let session = self
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(connection_id);
        if let Some(session) = session {
            session.stop_all_queries();
            // worker 可能还握着 spool 文件句柄（Windows 删不掉打开中的文件）：
            // 等各查询退出后删，限时兜底。
            let queries: Vec<Arc<QueryState>> = session
                .queries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .cloned()
                .collect();
            let root = session.spool_dir.clone();
            let _ = std::thread::Builder::new().name("qsdb-spool-cleanup".into()).spawn(move || {
                for state in queries {
                    wait_finished(&state, CLEANUP_WAIT);
                }
                spool::remove_query_root(&root);
            });
        }
    }

}

/// 生成查询 id：`q-` + 时间戳 + 进程 id + 计数器（hex）。只要求进程内唯一
/// + 可作 URL 路径段（`[A-Za-z0-9._:-]`）；不引 uuid 依赖。
pub fn new_query_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("q-{ts:012x}{}{n:04x}", std::process::id())
}
