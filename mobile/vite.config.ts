// iPhone 앱 화면. 저장소 루트에서 `vite build --config mobile/vite.config.ts`로 빌드한다(src-tauri/tauri.conf.json)
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
// @ts-expect-error type error without @types/node package
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1422 } : undefined,
    // 데스크톱 화면의 공용 코드(../src/markdown.ts 등)를 읽는다
    fs: { allow: [".."] },
    watch: { ignored: ["**/src-tauri/**", "**/plugin/**"] },
  },
  build: { outDir: "dist", emptyOutDir: true },
});
