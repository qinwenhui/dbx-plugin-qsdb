//! QSDB DBX 插件 Sidecar。
//!
//! DBX 宿主通过 stdio-jsonl 的 JSON-RPC 调用本进程；stdout 只输出协议消息，
//! 日志一律走 stderr（[`log!`] 宏）。所有 QSDB 网络 I/O 在这里完成，
//! Workbench UI 无直接网络访问。
//!
//! 方法面：
//! - 固定生命周期：`connection/test|connect|disconnect`（幂等）
//! - 元数据：`qsdb/server/info`、`qsdb/metadata`、`qsdb/partitions`、`qsdb/table/size`
//! - 查询：`qsdb/query/start|page|stop|close` + `qsdb/query/*` 事件
//!
//! 两条硬规则：
//! 1. Secret（密码/Token）只出现在 lifecycle 请求里，绝不回显、记录或转发；
//! 2. 每连接两个 `qsdb_client::Client`——流式查询独占 query client 期间，
//!    control client 仍然可做 metadata/cancel/health。

mod admin;
mod error;
mod lifecycle;
mod metadata;
mod params;
mod query;
mod session;
mod sqlclass;
mod spool;
mod value;

use std::sync::Arc;

use dbx_plugin_sdk::{
    PluginEmitter, PluginError, PluginHandler, PluginMetadata, PluginServer, RequestContext,
};
use serde_json::Value;

use session::Registry;

/// stderr 日志。**禁止**在 stdout 打任何非协议内容——一行日志就能毁掉整个
/// JSONL 协议流。
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("[qsdb-sidecar] {}", format_args!($($arg)*))
    };
}

struct QsdbPlugin {
    registry: Arc<Registry>,
}

impl PluginHandler for QsdbPlugin {
    fn handle(
        &self,
        _context: RequestContext,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, PluginError> {
        // 只记方法名与连接 id，不记 params——params 里可能带 SQL，甚至密钥。
        log!("method={} connectionId={:?}", method, params.get("connectionId").and_then(Value::as_str));
        match method {
            // ---- 固定生命周期（DBX 直接调用） ----
            "connection/test" => lifecycle::test(&self.registry, &params),
            "connection/connect" => lifecycle::connect(&self.registry, &params),
            "connection/disconnect" => lifecycle::disconnect(&self.registry, &params),
            // ---- 元数据 ----
            "qsdb/server/info" => metadata::server_info(&self.registry, &params),
            "qsdb/metadata" => metadata::metadata(&self.registry, &params),
            "qsdb/partitions" => metadata::partitions(&self.registry, &params),
            "qsdb/table/size" => metadata::table_size(&self.registry, &params),
            // ---- 查询 ----
            "qsdb/query/start" => query::start(&self.registry, &params, emitter),
            "qsdb/query/page" => query::page(&self.registry, &params),
            "qsdb/query/stop" => query::stop(&self.registry, &params),
            "qsdb/query/close" => query::close(&self.registry, &params),
            "qsdb/query/export" => query::export(&self.registry, &params),
            "qsdb/script/save" => spool::save_script(&params),
            // ---- 服务器运维（运行查询/会话/统计） ----
            "qsdb/server/stats" => admin::stats(&self.registry, &params),
            "qsdb/queries/list" => admin::queries_list(&self.registry, &params),
            "qsdb/queries/cancel" => admin::queries_cancel(&self.registry, &params),
            "qsdb/sessions/list" => admin::sessions_list(&self.registry, &params),
            "qsdb/sessions/kick" => admin::sessions_kick(&self.registry, &params),
            _ => Err(PluginError::method_not_found(method)),
        }
    }
}

/// stop 里的 cancel 失败是预期的（传输段服务端已注销回 404；查询已结束同理）
/// ——只上 stderr，不影响 stop 结果。
pub fn log_cancel_ignored(query_id: &str, err: &qsdb_client::Error) {
    log!("cancel({query_id}) ignored: {err}");
}

fn main() {
    // 启动时清理上次崩溃遗留的查询 spool 目录（disconnect 已清理过；
    // 这里兜底进程被杀的情形）。
    if let Err(e) = spool::cleanup_orphaned_runs() {
        log!("startup spool cleanup failed: {e}");
    }

    // id/version 必须与 manifest.json 完全一致（DBX 在 plugin/initialize 校验）。
    // Cargo version 与 manifest version 同步：改版本时两边一起改。
    let metadata = PluginMetadata::new("cn.qinwh.qsdb", env!("CARGO_PKG_VERSION"))
        .with_capability("connections")
        .with_capability("events");

    let server = PluginServer::new(metadata, QsdbPlugin { registry: Arc::new(Registry::new()) })
        // 查询在专用线程跑，不占 worker 池；这里给足并发让元数据/分页请求
        // 在长查询进行中仍能被及时处理。
        .worker_threads(8);
    if let Err(e) = server.serve() {
        log!("sidecar server exited: {e}");
        std::process::exit(1);
    }
}
