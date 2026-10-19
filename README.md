# dbx-plugin-qsdb

A [DBX](https://github.com/t8y2/dbx) plugin that adds a native
[QSDB](https://github.com/qinwenhui/qsdb) workbench: schema browsing,
analytical SQL, large-result paging, query cancellation, partitions and
table-size inspection. · [中文说明](README.zh-CN.md)

- **Plugin id:** `cn.qinwh.qsdb` · **Connection type:** `qsdb`
- **Requires:** DBX `>=0.6.0` (Host API `^1.1`), QSDB `0.1.x`
- **License:** Apache-2.0 (this plugin); `qsdb-client` is MIT

## Why a plugin instead of JDBC?

DBX already supports custom JDBC drivers, which is the fastest way to get
basic SQL access to QSDB. This plugin exists for a first-class QSDB
experience:

- QSDB-native schema/mounts model
- partition/generation and table-byte inspection
- two-phase query stop (execution cancel + transfer abort)
- large-result cursor with paging instead of one giant JSON payload
- a workbench designed for QSDB rather than a generic JDBC grid

The plugin talks to the QSDB HTTP API through a Rust sidecar; it does not
implement a second wire protocol.

## Features

- **QSDB-native schema browser** — databases/mounts → tables → columns, with
  filter, refresh, and one-click table preview
- **SQL editor** — CodeMirror 6, run/stop, query history and drafts stored in
  DBX plugin storage
- **Streaming results** — read queries stream through a disk spool and are
  browsed by page (default 500 rows, ≤1 MiB per page); 100k+ row results stay
  responsive and are never silently truncated
- **Two-phase query stop** — server-side cancel (execution phase) plus
  transfer abort (streaming phase); rows already received remain browsable
- **Partitions panel** — per-partition generation / rows / segments / bytes
  and table size; if a server does not report bytes it shows "unknown",
  never a fake `0 B`
- **Typed values** — NULL, big integers beyond the JS safe range, decimals,
  binary (hex), and date/time types render per the QSDB wire contract
- **Auth** — anonymous, Basic, or Bearer token; credentials live only in DBX
  secret storage, never in logs, events, or the workbench context
- **Read-only connections** enforced in the UI and again in the sidecar
- **Follows the host** — light/dark theme and English/简体中文 UI

## Architecture

```text
DBX
├── connection form (host / port / auth / default database)
├── QSDB workbench (Vue 3 static assets, sandboxed iframe)
└── Rust sidecar (stdio-jsonl JSON-RPC)
        └── qsdb-client → QSDB HTTP API (runtime.host:runtime.port)
```

The sidecar owns all QSDB network I/O and dials through the DBX tunnel/proxy
endpoint (`runtime.host` / `runtime.port` from the lifecycle payload). The
sandboxed UI has no direct network access; it talks to the sidecar through
the DBX plugin bridge only. Secrets are bound with DBX `password`/`secret`
bindings and stored in DBX secret storage.

## Install

**From the DBX Store:** search for “QSDB” (pending store review).

**From a release:** grab a `.dbxp` package from
[Releases](https://github.com/qinwenhui/dbx-plugin-qsdb/releases) and install
it through DBX's plugin manager.

**From source:**

```bash
git clone https://github.com/qinwenhui/dbx-plugin-qsdb
cd dbx-plugin-qsdb

# 1. sidecar
cargo build --release --manifest-path backend/Cargo.toml

# 2. workbench UI (produces ui/index.html)
cd frontend && npm install && npm run build && cd ..

# 3. package (requires the DBX plugin CLI: npm i -g @dbx-app/plugin-cli)
dbx-plugin package .
```

## Development

```bash
cd backend  && cargo test     # unit + end-to-end tests (mock QSDB on localhost)
cd frontend && npm run build  # build the workbench
node frontend/dev-harness.mjs # debug the UI in a browser against a real sidecar
                              # + real QSDB (see frontend/README.md)
dbx-plugin dev --path . --port 5190
```

Keep `backend/Cargo.toml` `version` in sync with `manifest.json`.

## Security notes

- QSDB speaks plain HTTP/1.1. Put a TLS-terminating proxy in front of QSDB
  when the link needs encryption; this plugin does not fabricate TLS.
- The plugin requests only `host.workbench`, `host.events`, `host.storage`.
- Read-only mode is enforced twice (UI + sidecar); server-side grants remain
  the final boundary.
- This is an independent integration, licensed Apache-2.0. It is not QSDB
  Studio and contains no QSDB Studio code.

## License

Apache License 2.0 — see [LICENSE](LICENSE).
