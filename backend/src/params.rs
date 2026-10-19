//! lifecycle 请求参数解析（`connection/test|connect|disconnect` 的 params）。
//!
//! 载荷形状（以 DBX 运行时为准）：
//!
//! ```json
//! {
//!   "connection": { "id": "...", "host": "...", "port": 9430, "database": "...",
//!                   "username": "...", "password": "...",
//!                   "external_config": { "auth": "basic", "read_only": false,
//!                                        "connect_timeout_secs": 10 },
//!                   "connection_secrets": { "token": "..." } },
//!   "runtime": { "host": "...", "port": 49152 }
//! }
//! ```
//!
//! binding 落点（DBX host.rs 实测）：`binding:"config"` → `external_config`；
//! `binding:"secret"` → `connection_secrets[<field.key>]`；`binding:"password"`
//! → 顶层 `password`；其余按名对应。Secret 解析做了多落点兼容，但**只进
//! `Auth` 材料，不进日志/错误/事件**。

use serde_json::Value;

use crate::error::PluginError;

/// 认证方式（对应连接表单 `auth` 下拉）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    None,
    Basic,
    Bearer,
}

/// 一次 lifecycle 调用的完整参数。`Debug` 手写——**不派生默认 Debug**，
/// 防止密码/Token 随 `{:?}` 进任何日志。
pub struct LifecycleParams {
    pub connection_id: String,
    pub database: Option<String>,
    pub username: String,
    pub password: String,
    pub token: Option<String>,
    pub auth: AuthKind,
    pub read_only: bool,
    pub connect_timeout_secs: u64,
    /// DBX 隧道/代理后的**最终逻辑端点**——所有连接都用它，不用表单地址。
    pub runtime_host: String,
    pub runtime_port: u16,
}

impl std::fmt::Debug for LifecycleParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 精确到"有没有"，不泄露值。
        f.debug_struct("LifecycleParams")
            .field("connection_id", &self.connection_id)
            .field("runtime", &format!("{}:{}", self.runtime_host, self.runtime_port))
            .field("database", &self.database)
            .field("auth", &self.auth)
            .field("has_username", &!self.username.is_empty())
            .field("has_password", &!self.password.is_empty())
            .field("has_token", &self.token.is_some())
            .field("read_only", &self.read_only)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .finish()
    }
}

fn invalid(msg: &str) -> PluginError {
    PluginError::new(-32602, msg)
}

/// 从 lifecycle params 提取连接参数。任何失败都是**显式** -32602。
pub fn parse(params: &Value) -> Result<LifecycleParams, PluginError> {
    let connection = params
        .get("connection")
        .filter(|v| v.is_object())
        .ok_or_else(|| invalid("lifecycle params missing 'connection'"))?;
    let connection_id = connection
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("connection is missing 'id'"))?
        .to_string();

    // 端点：runtime.host/port 优先（DBX 隧道/代理后的最终端点）；
    // 缺 runtime 时回退表单地址（本地 dev host 的兜底路径）。
    let runtime = params.get("runtime");
    let runtime_host = runtime
        .and_then(|r| r.get("host"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| connection.get("host").and_then(Value::as_str).map(str::to_string))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("missing runtime.host"))?;
    let runtime_port = runtime
        .and_then(|r| r.get("port"))
        .and_then(Value::as_u64)
        .or_else(|| connection.get("port").and_then(Value::as_u64))
        .filter(|p| *p > 0 && *p <= u16::MAX as u64)
        .ok_or_else(|| invalid("missing or invalid runtime.port"))? as u16;

    let external = connection.get("external_config").unwrap_or(&Value::Null);
    let auth = match external.get("auth").and_then(Value::as_str) {
        Some("basic") => AuthKind::Basic,
        Some("bearer") => AuthKind::Bearer,
        _ => AuthKind::None,
    };
    let read_only = external.get("read_only").and_then(Value::as_bool).unwrap_or(false);
    let connect_timeout_secs = external
        .get("connect_timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .clamp(1, 300);

    // Secret 落点兼容：connection_secrets["token"]（DBX 标准 binding 落点）
    // → 顶层 "token"（兼容形态）→ 顶层 "password"（把 token 填进密码框的
    // 旧连接）。
    let token = connection
        .get("connection_secrets")
        .and_then(|s| s.get("token"))
        .and_then(Value::as_str)
        .or_else(|| connection.get("token").and_then(Value::as_str))
        .or_else(|| connection.get("password").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(LifecycleParams {
        connection_id,
        database: connection
            .get("database")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        username: connection.get("username").and_then(Value::as_str).unwrap_or_default().to_string(),
        password: connection.get("password").and_then(Value::as_str).unwrap_or_default().to_string(),
        token,
        auth,
        read_only,
        connect_timeout_secs,
        runtime_host,
        runtime_port,
    })
}

/// 拼客户端 URL。`qsdb://` 是 qsdb-client 的逻辑别名；底层仍是明文 HTTP/1.1，
/// 凭证永远走 Authorization 头，不进 URL。
pub fn client_url(p: &LifecycleParams) -> String {
    format!("qsdb://{}:{}", p.runtime_host, p.runtime_port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({
            "connection": {
                "id": "conn-1", "db_type": "plugin",
                "host": "127.0.0.1", "port": 9430,
                "database": "default",
                "username": "root", "password": "pw",
                "external_config": { "auth": "basic", "read_only": false, "connect_timeout_secs": 7 }
            },
            "runtime": { "host": "127.0.0.1", "port": 49152 }
        })
    }

    #[test]
    fn parses_full_payload() {
        let p = parse(&base()).unwrap();
        assert_eq!(p.connection_id, "conn-1");
        assert_eq!(p.runtime_host, "127.0.0.1");
        assert_eq!(p.runtime_port, 49152);
        assert_eq!(p.auth, AuthKind::Basic);
        assert_eq!(p.password, "pw");
        assert_eq!(p.connect_timeout_secs, 7);
        assert_eq!(client_url(&p), "qsdb://127.0.0.1:49152");
    }

    #[test]
    fn token_from_connection_secrets() {
        let mut v = base();
        v["connection"]["connection_secrets"] = json!({ "token": "tok-xyz" });
        let p = parse(&v).unwrap();
        assert_eq!(p.token.as_deref(), Some("tok-xyz"));
    }

    #[test]
    fn defaults_and_fallbacks() {
        let v = json!({
            "connection": { "id": "c2", "host": "h1", "port": 1, "password": "tok-as-pw" }
        });
        let p = parse(&v).unwrap();
        assert_eq!(p.auth, AuthKind::None);
        assert_eq!(p.connect_timeout_secs, 10);
        assert!(!p.read_only);
        // runtime 缺失 → 回退表单地址；password 里的值兜底作 token
        assert_eq!(p.runtime_host, "h1");
        assert_eq!(p.token.as_deref(), Some("tok-as-pw"));
    }

    #[test]
    fn missing_connection_is_invalid() {
        assert!(parse(&json!({})).is_err());
        let mut v = base();
        v["connection"]["id"] = json!(null);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn debug_never_contains_secrets() {
        let p = parse(&base()).unwrap();
        let text = format!("{p:?}");
        assert!(!text.contains("pw"));
        assert!(text.contains("has_password"));
    }
}
