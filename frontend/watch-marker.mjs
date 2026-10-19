#!/usr/bin/env node
// vite build --watch 的包装：官方开发宿主（dbx-plugin dev）约定——只有一次
// 完整成功构建后输出独立一行 DBX_UI_BUILD_SUCCESS 才会重载 UI；失败路径
// 与退出钩子绝不输出该标记（见官方文档）。
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const child = spawn("npx", ["vite", "build", "--watch"], {
  cwd: path.dirname(fileURLToPath(import.meta.url)),
  shell: process.platform === "win32",
  stdio: ["ignore", "pipe", "inherit"],
});

let buffer = "";
child.stdout.on("data", (chunk) => {
  buffer += chunk.toString();
  process.stdout.write(chunk);
  // vite 每次成功重建都以 "built in ..." 收尾
  if (/\bbuilt in\b/.test(buffer)) {
    if (!/\berror\b/i.test(buffer)) {
      process.stdout.write("\nDBX_UI_BUILD_SUCCESS\n");
    }
    buffer = "";
  }
});
child.on("exit", (code) => process.exit(code ?? 0));
