//! 连接右键动态菜单（manifest: `context-menu` + `dynamic: true`）。
//!
//! 宿主在菜单打开时调 `contextMenu/resolve/<贡献id>`（**500ms 超时**），
//! 因此这里只读 Registry 的本地缓存态（版本/用户/角色/查询计数），绝不发
//! 网络请求。返回 `{ items }`，动作为 `open-workbench`（宿主负责打开工作台
//! 并附上连接上下文）。`invoke` 型动作会回打 `contextMenu/<贡献id>` ——
//! 当前未用（插件侧主动断开会让宿主连接状态失真，不做）。

use serde_json::{json, Value};

use crate::session::Registry;

const WORKBENCH_ID: &str = "cn.qinwh.qsdb.main";

fn zh(locale: &str) -> bool {
    locale.eq_ignore_ascii_case("zh-cn") || locale.to_ascii_lowercase().starts_with("zh")
}

/// 连接 id：宿主传 `connection.id`（我们的 Registry key）。
fn connection_id_of(params: &Value) -> Option<&str> {
    params
        .get("connection")
        .and_then(|c| c.get("id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// `contextMenu/resolve/<id>`：返回菜单项。会话缺失（未连接/已断开）时只给
/// 工作台入口——打开工作台会走宿主的重连生命周期。
pub fn resolve(registry: &Registry, params: &Value) -> Result<Value, dbx_plugin_sdk::PluginError> {
    let is_zh = params.get("locale").and_then(Value::as_str).map(zh).unwrap_or(false);
    let Some(connection_id) = connection_id_of(params) else {
        return Ok(json!({ "items": [] }));
    };

    let session = match registry.get(connection_id) {
        Ok(s) => s,
        Err(_) => {
            // 会话不在：菜单仍可用（open-workbench 会触发宿主重连）
            return Ok(json!({ "items": [workbench_item(is_zh)] }));
        }
    };

    let open_label = if is_zh { "服务器概览 · 会话 · 运行查询" } else { "Server overview · sessions · queries" };
    let tree_label = if is_zh { "数据库与 SQL 工作台" } else { "Databases & SQL workbench" };

    // 身份行（禁用项，纯信息）：版本 · 用户@库 · 角色（·只读）
    let user = session.user.clone().unwrap_or_else(|| if is_zh { "匿名".into() } else { "anonymous".into() });
    let mut identity = format!(
        "QSDB {} · {}@{}",
        session.version,
        user,
        session.default_db.clone().unwrap_or_else(|| "default".into())
    );
    if let Some(role) = &session.role {
        identity.push_str(&format!(" · {role}"));
    }
    if session.read_only {
        identity.push_str(if is_zh { " · 只读" } else { " · read-only" });
    }

    // 本地运行中的查询数（零开销：Registry 计数，不打服务端）
    let running = {
        let queries = session.queries.lock().unwrap_or_else(|e| e.into_inner());
        queries.values().filter(|q| !q.finished.load(std::sync::atomic::Ordering::SeqCst)).count()
    };
    let queries_label = if running > 0 {
        if is_zh {
            format!("运行中的查询：{running}")
        } else {
            format!("Running queries: {running}")
        }
    } else if is_zh {
        "无运行中的查询".to_string()
    } else {
        "No running queries".to_string()
    };

    let open = |label: String, enabled: bool| {
        json!({
            "label": label,
            "enabled": enabled,
            "action": { "type": "open-workbench", "workbench": WORKBENCH_ID },
        })
    };
    Ok(json!({
        "items": [
            { "label": identity, "enabled": false },
            open(queries_label, running > 0),
            open(open_label.to_string(), true),
            open(tree_label.to_string(), true),
        ]
    }))
}

fn workbench_item(is_zh: bool) -> Value {
    json!({
        "label": if is_zh { "打开 QSDB 工作台" } else { "Open QSDB workbench" },
        "action": { "type": "open-workbench", "workbench": WORKBENCH_ID },
    })
}

/// `contextMenu/<id>`：静态/`invoke` 型动作处理器。当前未声明 invoke 动作；
/// 保留入口让宿主调用不落 method_not_found（返回空消息即可）。
pub fn action(registry: &Registry, params: &Value) -> Result<Value, dbx_plugin_sdk::PluginError> {
    let _ = (registry, params);
    Ok(json!({ "message": "" }))
}
