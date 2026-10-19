//! 服务器运维面：运行中的查询、会话、存储统计（`qsdb/queries/*`、
//! `qsdb/sessions/*`、`qsdb/server/stats`）。
//!
//! 对应 qsdb-client 的诊断/管理端点：
//! - `GET /api/v1/queries`（运行中查询；`cancel` 按**服务端 query id**）
//! - `GET /api/v1/sessions` + kick（admin 专属；非 admin 403 原样透传）
//! - `GET /api/v1/stats`
//!
//! 全部走 control client，只读端点与写动作分离明确。

use std::sync::Arc;

use dbx_plugin_sdk::PluginError;
use serde_json::{json, Value};

use crate::error;
use crate::session::{Registry, Session};

fn params_of(registry: &Registry, params: &Value) -> Result<(Arc<Session>, String), PluginError> {
    let connection_id = params
        .get("connectionId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error::invalid_param("missing connectionId"))?;
    let session = registry.get(connection_id)?;
    Ok((session, connection_id.to_string()))
}

/// `qsdb/server/stats`：表数、落盘字节、写缓冲。
pub fn stats(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let (session, _) = params_of(registry, params)?;
    let s = session.control_client.stats().map_err(|e| error::from_qsdb(&e))?;
    Ok(json!({
        "tables": s.tables,
        "storeBytes": s.store_bytes,
        "writeBuffer": s.write_buffer.iter().map(|(t, batches, bytes)| json!({
            "table": t, "batches": batches, "bytes": bytes,
        })).collect::<Vec<_>>(),
    }))
}

/// `qsdb/queries/list`：服务端当前运行中的查询。
pub fn queries_list(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let (session, _) = params_of(registry, params)?;
    let list = session.control_client.queries_list().map_err(|e| error::from_qsdb(&e))?;
    Ok(Value::Array(
        list.iter()
            .map(|q| {
                json!({
                    "id": q.id,
                    "sql": q.sql,
                    "elapsedMs": q.elapsed_ms,
                    "memoryBytes": q.memory_bytes,
                    "cancelled": q.cancelled,
                    "sessionId": q.session_id,
                })
            })
            .collect(),
    ))
}

/// `qsdb/queries/cancel`：取消服务端查询（按**服务端回显的 query id**）。
pub fn queries_cancel(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let (session, _) = params_of(registry, params)?;
    let id = params
        .get("queryId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error::invalid_param("missing queryId"))?;
    session.control_client.cancel(id).map_err(|e| error::from_qsdb(&e))?;
    Ok(json!({ "cancelled": true, "queryId": id }))
}

/// `qsdb/sessions/list`：会话清单（admin 专属；403 原样透出，UI 提示）。
pub fn sessions_list(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let (session, _) = params_of(registry, params)?;
    let list = session.control_client.sessions_list().map_err(|e| error::from_qsdb(&e))?;
    Ok(Value::Array(
        list.iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "user": s.user,
                    "role": s.role,
                    "client": s.client,
                    "createdAtMs": s.created_at_ms,
                    "lastActiveMs": s.last_active_ms,
                    "idleMs": s.idle_ms,
                    "defaultDb": s.default_db,
                    "runningQueries": s.running_queries,
                    "queriesTotal": s.queries_total,
                    "rowsWritten": s.rows_written,
                })
            })
            .collect(),
    ))
}

/// `qsdb/sessions/kick`：踢掉一个会话（admin）。
pub fn sessions_kick(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let (session, _) = params_of(registry, params)?;
    let id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error::invalid_param("missing sessionId"))?;
    let (session_id, killed_queries) = session
        .control_client
        .session_kick(id)
        .map_err(|e| error::from_qsdb(&e))?;
    Ok(json!({ "kicked": true, "sessionId": session_id, "killedQueries": killed_queries }))
}
