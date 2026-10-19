//! 错误映射：`qsdb_client::Error` → JSON-RPC `PluginError`。
//!
//! 错误形状：
//!
//! ```json
//! { "code": -32000, "message": "FORBIDDEN: …",
//!   "data": { "qsdbCode": "FORBIDDEN", "status": 403, "retriable": false } }
//! ```
//!
//! 红线：消息里不出现密码、Token、带凭证的连接串（我们的 URL 只有
//! host:port，凭证走头，不会泄进服务端错误文案；仍做一层防御性裁剪）。

pub use dbx_plugin_sdk::PluginError;
use serde_json::json;

/// 协议层通用错误码（JSON-RPC 保留段之外的插件自定义段，随 SDK 惯例用 -32000）。
pub const CODE: i32 = -32000;

/// 构造带 QSDB 业务码的错误。
pub fn qsdb_error(qsdb_code: &str, status: Option<u16>, retriable: bool, message: impl Into<String>) -> PluginError {
    let mut e = PluginError::new(CODE, sanitize(&message.into()));
    e.data = Some(json!({
        "qsdbCode": qsdb_code,
        "status": status,
        "retriable": retriable,
    }));
    e
}

/// 无 QSDB 业务码的通用失败。
pub fn internal(message: impl Into<String>) -> PluginError {
    PluginError::new(CODE, sanitize(&message.into()))
}

/// 参数错误（缺字段/类型不对）。
pub fn invalid_param(message: impl Into<String>) -> PluginError {
    PluginError::new(-32602, sanitize(&message.into()))
}

/// 把 qsdb-client 错误映射成协议错误；401/403 保持可区分。
pub fn from_qsdb(err: &qsdb_client::Error) -> PluginError {
    match err {
        qsdb_client::Error::Api { status, code, message } => {
            let retriable = err.retriable();
            let text = if code.is_empty() {
                message.clone()
            } else {
                format!("{code}: {message}")
            };
            qsdb_error(if code.is_empty() { "API_ERROR" } else { code.as_str() }, Some(*status), retriable, text)
        }
        qsdb_client::Error::Io(e) => {
            qsdb_error("NETWORK_ERROR", None, true, format!("connection to QSDB failed: {e}"))
        }
        qsdb_client::Error::Http(m) => qsdb_error("PROTOCOL_ERROR", None, false, format!("HTTP protocol error: {m}")),
        qsdb_client::Error::Decode(m) => qsdb_error("DECODE_ERROR", None, false, format!("failed to decode QSDB response: {m}")),
        qsdb_client::Error::Usage(m) => qsdb_error("USAGE_ERROR", None, false, format!("client misuse: {m}")),
    }
}

/// 防御性裁剪：防万一上游错误文案把 Authorization 头或长连接串带出来。
/// 保守做法是掐掉常见凭证形态，而不是试图"清洗得更好看"。
fn sanitize(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    for line in msg.lines() {
        let line = if line.to_ascii_lowercase().contains("authorization:") {
            "authorization: [redacted]"
        } else {
            line
        };
        out.push_str(line);
        out.push('\n');
    }
    let mut out = out.trim_end().to_string();
    out.truncate(2000); // 单条错误消息上限，避免服务端大 HTML 冲垮 UI
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_api_error_with_code_and_status() {
        let e = from_qsdb(&qsdb_client::Error::Api {
            status: 403,
            code: "FORBIDDEN".into(),
            message: "grants deny SELECT".into(),
        });
        assert_eq!(e.code, CODE);
        assert!(e.message.starts_with("FORBIDDEN: "));
        let data = e.data.unwrap();
        assert_eq!(data["qsdbCode"], "FORBIDDEN");
        assert_eq!(data["status"], 403);
        assert_eq!(data["retriable"], false);
    }

    #[test]
    fn maps_io_error_retriable() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let e = from_qsdb(&qsdb_client::Error::Io(io));
        assert_eq!(e.data.as_ref().unwrap()["qsdbCode"], "NETWORK_ERROR");
        assert_eq!(e.data.as_ref().unwrap()["retriable"], true);
    }

    #[test]
    fn sanitizes_authorization_lines() {
        let e = internal("GET / with authorization: Basic c2VjcmV0 failed");
        assert!(!e.message.contains("c2VjcmV0"));
        assert!(e.message.contains("[redacted]"));
    }
}
