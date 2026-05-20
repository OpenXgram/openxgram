import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import path from "node:path";

// Web GUI 빌드 — 정적 호스팅 (nginx /gui/) 전용.
//   - Tauri-specific 옵션 (envPrefix TAURI_ENV_, port 1420) 제거.
//   - base "/gui/" — nginx alias /gui/ → /var/www/openxgram-gui/ 와 매칭.
//   - "@/*" alias = src/* (api/client.ts 등 절대 경로 import).
export default defineConfig({
  plugins: [solid()],
  base: "/gui/",
  clearScreen: false,
  server: {
    port: 5173,
    host: "127.0.0.1",
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "src"),
    },
  },
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: false,
    outDir: "dist",
    // rust-embed (xgram daemon GUI 임베드) 가 base64 hash 의 trailing `-`
    // 를 처리 못 해 rustc ICE → hex 강제.
    rollupOptions: {
      output: {
        hashCharacters: "hex",
      },
    },
  },
});
