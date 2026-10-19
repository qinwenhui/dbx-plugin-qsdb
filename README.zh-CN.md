# dbx-plugin-qsdb

[DBX](https://github.com/t8y2/dbx) 插件：为 [QSDB](https://github.com/qinwenhui/qsdb)
提供原生工作台 —— schema 浏览、分析 SQL、大结果分页、查询停止、分区与表大小。
· [English](README.md)

- **插件 id：** `cn.qinwh.qsdb` · **连接类型：** `qsdb`
- **要求：** DBX `>=0.6.0`（Host API `^1.1`），QSDB `0.1.x`
- **许可证：** Apache-2.0（本插件）；`qsdb-client` 为 MIT

## 为什么不用 JDBC？

DBX 的自定义 JDBC 驱动能最快打通基本 SQL 访问；本插件提供的是一等 QSDB 体验：

- QSDB 原生的库/挂载模型
- 分区 generation 与表字节级检查
- 两段式查询停止（执行段 cancel + 传输段 abort）
- 大结果游标分页，而不是一次性巨型 JSON
- 为 QSDB 设计的工作台，而非通用 JDBC 表格

插件通过 Rust Sidecar 访问 QSDB HTTP API，不实现第二套线协议。

## 功能

- **原生 schema 浏览** —— 库/挂载 → 表 → 列，支持过滤、刷新、一键预览
- **SQL 编辑器** —— CodeMirror 6，运行/停止，历史与草稿存 DBX 插件存储
- **流式结果** —— 读查询流式落盘 spool，按页浏览（默认 500 行、单页 ≤1 MiB）；
  十万行级结果依然流畅，绝不静默截断
- **两段式停止** —— 服务端 cancel（执行段）+ 传输中断开（流式段）；
  已收到的行仍可继续浏览
- **分区面板** —— 每分区 generation / rows / segments / bytes 与表大小汇总；
  服务端未上报 bytes 时显示"未知"，绝不伪造 `0 B`
- **类型化值** —— NULL、超出 JS 安全整数的大整数、十进制、二进制（hex）、
  日期/时间类型均按 QSDB wire 契约渲染
- **认证** —— 匿名 / Basic / Bearer；凭证只进 DBX Secret Store，
  不出现在日志、事件或工作台上下文
- **只读连接** UI 与 Sidecar 双层拦截
- **跟随宿主** —— 明暗主题、中英文界面

## 架构

```text
DBX
├── 连接表单（host / port / 认证 / 默认库）
├── QSDB 工作台（Vue 3 静态资源，沙箱 iframe）
└── Rust Sidecar（stdio-jsonl JSON-RPC）
        └── qsdb-client → QSDB HTTP API（runtime.host:runtime.port）
```

QSDB 的全部网络 I/O 在 Sidecar 内完成，并经生命周期载荷的
`runtime.host` / `runtime.port`（DBX 隧道/代理后的最终端点）连接。
沙箱 UI 无任何直接网络访问，仅通过 DBX 插件桥调用 Sidecar。
密码/Token 经 DBX `password`/`secret` binding 落入 Secret Store。

## 安装

**DBX Store：** 上架审核中，之后可直接搜索 “QSDB”。

**Release 附件：** 从
[Releases](https://github.com/qinwenhui/dbx-plugin-qsdb/releases)
下载 `.dbxp`，在 DBX 插件管理器中安装。

**源码构建：**

```bash
git clone https://github.com/qinwenhui/dbx-plugin-qsdb
cd dbx-plugin-qsdb

# 1. Sidecar
cargo build --release --manifest-path backend/Cargo.toml

# 2. 工作台 UI（产物 ui/index.html）
cd frontend && npm install && npm run build && cd ..

# 3. 打包（需 DBX 插件 CLI：npm i -g @dbx-app/plugin-cli）
dbx-plugin package .
```

## 开发

```bash
cd backend  && cargo test     # 单元 + 端到端测试（本地 mock QSDB）
cd frontend && npm run build  # 构建工作台
node frontend/dev-harness.mjs # 浏览器里直连真实 Sidecar + 真实 QSDB 调试 UI
                              # （见 frontend/README.md）
dbx-plugin dev --path . --port 5190
```

`backend/Cargo.toml` 的 `version` 需与 `manifest.json` 保持一致。

## 安全说明

- QSDB 是明文 HTTP/1.1；需要加密时请在 QSDB 前挂 TLS 终止代理，
  本插件不伪造加密。
- 权限仅申请 `host.workbench`、`host.events`、`host.storage`。
- 只读模式在 UI 与 Sidecar 双层拦截；服务端授权仍是最终边界。
- 本插件为独立集成，Apache-2.0；与 QSDB Studio 无关，不含其代码。

## 许可证

Apache License 2.0 —— 见 [LICENSE](LICENSE)。
