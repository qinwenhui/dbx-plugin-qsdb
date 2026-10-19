//! 端到端集成测试：**假 QSDB HTTP 服务** + 通过 stdio JSONL 驱动真实 sidecar。
//!
//! 覆盖接口验收用例：错误密码 401（消息不含密码）、connect/disconnect
//! 幂等、流式分页、两段式停止（已收行可读）、写语句回执、元数据、只读拦截、
//! 未知连接拒绝。
//!
//! mock QSDB 实现 sidecar 实际用到的端点子集（形状以 qsdb-client 0.1.0 的
//! 解析器为准）：/health、/api/v1/auth/login、/api/v1/databases、
//! /api/v1/sessions/current/database、/api/v1/tables?withSchema=1、
//! /api/v1/tables/{t}/partitions、/api/v1/queries/{id}/cancel、
//! POST /api/v1/query（rows 流式 / json 回执）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "util/rows.rs"]
mod qsdb_test_util;

// ---------------------------------------------------------------------------
// 假 QSDB 服务
// ---------------------------------------------------------------------------

const USER: &str = "root";
const PASSWORD: &str = "s3cret-pw";
const TOKEN: &str = "tok-mock-1";

struct Mock {
    port: u16,
    cancelled: Arc<AtomicBool>,
    stream_generation: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
}

impl Mock {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let port = listener.local_addr().unwrap().port();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream_generation = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let m = Mock {
            port,
            cancelled: Arc::clone(&cancelled),
            stream_generation: Arc::clone(&stream_generation),
            shutdown: Arc::clone(&shutdown),
        };
        // accept 循环放后台线程——Mock::start 必须立即返回，否则测试挂死。
        // 线程句柄 forget（进程退出即收；shutdown 只影响 accept 是否继续派生）。
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let cancelled = Arc::clone(&cancelled);
                let gen = Arc::clone(&stream_generation);
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, cancelled, gen);
                });
            }
        });
        std::mem::forget(handle);
        m
    }

    fn url(&self) -> String {
        format!("qsdb://127.0.0.1:{}", self.port)
    }

    fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 掏空 listener：连接一次促使 accept 返回
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Request {
    method: String,
    path: String,
    #[allow(dead_code)]
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    // 读到 \r\n\r\n
    loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 1024 * 1024 {
            return None;
        }
    }
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").expect("checked") + 4;
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let content_length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(Request { method, path, headers, body })
}

fn auth_ok(req: &Request) -> bool {
    let auth = req
        .headers
        .iter()
        .find(|(k, _)| k == "authorization")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    // Bearer 用户 token > Basic（与服务端优先级一致）
    if let Some(token) = auth.strip_prefix("Bearer ") {
        return token == TOKEN;
    }
    if let Some(b64) = auth.strip_prefix("Basic ") {
        return b64 == base64(format!("{USER}:{PASSWORD}"));
    }
    false
}

/// 无依赖 base64（标准字母表，带 padding；mock 只编码 ASCII）。
fn base64(s: String) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (chunk.get(1).copied().unwrap_or(0) as u32) << 8 | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, extra: &[(&str, &str)], body: &[u8]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n", body.len());
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_json(stream: &mut TcpStream, status: &str, body: Value) -> std::io::Result<()> {
    respond(stream, status, "application/json", &[], body.to_string().as_bytes())
}

fn unauthorized() -> Value {
    json!({ "error": "unauthorized", "code": "UNAUTHORIZED" })
}

fn handle_conn(mut stream: TcpStream, cancelled: Arc<AtomicBool>, generation: Arc<AtomicU64>) -> std::io::Result<()> {
    // keep-alive：qsdb-client 在一条连接上连续发多个请求——必须循环处理到
    // EOF，否则第二个请求撞上已关闭的连接（10053）。
    loop {
        let Some(req) = read_request(&mut stream) else { return Ok(()) };
        if dispatch(&mut stream, req, &cancelled, &generation)? {
            return Ok(()); // 流式响应（Connection: close）——连接到此为止
        }
    }
}

/// 处理单个请求；返回 true 表示连接应关闭（流式响应独占）。
fn dispatch(stream: &mut TcpStream, req: Request, cancelled: &Arc<AtomicBool>, generation: &Arc<AtomicU64>) -> std::io::Result<bool> {
    let path = req.path.split('?').next().unwrap_or("").to_string();
    match (req.method.as_str(), path.as_str()) {
        ("GET", "/health") => {
            respond_json(stream, "200 OK", json!({ "status": "ok", "version": "0.1.0-mock" }))?;
            Ok(false)
        }
        ("POST", "/api/v1/auth/login") => {
            if auth_ok(&req) {
                respond_json(
                    stream,
                    "200 OK",
                    json!({
                        "token": TOKEN,
                        "sessionId": "sess-1",
                        "user": USER,
                        "role": "admin",
                        "grants": {},
                        "defaultDb": "default"
                    }),
                )?;
            } else {
                respond_json(stream, "401 Unauthorized", unauthorized())?;
            }
            Ok(false)
        }
        ("GET", "/api/v1/databases") => {
            if !auth_ok(&req) {
                respond_json(stream, "401 Unauthorized", unauthorized())?;
                return Ok(false);
            }
            respond_json(
                stream,
                "200 OK",
                json!({
                    "databases": [
                        { "name": "default", "readOnly": false },
                        { "name": "settle", "readOnly": true }
                    ]
                }),
            )?;
            Ok(false)
        }
        ("PUT", "/api/v1/sessions/current/database") => {
            if !auth_ok(&req) {
                respond_json(stream, "401 Unauthorized", unauthorized())?;
                return Ok(false);
            }
            let db: Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            let name = db.get("database").and_then(Value::as_str).unwrap_or("default");
            if name == "nope" {
                respond_json(stream, "404 Not Found", json!({ "error": "no such database", "code": "DATABASE_NOT_FOUND" }))?;
                return Ok(false);
            }
            respond_json(stream, "200 OK", json!({ "defaultDb": name }))?;
            Ok(false)
        }
        ("GET", "/api/v1/tables") => {
            if !auth_ok(&req) {
                respond_json(stream, "401 Unauthorized", unauthorized())?;
                return Ok(false);
            }
            // 多挂载：全部表名给限定形态（与真实服务端行为一致）
            respond_json(
                stream,
                "200 OK",
                json!({
                    "tables": [
                        {
                            "name": "default.sales",
                            "columns": [
                                { "name": "id", "type": "Int64", "nullable": false, "partition": false },
                                { "name": "region", "type": "Utf8", "nullable": true, "partition": false },
                                { "name": "dt", "type": "Date32", "nullable": false, "partition": true }
                            ]
                        },
                        { "name": "settle.txn", "columns": [ { "name": "amount", "type": "Int64", "nullable": true, "partition": false } ] }
                    ]
                }),
            )?;
            Ok(false)
        }
        ("GET", p) if p.ends_with("/default.sales/partitions") => {
            if !auth_ok(&req) {
                respond_json(stream, "401 Unauthorized", unauthorized())?;
                return Ok(false);
            }
            respond_json(
                stream,
                "200 OK",
                json!({
                    "detail": [
                        { "name": "dt=2026-01-01", "generation": 7, "rows": 100000, "segments": 3, "bytes": 2387104 },
                        { "name": "dt=2026-01-02", "generation": 2, "rows": 50, "segments": 1, "bytes": 20480 }
                    ]
                }),
            )?;
            Ok(false)
        }
        ("POST", p) if p.starts_with("/api/v1/queries/") && p.ends_with("/cancel") => {
            // 传输段已注销 → 404 预期；这里两种都接受（sidecar 都要容忍）
            if cancelled.load(Ordering::SeqCst) {
                respond_json(stream, "404 Not Found", json!({ "error": "not found", "code": "QUERY_NOT_FOUND" }))?;
            } else {
                cancelled.store(true, Ordering::SeqCst);
                respond_json(stream, "200 OK", json!({ "cancelled": true }))?;
            }
            Ok(false)
        }
        ("POST", "/api/v1/query") => {
            handle_query(stream, &req, Arc::clone(cancelled), Arc::clone(generation))?;
            Ok(true) // 流式响应独占连接
        }
        _ => {
            respond_json(stream, "404 Not Found", json!({ "error": "no such endpoint", "code": "NOT_FOUND" }))?;
            Ok(false)
        }
    }
}

/// POST /api/v1/query：按 body.format 分派 rows（流式）与 json（回执）。
fn handle_query(
    stream: &mut TcpStream,
    req: &Request,
    cancelled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
) -> std::io::Result<()> {
    if !auth_ok(req) {
        return respond_json(stream, "401 Unauthorized", unauthorized());
    }
    let body: Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
    let sql = body.get("sql").and_then(Value::as_str).unwrap_or_default();
    let format = body.get("format").and_then(Value::as_str).unwrap_or("bulk");
    let query_id = body.get("queryId").and_then(Value::as_str).unwrap_or("q-mock").to_string();

    // 显式错误路径
    if sql.contains("FROM missing") {
        return respond_json(stream, "400 Bad Request", json!({ "error": "table not found: missing", "code": "TABLE_NOT_FOUND" }));
    }
    if sql.contains("DENIED") {
        return respond_json(stream, "403 Forbidden", json!({ "error": "grants deny SELECT", "code": "FORBIDDEN" }));
    }

    let headers = [("x-qsdb-query-id", query_id.as_str())];
    if format == "rows" {
        // 流式：小查询 1 块；`FROM big` 慢流多块（停止用例）。generation 递增
        // 标记新流（stop 用例在第二批数据流中触发 abort）。
        let gen = generation.fetch_add(1, Ordering::SeqCst);
        let big = sql.contains("FROM big");
        let columns: Vec<(String, qsdb_test_util::Tag)> = if big {
            vec![("n".into(), qsdb_test_util::Tag::I64)]
        } else {
            vec![
                ("region".into(), qsdb_test_util::Tag::Utf8),
                ("amount".into(), qsdb_test_util::Tag::I64),
            ]
        };
        let header = qsdb_test_util::encode_header(&columns);
        let mut head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/vnd.qsdb.rows\r\nx-qsdb-query-id: {query_id}\r\nx-qsdb-elapsed-ms: 12.5\r\nconnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes())?;
        head.clear();
        stream.flush()?;
        let _ = head;
        let mut w = stream.try_clone()?;
        let mut send_chunk = move |payload: &[u8]| -> std::io::Result<()> {
            w.write_all(payload)?;
            w.flush()
        };
        if !big {
            // 两行 + 一个 NULL
            let chunk = qsdb_test_util::encode_chunk(
                &columns,
                &[
                    vec![qsdb_test_util::Cell::Str("east"), qsdb_test_util::Cell::Int(1200)],
                    vec![qsdb_test_util::Cell::Str("west"), qsdb_test_util::Cell::Int(980)],
                    vec![qsdb_test_util::Cell::Null, qsdb_test_util::Cell::Int(7)],
                ],
            );
            let _ = send_chunk(&header);
            let _ = send_chunk(&chunk);
            return Ok(());
        }
        // 慢流：200 块 × 500 行，块间 30ms → 传输段窗口充足
        let _ = send_chunk(&header);
        for i in 0..200u32 {
            if cancelled.load(Ordering::SeqCst) {
                // 真实服务端在取消后收尾流；这里也停止发送
                return Ok(());
            }
            let rows: Vec<Vec<qsdb_test_util::Cell>> =
                (0..500).map(|j| vec![qsdb_test_util::Cell::Int((i * 500 + j) as i64)]).collect();
            let chunk = qsdb_test_util::encode_chunk(&columns, &rows);
            if send_chunk(&chunk).is_err() {
                return Ok(()); // 客户端 abort 关闭了 socket —— 预期
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let _ = gen;
        Ok(())
    } else {
        // JSON：回执（DDL/DML）
        let (command, message, affected) = if sql.starts_with("CREATE") {
            ("CREATE", "table created", 0)
        } else if sql.starts_with("INSERT") {
            ("INSERT", "inserted", 2)
        } else {
            ("EXEC", "ok", 0)
        };
        respond_json(
            stream,
            "200 OK",
            json!({
                "queryId": query_id,
                "schema": [],
                "rows": [],
                "rowCount": 0,
                "elapsedMs": 1.0,
                "message": message,
                "command": command,
                "rowsAffected": affected
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Sidecar 驱动
// ---------------------------------------------------------------------------

struct Sidecar {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    events: Arc<Mutex<Vec<Value>>>,
    responses: std::sync::mpsc::Receiver<(u64, Value)>,
    next_id: u64,
    stderr: Arc<Mutex<String>>,
    data_dir: std::path::PathBuf,
}

impl Sidecar {
    fn spawn() -> Self {
        // 唯一目录：并行测试各自独立（同毫秒 + 同进程 id 会撞）。
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let data_dir = std::env::temp_dir().join(format!("qsdb-e2e-data-{}-{}-{seq}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_dbx-plugin-qsdb"))
            .env("DBX_PLUGIN_DATA_DIR", &data_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sidecar");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let events = Arc::clone(&events);
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stdout);
                for line in std::io::BufRead::lines(reader) {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        panic!("stdout carried non-JSON: {line}");
                    };
                    if v.get("method").is_some() && v.get("id").is_none() {
                        events.lock().unwrap().push(v);
                    } else if let Some(id) = v.get("id").and_then(Value::as_u64) {
                        let _ = tx.send((id, v));
                    }
                }
            });
        }
        let stderr_acc = Arc::new(Mutex::new(String::new()));
        {
            let acc = Arc::clone(&stderr_acc);
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = std::io::Read::read_to_string(&mut { stderr }, &mut s);
                *acc.lock().unwrap() = s;
            });
        }
        Sidecar { child, stdin, events, responses: rx, next_id: 1, stderr: stderr_acc, data_dir }
    }

    /// 发请求并等响应（跳过沿途事件；响应里的 result / error 原样返回）。
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{line}").expect("write request");
        self.stdin.flush().expect("flush");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let remain = deadline.saturating_duration_since(Instant::now());
            let (rid, v) = self
                .responses
                .recv_timeout(remain.max(Duration::from_millis(1)))
                .expect("sidecar response timeout");
            assert_eq!(rid, id, "response id mismatch");
            if let Some(err) = v.get("error") {
                return Err(err.clone());
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// 等待一个特定事件（谓词匹配；最多等 `secs` 秒）。
    fn wait_event(&self, secs: u64, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            {
                let events = self.events.lock().unwrap();
                if let Some(e) = events.iter().find(|e| pred(e)) {
                    return Some(e.clone());
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn events(&self) -> Vec<Value> {
        self.events.lock().unwrap().clone()
    }

    /// 初始化握手（plugin/initialize）。
    fn initialize(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({
            "jsonrpc": "2.0", "id": id, "method": "plugin/initialize",
            "params": { "host": { "protocolVersions": [1], "hostApiVersion": "1.1.0", "features": [] } }
        });
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remain = deadline.saturating_duration_since(Instant::now());
            let (rid, v) = self.responses.recv_timeout(remain.max(Duration::from_millis(1))).expect("initialize timeout");
            assert_eq!(rid, id);
            let result = v.get("result").expect("initialize should succeed");
            assert_eq!(result["plugin"]["id"], "cn.qinwh.qsdb");
            assert_eq!(result["plugin"]["version"], "0.1.0");
            assert_eq!(result["protocolVersion"], 1);
            break;
        }
    }

    fn lifecycle_params(&self, mock: &Mock, auth: &str, password: &str) -> Value {
        json!({
            "provider": { "id": "cn.qinwh.qsdb.connection", "databaseType": "qsdb" },
            "connection": {
                "id": "conn-e2e", "db_type": "plugin",
                "host": "127.0.0.1", "port": mock.port,
                "database": "default",
                "username": USER,
                "password": password,
                "external_config": { "auth": auth, "read_only": false, "connect_timeout_secs": 5 }
            },
            "runtime": { "host": "127.0.0.1", "port": mock.port }
        })
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

// ---------------------------------------------------------------------------
// 用例
// ---------------------------------------------------------------------------

fn fresh_sidecar() -> Sidecar {
    let mut sc = Sidecar::spawn();
    sc.initialize();
    sc
}

#[test]
fn handshake_identity_matches_manifest() {
    let _ = fresh_sidecar();
}

#[test]
fn test_connect_disconnect_lifecycle() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();

    // test（Basic 正确密码）
    let r = sc.call("connection/test", sc.lifecycle_params(&mock, "basic", PASSWORD)).expect("test ok");
    assert_eq!(r["success"], true);
    assert_eq!(r["version"], "0.1.0-mock");
    assert_eq!(r["user"], USER);
    assert_eq!(r["role"], "admin");
    assert_eq!(r["defaultDb"], "default");

    // test（错误密码）→ 401 UNAUTHORIZED，消息不含密码
    let e = sc.call("connection/test", sc.lifecycle_params(&mock, "basic", "WRONG")).unwrap_err();
    assert_eq!(e["data"]["status"], 401, "{e}");
    assert_eq!(e["data"]["qsdbCode"], "UNAUTHORIZED");
    assert!(!e["message"].to_string().contains(PASSWORD));
    assert!(!e["message"].to_string().contains("WRONG"));

    // test（Bearer 错 token）→ 401
    let mut p = sc.lifecycle_params(&mock, "bearer", "");
    p["connection"]["connection_secrets"] = json!({ "token": "bad-token" });
    p["connection"].as_object_mut().unwrap().remove("password");
    let e = sc.call("connection/test", p).unwrap_err();
    assert_eq!(e["data"]["status"], 401, "{e}");

    // connect ×2 幂等
    let r1 = sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).expect("connect");
    assert_eq!(r1["success"], true);
    let r2 = sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).expect("connect again");
    assert_eq!(r2["success"], true);

    // disconnect ×2 幂等
    let d1 = sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).expect("disconnect");
    assert_eq!(d1["success"], true);
    let d2 = sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).expect("disconnect again");
    assert_eq!(d2["success"], true);

    // 断开后旧连接不可访问
    let e = sc
        .call("qsdb/server/info", json!({ "connectionId": "conn-e2e" }))
        .unwrap_err();
    assert_eq!(e["data"]["qsdbCode"], "CONNECTION_NOT_FOUND");

    // stderr 不含密码 / token
    let err = sc.stderr.lock().unwrap().clone();
    assert!(!err.contains(PASSWORD), "stderr leaked password: {err}");
    assert!(!err.contains(TOKEN), "stderr leaked token");

    mock.stop();
}

#[test]
fn metadata_and_partitions() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();
    sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();

    let info = sc.call("qsdb/server/info", json!({ "connectionId": "conn-e2e" })).unwrap();
    assert_eq!(info["version"], "0.1.0-mock");
    assert_eq!(info["user"], USER);
    assert_eq!(info["role"], "admin");
    assert_eq!(info["defaultDb"], "default");
    assert_eq!(info["readOnly"], false);

    let meta = sc.call("qsdb/metadata", json!({ "connectionId": "conn-e2e" })).unwrap();
    let dbs = meta["databases"].as_array().unwrap();
    assert_eq!(dbs.len(), 2);
    assert_eq!(dbs[0]["name"], "default");
    assert_eq!(dbs[1]["readOnly"], true);
    let tables = meta["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0]["name"], "default.sales");
    assert_eq!(tables[0]["columns"].as_array().unwrap().len(), 3);
    assert_eq!(tables[0]["columns"][2]["partition"], true);
    // 限定名归属
    assert_eq!(tables[1]["database"], "settle");
    // 按库过滤
    let meta2 = sc.call("qsdb/metadata", json!({ "connectionId": "conn-e2e", "database": "settle" })).unwrap();
    assert_eq!(meta2["tables"].as_array().unwrap().len(), 1);

    let parts = sc.call("qsdb/partitions", json!({ "connectionId": "conn-e2e", "table": "default.sales" })).unwrap();
    let parts = parts.as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["generation"], 7);
    assert_eq!(parts[0]["bytes"], 2387104);
    assert_eq!(parts[0]["bytesKnown"], true);

    let size = sc.call("qsdb/table/size", json!({ "connectionId": "conn-e2e", "table": "default.sales" })).unwrap();
    assert_eq!(size["partitionCount"], 2);
    assert_eq!(size["rows"], 100050);
    assert_eq!(size["bytesKnown"], true);

    sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();
    mock.stop();
}

#[test]
fn streaming_query_pages_and_finishes() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();
    sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();

    let started_resp = sc
        .call(
            "qsdb/query/start",
            json!({ "connectionId": "conn-e2e", "sql": "SELECT region, amount FROM default.sales", "database": "default" }),
        )
        .expect("start");
    let qid = started_resp["queryId"].as_str().unwrap().to_string();

    // started 事件带列
    let ev = sc.wait_event(10, |e| e["method"] == "qsdb/query/started" && e["params"]["queryId"] == qid.as_str()).expect("started event");
    let cols = ev["params"]["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0]["name"], "region");
    assert_eq!(cols[0]["typeName"], "VARCHAR");
    assert_eq!(cols[1]["tag"], "i64");

    // finished 事件
    let ev = sc.wait_event(10, |e| e["method"] == "qsdb/query/finished" && e["params"]["queryId"] == qid.as_str()).expect("finished event");
    assert_eq!(ev["params"]["kind"], "rows");
    assert_eq!(ev["params"]["totalRows"], 3);

    // page：NULL 与特殊值
    let page = sc
        .call("qsdb/query/page", json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 0, "limit": 2 }))
        .unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    assert_eq!(page["totalRows"], 3);
    assert_eq!(page["hasMore"], true);
    let page2 = sc
        .call("qsdb/query/page", json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 2, "limit": 500 }))
        .unwrap();
    let rows2 = page2["rows"].as_array().unwrap();
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0][0], Value::Null); // NULL → null
    assert_eq!(rows2[0][1], 7);
    assert_eq!(page2["done"], true);
    assert_eq!(page2["hasMore"], false);

    // 越界
    let page3 = sc
        .call("qsdb/query/page", json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 99 }))
        .unwrap();
    assert_eq!(page3["rows"].as_array().unwrap().len(), 0);

    // close 后不可访问
    sc.call("qsdb/query/close", json!({ "connectionId": "conn-e2e", "queryId": qid })).unwrap();
    let e = sc
        .call("qsdb/query/page", json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 0 }))
        .unwrap_err();
    assert_eq!(e["data"]["qsdbCode"], "QUERY_NOT_FOUND");

    sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();
    mock.stop();
}

#[test]
fn query_error_surfaces_qsdb_code() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();
    sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();

    let started = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "SELECT * FROM missing" }))
        .unwrap();
    let qid = started["queryId"].as_str().unwrap();
    let ev = sc
        .wait_event(10, |e| e["method"] == "qsdb/query/error" && e["params"]["queryId"] == qid)
        .expect("error event");
    assert_eq!(ev["params"]["code"], "TABLE_NOT_FOUND");
    assert_eq!(ev["params"]["receivedRows"], 0);

    // 403 授权错误同样透码
    let started = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "SELECT * FROM DENIED" }))
        .unwrap();
    let qid = started["queryId"].as_str().unwrap();
    let ev = sc
        .wait_event(10, |e| e["method"] == "qsdb/query/error" && e["params"]["queryId"] == qid)
        .expect("error event");
    assert_eq!(ev["params"]["code"], "FORBIDDEN");

    sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();
    mock.stop();
}

#[test]
fn exec_statement_returns_receipt_not_rows() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();
    sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();

    let started = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "CREATE TABLE t2 (a BIGINT)" }))
        .unwrap();
    let qid = started["queryId"].as_str().unwrap();
    let ev = sc
        .wait_event(10, |e| e["method"] == "qsdb/query/finished" && e["params"]["queryId"] == qid)
        .expect("finished");
    assert_eq!(ev["params"]["kind"], "exec");
    assert_eq!(ev["params"]["command"], "CREATE");
    assert_eq!(ev["params"]["rowsAffected"], 0);

    // 写查询没有 cursor
    let e = sc
        .call("qsdb/query/page", json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 0 }))
        .unwrap_err();
    assert_eq!(e["data"]["qsdbCode"], "NO_CURSOR");

    sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();
    mock.stop();
}

#[test]
fn stop_during_transfer_keeps_received_rows() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();
    sc.call("connection/connect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();

    let started = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "SELECT n FROM big" }))
        .unwrap();
    let qid = started["queryId"].as_str().unwrap().to_string();

    // 等一些行已收
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut received = 0u64;
    while Instant::now() < deadline {
        if let Ok(page) = sc.call(
            "qsdb/query/page",
            json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 0, "limit": 1 }),
        ) {
            received = page["totalRows"].as_u64().unwrap_or(0);
            if received >= 500 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(received >= 500, "expected progress before stop, got {received}");

    // 停止
    let stopped = sc.call("qsdb/query/stop", json!({ "connectionId": "conn-e2e", "queryId": qid })).unwrap();
    assert_eq!(stopped["stopped"], true);

    // cancelled 事件
    let ev = sc
        .wait_event(15, |e| e["method"] == "qsdb/query/error" && e["params"]["queryId"] == qid.as_str())
        .expect("cancelled event");
    assert_eq!(ev["params"]["code"], "QUERY_CANCELLED");
    assert!(ev["params"]["receivedRows"].as_u64().unwrap_or(0) >= 500);

    // 已收行仍可读
    let page = sc
        .call("qsdb/query/page", json!({ "connectionId": "conn-e2e", "queryId": qid, "offset": 0, "limit": 10 }))
        .expect("page after stop");
    assert!(page["rows"].as_array().unwrap().len() > 0);

    // 连接仍可用：新查询照常
    let started2 = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "SELECT region, amount FROM default.sales" }))
        .expect("new query after stop");
    let qid2 = started2["queryId"].as_str().unwrap();
    let _ = sc.wait_event(10, |e| e["method"] == "qsdb/query/finished" && e["params"]["queryId"] == qid2);
    // 收尾
    let _ = sc.call("qsdb/query/close", json!({ "connectionId": "conn-e2e", "queryId": qid }));
    let _ = sc.call("qsdb/query/close", json!({ "connectionId": "conn-e2e", "queryId": qid2 }));
    sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();
    mock.stop();
}

#[test]
fn read_only_connection_rejects_writes() {
    let mock = Mock::start();
    let mut sc = fresh_sidecar();
    let mut p = sc.lifecycle_params(&mock, "basic", PASSWORD);
    p["connection"]["external_config"]["read_only"] = json!(true);
    sc.call("connection/connect", p).unwrap();

    let info = sc.call("qsdb/server/info", json!({ "connectionId": "conn-e2e" })).unwrap();
    assert_eq!(info["readOnly"], true);

    let e = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "INSERT INTO t VALUES (1)" }))
        .unwrap_err();
    assert_eq!(e["data"]["qsdbCode"], "READ_ONLY_CONNECTION", "{e}");

    // 读照常
    let started = sc
        .call("qsdb/query/start", json!({ "connectionId": "conn-e2e", "sql": "SELECT region, amount FROM default.sales" }))
        .expect("read on read-only connection");
    let qid = started["queryId"].as_str().unwrap();
    assert!(sc.wait_event(10, |e| e["method"] == "qsdb/query/finished" && e["params"]["queryId"] == qid).is_some());

    sc.call("connection/disconnect", sc.lifecycle_params(&mock, "basic", PASSWORD)).unwrap();
    mock.stop();
}

#[test]
fn unknown_connection_rejected() {
    let _mock_unused = Mock::start();
    let mut sc = fresh_sidecar();
    let e = sc
        .call("qsdb/query/start", json!({ "connectionId": "ghost", "sql": "SELECT 1" }))
        .unwrap_err();
    assert_eq!(e["data"]["qsdbCode"], "CONNECTION_NOT_FOUND");
    let e = sc.call("qsdb/metadata", json!({ "connectionId": "ghost" })).unwrap_err();
    assert_eq!(e["data"]["qsdbCode"], "CONNECTION_NOT_FOUND");
}
