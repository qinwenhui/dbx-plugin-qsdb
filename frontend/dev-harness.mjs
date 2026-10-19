#!/usr/bin/env node
// 无宿主调试 harness：本地 HTTP 服务 + 注入 window.dbxPlugin 垫片，
// 让构建产物（ui/index.html）能在浏览器里直连真实 sidecar。
//
//   node frontend/dev-harness.mjs                 # 匿名连 127.0.0.1:9430
//   QSDB_DEV_AUTH=basic QSDB_DEV_USER=root QSDB_DEV_PASSWORD=... node frontend/dev-harness.mjs
//   QSDB_DEV_URL=your-qsdb-host:9430 QSDB_DEV_DB=settle node frontend/dev-harness.mjs
//
// 仅开发用——不进包（[package].include 只有 assets/ui）；release 无任何
// dev server 依赖。

import http from "node:http";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import os from "node:os";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BACKEND_BIN = path.join(
    ROOT,
    "backend",
    "target",
    "release",
    process.platform === "win32" ? "dbx-plugin-qsdb.exe" : "dbx-plugin-qsdb",
  );
const UI_HTML = path.join(ROOT, "ui", "index.html");
const PORT = Number(process.env.QSDB_DEV_PORT ?? 5191);

const cfg = {
  url: process.env.QSDB_DEV_URL ?? "127.0.0.1:9430",
  auth: process.env.QSDB_DEV_AUTH ?? "none",
  user: process.env.QSDB_DEV_USER ?? "root",
  password: process.env.QSDB_DEV_PASSWORD ?? "",
  token: process.env.QSDB_DEV_TOKEN ?? "",
  database: process.env.QSDB_DEV_DB ?? "",
  readOnly: process.env.QSDB_DEV_READONLY === "1",
};

// ---- sidecar 进程 ----------------------------------------------------------
const dataDir = path.join(os.tmpdir(), `qsdb-dev-harness-${process.pid}`);
const child = spawn(BACKEND_BIN, [], {
  env: { ...process.env, DBX_PLUGIN_DATA_DIR: dataDir },
  stdio: ["pipe", "pipe", "inherit"],
});
child.on("exit", (code) => {
  console.error(`[harness] sidecar exited (${code})`);
  process.exit(1);
});
process.on("exit", () => child.kill());

let nextId = 1;
const pending = new Map();
const eventLog = [];
const eventWaiters = [];
let buffer = "";
child.stdout.on("data", (chunk) => {
  buffer += chunk.toString("utf8");
  let idx;
  while ((idx = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, idx).trim();
    buffer = buffer.slice(idx + 1);
    if (!line) continue;
    const msg = JSON.parse(line);
    if (msg.id !== undefined && msg.id !== null) {
      const resolve = pending.get(msg.id);
      if (resolve) {
        pending.delete(msg.id);
        resolve(msg);
      }
    } else if (msg.method) {
      eventLog.push({ seq: eventLog.length, ...msg });
      while (eventWaiters.length) eventWaiters.pop()();
    }
  }
});

function call(method, params) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, (msg) => (msg.error ? reject(msg.error) : resolve(msg.result)));
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    setTimeout(() => {
      if (pending.has(id)) {
        pending.delete(id);
        reject(new Error("sidecar timeout"));
      }
    }, 30000);
  });
}

// ---- 启动握手 + 连接 -------------------------------------------------------
child.stdin.write(
  JSON.stringify({
    jsonrpc: "2.0",
    id: 0,
    method: "plugin/initialize",
    params: { host: { protocolVersions: [1], hostApiVersion: "1.1.0", features: [] } },
  }) + "\n",
);

const CONNECTION_ID = "dev-conn";
async function connectSidecar() {
  const [host, port] = cfg.url.split(":");
  await call("connection/connect", {
    provider: { id: "cn.qinwh.qsdb.connection", databaseType: "qsdb" },
    connection: {
      id: CONNECTION_ID,
      db_type: "plugin",
      host,
      port: Number(port),
      database: cfg.database,
      username: cfg.user,
      password: cfg.password,
      external_config: { auth: cfg.auth, read_only: cfg.readOnly, connect_timeout_secs: 10 },
      connection_secrets: cfg.token ? { token: cfg.token } : {},
    },
    runtime: { host, port: Number(port) },
  });
  console.log(`[harness] connected to ${cfg.url} (auth=${cfg.auth}, db=${cfg.database || "default"})`);
}

// ---- storage（内存实现） ---------------------------------------------------
const storage = new Map();

// ---- HTTP 服务 -------------------------------------------------------------
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  try {
    if (req.method === "POST" && url.pathname === "/invoke") {
      const body = await readBody(req);
      const { method, params, timeoutMs } = JSON.parse(body);
      try {
        const result = await call(method, params);
        json(res, 200, { result });
      } catch (e) {
        json(res, 200, { error: e });
      }
      return;
    }
    if (url.pathname === "/events") {
      const since = Number(url.searchParams.get("since") ?? -1);
      const wait = () => new Promise((r) => eventWaiters.push(r));
      while (eventLog.length - 1 <= since) await Promise.race([wait(), sleep(15000)]);
      json(res, 200, { events: eventLog.slice(since + 1) });
      return;
    }
    if (url.pathname === "/storage") {
      const body = await readBody(req);
      const { op, key, value } = JSON.parse(body);
      if (op === "get") json(res, 200, { value: storage.get(key) ?? null });
      else if (op === "set") {
        storage.set(key, value);
        json(res, 200, {});
      } else {
        storage.delete(key);
        json(res, 200, {});
      }
      return;
    }
    if (url.pathname === "/") {
      let html = readFileSync(UI_HTML, "utf8");
      const shim = `<script>${shimSource()}</script>`;
      html = html.replace("<head>", `<head>${shim}`);
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(html);
      return;
    }
    res.writeHead(404).end();
  } catch (e) {
    json(res, 500, { error: String(e) });
  }
});

function readBody(req) {
  return new Promise((resolve, reject) => {
    let b = "";
    req.on("data", (c) => (b += c));
    req.on("end", () => resolve(b));
    req.on("error", reject);
  });
}

function json(res, status, body) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** 注入页面的 window.dbxPlugin 垫片（fetch 同源转发到 harness）。 */
function shimSource() {
  return `
  (() => {
    let seq = 0;
    const pending = new Map();
    const eventListeners = new Set();
    let lastSeq = -1;
    const context = { connectionId: ${JSON.stringify(CONNECTION_ID)}, database: ${JSON.stringify(cfg.database || undefined)} };
    const post = async (path, body) => (await fetch(path, { method: "POST", body: JSON.stringify(body) })).json();
    (async () => {
      for (;;) {
        const r = await fetch("/events?since=" + lastSeq).then((r) => r.json()).catch(() => null);
        if (r) for (const e of r.events) { lastSeq = e.seq; for (const l of [...eventListeners]) l(e); }
        await new Promise((r) => setTimeout(r, 120));
      }
    })();
    window.dbxPlugin = Object.freeze({
      ready: Promise.resolve(context),
      context,
      locale: navigator.language || "en",
      theme: { appearance: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light" },
      invoke: async (method, params, options) => {
        const r = await post("/invoke", { method, params, timeoutMs: options?.timeoutMs });
        if (r.error) {
          const d = r.error.data || {};
          const err = new Error((d.qsdbCode ? d.qsdbCode + ": " : "") + (r.error.message ?? "error"));
          err.data = d;
          throw err;
        }
        return r.result;
      },
      onEvent: (l) => { eventListeners.add(l); return () => eventListeners.delete(l); },
      onContext: () => () => undefined,
      storage: {
        get: async (key) => (await post("/storage", { op: "get", key })).value,
        set: async (key, value) => { await post("/storage", { op: "set", key, value }); },
        delete: async (key) => { await post("/storage", { op: "delete", key }); },
      },
    });
  })();`;
}

server.listen(PORT, async () => {
  console.log(`[harness] http://localhost:${PORT}  (sidecar pid ${child.pid})`);
  try {
    await connectSidecar();
  } catch (e) {
    console.error(`[harness] connect failed: ${JSON.stringify(e)}`);
    console.error("[harness] UI will show its no-connection state; fix QSDB_DEV_* env and restart.");
  }
});
