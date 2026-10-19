//! 元数据：`qsdb/server/info` / `qsdb/metadata` / `qsdb/partitions` / `qsdb/table/size`。
//!
//! 全部走 control client（流式查询占住 query client 时仍然可用）。
//! 表清单用 `tables_with_schema` **一次取全列集**，禁止逐表 N+1。
//! 多挂载：服务端把表名给成 `db.table` 限定形态；单挂载给裸名——
//! `database` 字段在这里归一化，按 `database` 过滤。

use std::sync::Arc;

use dbx_plugin_sdk::PluginError;
use serde_json::{json, Value};

use crate::error;
use crate::session::{Registry, Session};

fn invalid(message: &str) -> PluginError {
    error::invalid_param(message)
}

/// `qsdb/server/info`。
pub fn server_info(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let connection_id = params
        .get("connectionId")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing connectionId"))?;
    let session = registry.get(connection_id)?;
    Ok(session.server_info())
}

/// `qsdb/metadata`：databases + tables(+columns)，一次取全。
pub fn metadata(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let connection_id = params
        .get("connectionId")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing connectionId"))?;
    let database = params.get("database").and_then(Value::as_str).filter(|s| !s.is_empty());
    let session = registry.get(connection_id)?;

    let dbs = session.control_client.databases().map_err(|e| error::from_qsdb(&e))?;
    let databases: Vec<Value> = dbs
        .iter()
        .map(|d| json!({ "name": d.name, "readOnly": d.read_only }))
        .collect();

    // 表清单：服务端一次返回全部挂载的表（多挂载给 `db.table`，单挂载给裸名）。
    let tables = session
        .control_client
        .tables_with_schema()
        .map_err(|e| error::from_qsdb(&e))?;

    // 归属判定：名字以 `<db>.` 开头且 db 在挂载清单里 → 归该库；否则归
    // 请求的库（过滤场景）或唯一挂载库/默认库（展示场景）。
    let known_dbs: Vec<&str> = dbs.iter().map(|d| d.name.as_str()).collect();
    let fallback_db = |session: &Session| -> String {
        match (database, known_dbs.len()) {
            (Some(db), _) => db.to_string(),
            (None, 1) => known_dbs[0].to_string(),
            (None, _) => session.default_db.clone().unwrap_or_else(|| "default".to_string()),
        }
    };

    let mut out_tables: Vec<Value> = Vec::new();
    for t in &tables {
        let (table_db, bare_name): (String, &str) = match t.name.split_once('.') {
            Some((prefix, rest)) if known_dbs.contains(&prefix) => (prefix.to_string(), rest),
            _ => (fallback_db(&session), t.name.as_str()),
        };
        if let Some(want) = database {
            if &table_db != want {
                continue;
            }
        }
        let columns: Vec<Value> = t
            .columns
            .iter()
            .map(|c| json!({ "name": c.name, "typeName": c.type_name, "nullable": c.nullable, "partition": c.partition }))
            .collect();
        out_tables.push(json!({
            // 展示名保留限定形态（多挂载下可直接当表名查询）；database 单独给。
            "name": t.name,
            "database": table_db,
            "bareName": bare_name,
            "columns": columns,
        }));
    }

    Ok(json!({
        "databases": databases,
        "tables": out_tables,
    }))
}

/// 表名解析：`db.table` 直接用；裸名在多挂载下无法定位——按请求里的
/// `database`（或会话默认库）拼限定名，交给服务端判错。
fn resolve_table(session: &Session, table: &str, database: Option<&str>) -> String {
    if table.contains('.') {
        table.to_string()
    } else {
        match database.or(session.default_db.as_deref()) {
            Some(db) => format!("{db}.{table}"),
            None => table.to_string(),
        }
    }
}

/// `qsdb/partitions`：generation/rows/segments/bytes。
/// 旧内核没有 bytes 时服务端回缺省 0——按"未知"处理（bytesKnown=false），
/// **不伪造 0 B**。
pub fn partitions(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let connection_id = params
        .get("connectionId")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing connectionId"))?;
    let table = params
        .get("table")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("missing table"))?;
    let database = params.get("database").and_then(Value::as_str);
    let session = registry.get(connection_id)?;

    let qualified = resolve_table(&session, table, database);
    let parts = session
        .control_client
        .partitions(&qualified)
        .map_err(|e| error::from_qsdb(&e))?;

    // bytesKnown：全部 >0 才算"已知"；任一为 0（旧内核缺字段缺省）按未知。
    let bytes_known = !parts.is_empty() && parts.iter().all(|p| p.bytes > 0);
    let out: Vec<Value> = parts
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "generation": p.generation,
                "rows": p.rows,
                "segments": p.segments,
                "bytes": if bytes_known { Value::from(p.bytes) } else { Value::Null },
                "bytesKnown": bytes_known,
            })
        })
        .collect();
    Ok(Value::Array(out))
}

/// `qsdb/table/size`：分区求和的表大小。
pub fn table_size(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let connection_id = params
        .get("connectionId")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing connectionId"))?;
    let table = params
        .get("table")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("missing table"))?;
    let database = params.get("database").and_then(Value::as_str);
    let session: Arc<Session> = registry.get(connection_id)?;

    let qualified = resolve_table(&session, table, database);
    let parts = session
        .control_client
        .partitions(&qualified)
        .map_err(|e| error::from_qsdb(&e))?;

    let rows: u64 = parts.iter().map(|p| p.rows).sum();
    let bytes_known = !parts.is_empty() && parts.iter().all(|p| p.bytes > 0);
    let bytes: u64 = parts.iter().map(|p| p.bytes).sum();
    Ok(json!({
        "table": qualified,
        "partitionCount": parts.len(),
        "rows": rows,
        "bytes": if bytes_known { Value::from(bytes) } else { Value::Null },
        "bytesKnown": bytes_known,
    }))
}
