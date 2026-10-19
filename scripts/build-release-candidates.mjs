#!/usr/bin/env node
// 汇总 dist/ 下的 *.artifact.json 为商店同步器读取的 release-candidates.json
// （结构见官方文档：schemaVersion/id/publisher/version/name/source/license/targets）。
//   node scripts/build-release-candidates.mjs <version> <base-url> <dist-dir>
// 用法（CI 里）：
//   node scripts/build-release-candidates.mjs 0.1.0 \
//     https://github.com/<owner>/<repo>/releases/download/v0.1.0 dist
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

const [, , version, baseUrl, distDir = "dist"] = process.argv;
if (!version || !baseUrl) {
  console.error("usage: build-release-candidates.mjs <version> <base-url> [dist-dir]");
  process.exit(1);
}

const manifest = JSON.parse(readFileSync("manifest.json", "utf8"));
const targets = readdirSync(distDir)
  .filter((f) => f.endsWith(".artifact.json"))
  .sort()
  .map((f) => {
    const a = JSON.parse(readFileSync(path.join(distDir, f), "utf8"));
    const target = a.target ?? path.basename(f).replace(/\.artifact\.json$/, "");
    // artifact.json 的 url 是包文件名（打包目录内相对路径）；换成 Release 资产地址
    const dbxp = path.basename(a.url ?? path.basename(f).replace(/\.artifact\.json$/, ".dbxp"));
    return {
      target,
      url: `${baseUrl.replace(/\/$/, "")}/${dbxp}`,
      sha256: a.sha256,
      size: a.size,
    };
  });

if (targets.length === 0) {
  console.error("no *.artifact.json found in dist");
  process.exit(1);
}

const out = {
  schemaVersion: 1,
  id: manifest.id,
  publisher: manifest.publisher,
  version: manifest.version,
  name: manifest.name,
  source: `${(manifest.source ?? "").replace(/\/$/, "")}/tree/v${version}`,
  license: "Apache-2.0",
  targets,
};
process.stdout.write(JSON.stringify(out, null, 2) + "\n");
