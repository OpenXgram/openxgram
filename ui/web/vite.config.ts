import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import path from "node:path";
import { readFileSync } from "node:fs";

// CLAUDE.md 룰 6 — 프론트 한 곳 버전 표시. package.json + git short SHA 주입.
const pkg = JSON.parse(readFileSync(path.resolve(__dirname, "package.json"), "utf-8"));
const APP_VERSION = pkg.version as string;
const BUILD_TIME = new Date().toISOString();

// Web GUI 빌드 — 정적 호스팅 (nginx /gui/) 전용.
//   - Tauri-specific 옵션 (envPrefix TAURI_ENV_, port 1420) 제거.
//   - base "/gui/" — nginx alias /gui/ → /var/www/openxgram-gui/ 와 매칭.
//   - "@/*" alias = src/* (api/client.ts 등 절대 경로 import).
export default defineConfig({
  plugins: [
    solid(),
    {
      // Vite `define` 가 solid JSX transform 와 충돌해서 __APP_VERSION__ inline
      // 치환이 안 됨. 대신 index.html head 에 직접 globalThis 주입 — App.tsx의
      // `__APP_VERSION__` 참조가 글로벌로 resolve 되어 정상 동작.
      name: "inject-build-globals",
      transformIndexHtml(html) {
        return html.replace(
          "</head>",
          `<script>globalThis.__APP_VERSION__=${JSON.stringify(APP_VERSION)};globalThis.__BUILD_TIME__=${JSON.stringify(BUILD_TIME)};</script></head>`,
        );
      },
    },
  ],
  base: "/gui/",
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(APP_VERSION),
    __BUILD_TIME__: JSON.stringify(BUILD_TIME),
  },
  server: {
    port: 5173,
    host: "0.0.0.0",
    allowedHosts: ["server-seoul.tail0957ca.ts.net"],
    hmr: {
      protocol: "wss",
      host: "server-seoul.tail0957ca.ts.net",
      clientPort: 443,
    },
    proxy: {
      "/v1": {
        target: "http://100.101.237.9:47302",
        changeOrigin: false,
        ws: true,
      },
    },
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
