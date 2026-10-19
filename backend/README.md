# QSDB DBX Sidecar

Rust sidecar for the DBX QSDB plugin (`cn.qinwh.qsdb`): a JSON-RPC
(`stdio-jsonl`) process that owns all QSDB network I/O on top of
[`qsdb-client`](https://crates.io/crates/qsdb-client) (MIT).

## Modules

```text
src/main.rs        SDK server + method dispatch (stdout = protocol only)
src/params.rs      lifecycle payload parsing (secret-aware, redacted Debug)
src/error.rs       qsdb_client::Error → JSON-RPC error with data.qsdbCode
src/sqlclass.rs    read vs exec classification (conservative)
src/session.rs     session registry keyed by connection.id; QueryState
src/lifecycle.rs   connection/test / connect / disconnect (idempotent)
src/metadata.rs    server/info, metadata, partitions, table/size
src/query.rs       start/page/stop/close + started/progress/finished/error
src/spool.rs       JSONL row spool + offsets + ≤1 MiB pages
src/value.rs       contract value encoding (dates/decimal/interval/…)
vendor/dbx-plugin-sdk/  vendored SDK 0.1.0 (see its README)
tests/e2e.rs       end-to-end: real sidecar process over stdio + mock QSDB
tests/util/rows.rs minimal qsdb-rows wire encoder for the mock
```

## Behavior rules (enforced in code)

- stdout is protocol-only; all logs go to **stderr**.
- `connection/test/connect/disconnect` are idempotent.
- Endpoint = lifecycle `runtime.host` / `runtime.port`, never the form host.
- Two clients per connection: `query_client` (streaming reads) +
  `control_client` (metadata / cancel / writes).
- Secrets never appear in logs, events, errors, or `Debug` output (tested).
- Reads: `query_stream` → spool JSONL → `query/page` (default 500, max 2000,
  ≤1 MiB target per page). Writes/DDL: JSON receipt; a result set that comes
  back on the receipt channel is spooled, never dropped.
- Stop is two-phase: stop flag → `AbortHandle::abort()` / drop the parked
  `RowStream` (transfer phase, instant) → `control.cancel()` (execution
  phase; 404 expected once the query is deregistered). Received rows stay
  readable.
- No silent truncation: over-budget pages stop at a row boundary with
  `hasMore`; `bytes` missing from older servers is reported as unknown,
  not `0 B`.

## Build & test

```bash
cargo test          # 31 unit + 9 end-to-end tests (mock QSDB on localhost)
cargo build --release
```

`cargo test` spawns the real sidecar binary — no external network needed.

## Packaging

Use the DBX plugin CLI so it rewrites the packaged executable path:

```bash
dbx-plugin dev --path .. --port 5190
dbx-plugin package ..
```

Before store submission: keep `Cargo.toml` `version` equal to the root
`manifest.json` `version`, and switch the vendored SDK (`vendor/dbx-plugin-sdk`)
back to a version dependency once `dbx-plugin-sdk` is published on crates.io.
This project is Apache-2.0; do not copy QSDB Studio (PolyForm Noncommercial)
code into it.
