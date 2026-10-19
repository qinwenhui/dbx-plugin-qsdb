//! 连接生命周期：`connection/test` / `connection/connect` / `connection/disconnect`。
//!
//! 端点永远用 lifecycle 的 `runtime.host` / `runtime.port`（DBX 隧道/代理后的
//! 最终逻辑端点），不用连接表单里的原始地址。
//!
//! 认证（连接表单 `auth`）：
//! - `none`：`Auth::None`；`/health` 免鉴权即证可达；
//! - `basic`：`login` 换会话（成功后客户端自动带 Bearer，可 `use_database`、
//!   撤销后自动重登）；失败原样上抛 401/403；
//! - `bearer`：静态 token；`/health` 免鉴权，**必须**再打一个受保护接口
//!   （`/databases`）验证 Token，不能只看 health 就报成功。
//!
//! `connect`/`disconnect` 都幂等：已连接直接成功；不存在也返回成功。

use std::sync::Arc;
use std::time::Duration;

use dbx_plugin_sdk::PluginError;
use qsdb_client::{Auth, Client, ConnectOptions};
use serde_json::{json, Value};

use crate::error;
use crate::params::{client_url, parse, AuthKind, LifecycleParams};
use crate::session::{Registry, Session};

/// 临时/正式 client 的公共构造。
fn build_client(p: &LifecycleParams, auth: Auth) -> Result<Client, PluginError> {
    Client::with_options(
        &client_url(p),
        auth,
        ConnectOptions {
            connect_timeout: Some(Duration::from_secs(p.connect_timeout_secs)),
            // 不设 io_timeout：执行中的查询在响应头到来前 socket 是静默的，
            // 客户端读超时会把慢查询掐死（qsdb-client 文档的"好心办坏事"）。
            io_timeout: None,
        },
    )
    .map_err(|e| error::internal(format!("invalid endpoint: {e}")))
}

fn auth_for(p: &LifecycleParams) -> Auth {
    match p.auth {
        AuthKind::None => Auth::None,
        AuthKind::Basic => Auth::Basic {
            user: p.username.clone(),
            password: p.password.clone(),
        },
        AuthKind::Bearer => Auth::Bearer(p.token.clone().unwrap_or_default()),
    }
}

/// `connection/test`：临时 client 上做 health + 认证校验 + 默认库验证。
pub fn test(_registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let p = parse(params)?;
    let client = build_client(&p, auth_for(&p))?;

    // 1. 可达性 + 版本（免鉴权）
    let version = client.health().map_err(|e| {
        error::qsdb_error(
            "CONNECTION_FAILED",
            None,
            true,
            format!("cannot reach QSDB at {}:{}: {}", p.runtime_host, p.runtime_port, e),
        )
    })?;

    // 2. 认证校验
    let (user, role, default_db) = match p.auth {
        AuthKind::None => {
            // 匿名：health 即可。给了库名时验证可见性（401 会说明服务端要求登录）。
            if let Some(db) = &p.database {
                let dbs = client.databases().map_err(|e| {
                    error::qsdb_error(
                        "AUTH_REQUIRED",
                        None,
                        false,
                        format!("anonymous access cannot verify database '{db}': {e}"),
                    )
                })?;
                if !dbs.iter().any(|d| &d.name == db) {
                    return Err(error::qsdb_error(
                        "DATABASE_NOT_FOUND",
                        Some(404),
                        false,
                        format!("database '{db}' is not visible to this connection"),
                    ));
                }
            }
            (None, None, p.database.clone())
        }
        AuthKind::Basic => {
            // 凭证错 = 显式错（401 UNAUTHORIZED 原样透传），绝不静默降级匿名
            let info = client.login(&p.username, &p.password).map_err(|e| from_auth_error(e, "login failed"))?;
            // 默认库：给了就验证/设置（use_database 是会话属性）
            let mut default_db = info.default_db.clone();
            if let Some(db) = &p.database {
                client
                    .use_database(db)
                    .map_err(|e| from_auth_error(e, format!("cannot select default database '{db}'")))?;
                default_db = db.clone();
            }
            (Some(info.user), info.role, Some(default_db))
        }
        AuthKind::Bearer => {
            let token = p.token.as_deref().unwrap_or_default();
            if token.is_empty() {
                return Err(error::invalid_param("bearer auth requires a token"));
            }
            // 受保护接口验证：401 = Token 无效，403 = 有效但无授权
            let dbs = client.databases().map_err(|e| from_auth_error(e, "token validation failed"))?;
            if let Some(db) = &p.database {
                if !dbs.iter().any(|d| &d.name == db) {
                    return Err(error::qsdb_error(
                        "DATABASE_NOT_FOUND",
                        Some(404),
                        false,
                        format!("database '{db}' is not visible to this token"),
                    ));
                }
            }
            (None, None, p.database.clone())
        }
    };

    let mut message = format!("Connected to QSDB {version}");
    if let Some(u) = &user {
        message.push_str(&format!(" as {u}"));
    }
    Ok(json!({
        "success": true,
        "message": message,
        "version": version,
        "user": user,
        "role": role,
        "defaultDb": default_db,
    }))
}

/// 认证相关错误：保留 401/403 区分，文案不含凭证。
fn from_auth_error(e: qsdb_client::Error, what: impl AsRef<str>) -> PluginError {
    let mut mapped = error::from_qsdb(&e);
    if !mapped.message.is_empty() {
        mapped.message = format!("{}: {}", what.as_ref(), mapped.message);
    }
    mapped
}

/// `connection/connect`：建 query/control 两个 client，登录 + 默认库，
/// 按 `connection.id` 注册会话。已连接 → 直接成功（幂等）。
pub fn connect(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let p = parse(params)?;

    // 幂等：同 id 已连接 → 不重复建（避免占用双倍连接）。
    if registry.contains(&p.connection_id) {
        let existing = registry.get(&p.connection_id)?;
        return Ok(json!({
            "success": true,
            "message": "Already connected",
            "version": existing.version,
        }));
    }

    // 1. 认证（在 control client 上做一次，query client 复用同一凭证）
    let version;
    let (user, role, default_db) = match p.auth {
        AuthKind::None => {
            let probe = build_client(&p, Auth::None)?;
            version = probe.health().map_err(|e| {
                error::qsdb_error(
                    "CONNECTION_FAILED",
                    None,
                    true,
                    format!("cannot reach QSDB at {}:{}: {}", p.runtime_host, p.runtime_port, e),
                )
            })?;
            if let Some(db) = &p.database {
                let dbs = probe.databases().map_err(|e| from_auth_error(e, "anonymous access rejected"))?;
                if !dbs.iter().any(|d| &d.name == db) {
                    return Err(error::qsdb_error(
                        "DATABASE_NOT_FOUND",
                        Some(404),
                        false,
                        format!("database '{db}' is not visible to this connection"),
                    ));
                }
            }
            (None, None, p.database.clone())
        }
        AuthKind::Basic => {
            let probe = build_client(&p, auth_for(&p))?;
            let info = probe.login(&p.username, &p.password).map_err(|e| from_auth_error(e, "login failed"))?;
            version = probe.health().unwrap_or_else(|_| "unknown".to_string());
            let mut default_db = info.default_db.clone();
            if let Some(db) = &p.database {
                probe
                    .use_database(db)
                    .map_err(|e| from_auth_error(e, format!("cannot select default database '{db}'")))?;
                default_db = db.clone();
            }
            (Some(info.user), info.role, Some(default_db))
        }
        AuthKind::Bearer => {
            let probe = build_client(&p, auth_for(&p))?;
            let dbs = probe.databases().map_err(|e| from_auth_error(e, "token validation failed"))?;
            version = probe.health().unwrap_or_else(|_| "unknown".to_string());
            if let Some(db) = &p.database {
                if !dbs.iter().any(|d| &d.name == db) {
                    return Err(error::qsdb_error(
                        "DATABASE_NOT_FOUND",
                        Some(404),
                        false,
                        format!("database '{db}' is not visible to this token"),
                    ));
                }
            }
            (None, None, p.database.clone())
        }
    };

    // 2. 常驻双 client：query（流式独占）+ control（metadata/cancel/写执行）。
    //    Basic 时两个 client 都 login——各自持有会话，revoked 后都能自动重登。
    let query_client = Arc::new(build_client(&p, auth_for(&p))?);
    let control_client = Arc::new(build_client(&p, auth_for(&p))?);
    if p.auth == AuthKind::Basic {
        for client in [&query_client, &control_client] {
            client
                .login(&p.username, &p.password)
                .map_err(|e| from_auth_error(e, "login failed"))?;
            if let Some(db) = &default_db {
                client
                    .use_database(db)
                    .map_err(|e| from_auth_error(e, format!("cannot select default database '{db}'")))?;
            }
        }
    }

    let session = Session {
        connection_id: p.connection_id.clone(),
        query_client,
        control_client,
        version: version.clone(),
        user,
        role,
        default_db,
        read_only: p.read_only,
        queries: std::sync::Mutex::new(Default::default()),
        spool_dir: crate::spool::connection_dir(&p.connection_id),
    };
    registry.insert(session);

    Ok(json!({
        "success": true,
        "message": "Connected",
        "version": version,
    }))
}

/// `connection/disconnect`：停查询、清 spool、移除会话。幂等。
pub fn disconnect(registry: &Registry, params: &Value) -> Result<Value, PluginError> {
    let p = parse(params)?;
    registry.remove(&p.connection_id); // 不存在也成功（幂等）
    Ok(json!({ "success": true }))
}
