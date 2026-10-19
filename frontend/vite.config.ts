import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { viteSingleFile } from "vite-plugin-singlefile";

// 产物 = ui/index.html（单文件：JS/CSS 内联）。
// Workbench 跑在 sandbox iframe（CSP `default-src 'none'`）里，单文件避免
// 对 module 脚本/MIME 的依赖；DBX 只要求 ui/ 是纯静态资源，无 dev server、
// 无 CDN。manifest.json 的 `entrypoints.ui` 指向 ui/index.html。
export default defineConfig({
  plugins: [vue(), viteSingleFile()],
  base: "./",
  build: {
    outDir: "../ui",
    emptyOutDir: true,
    target: "es2022",
    reportCompressedSize: false,
  },
});
