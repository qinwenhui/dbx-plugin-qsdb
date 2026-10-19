# QSDB DBX Workbench UI

Vue 3 + TypeScript + Vite workbench for the DBX QSDB plugin. Builds to a
**single static file** `ui/index.html` (JS/CSS inlined via
vite-plugin-singlefile); release contains no dev server, no CDN, no external
fonts. `ui/` is generated — do not edit by hand.

## Host API only

```ts
const api = window.dbxPlugin
await api.ready

const info = await api.invoke('qsdb/server/info', { connectionId })
const start = await api.invoke('qsdb/query/start', { connectionId, sql })
api.onEvent((event) => {
  // qsdb/query/started | progress | finished | error
})
const page = await api.invoke('qsdb/query/page', {
  connectionId, queryId: start.queryId, offset: 0, limit: 500,
})
```

- Never call QSDB directly from the iframe (CSP has no network at all).
- `connectionId` comes from the Workbench context (`api.ready`).
- Theme/locale follow `api.theme` / `api.locale` (plus `env` pushes); light &
  dark are handled with DBX design tokens only.
- `api.storage` holds the SQL draft and query history (≤256 KiB per value,
  1 MiB total). Never store passwords/tokens.

## Structure

```text
src/
├── main.ts                  # mount after api.ready
├── bridge.ts                # window.dbxPlugin types + invoke/event helpers
├── store.ts                 # reactive store: connection, query lifecycle, paging cache
├── i18n.ts                  # en / zh-CN copy
├── styles.css               # layout on top of the host-injected ui-kit + tokens
└── components/
    ├── SchemaTree.vue       # databases → tables → columns; filter / refresh / preview
    ├── SqlEditor.vue        # CodeMirror 6; Ctrl/Cmd+Enter runs
    ├── ResultGrid.vue       # virtualized grid over paged spool; NULL styling
    └── PartitionPanel.vue   # generation / rows / segments / bytes + table size
```

UX rules:

- Run disables to Stop while executing; stop shows "cancelled" and **keeps
  received rows browsable**.
- Results distinguish `rows` vs `exec`: DDL/DML render a receipt with affected
  rows; they have no cursor.
- Row counts show `loaded N / M` and `receiving…` while the stream is open.
- Large results render only the visible window; pages are fetched on demand
  and FIFO-evicted. No silent truncation: partial results are marked.
- `bytes` that the server did not report render as "unknown", never `0 B`.
- Read-only connections block non-read SQL in the UI **and** the sidecar.

## Build

```bash
npm install
npm run build     # → ../ui/index.html
```

## Developing without the DBX host

`dev-harness.mjs` serves `ui/index.html` with a `window.dbxPlugin` shim that
forwards `invoke`/events/storage to a **real sidecar** (release build), which
talks to a real QSDB:

```bash
cargo build --release --manifest-path ../backend/Cargo.toml
node dev-harness.mjs                       # anonymous → 127.0.0.1:9430
QSDB_DEV_AUTH=basic QSDB_DEV_USER=root QSDB_DEV_PASSWORD=… \
QSDB_DEV_DB=default QSDB_DEV_URL=your-qsdb-host:9430 node dev-harness.mjs
```

Then open <http://localhost:5191>. The harness is a dev-only tool and is never
packaged.
