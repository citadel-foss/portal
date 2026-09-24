import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

const resolve = (path: string) => fileURLToPath(new URL(path, import.meta.url));

/**
 * One React app, two hosts. `--mode web` swaps the host adapter and the output directory;
 * everything else — pages, components, stores — is shared and compiled twice.
 */
export default defineConfig(async ({ mode }) => {
  const web = mode === "web";
  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        // The single seam between the app and its host. Aliasing rather than branching at
        // runtime is what keeps `@tauri-apps/*` out of the web bundle entirely.
        "@host": web ? resolve("./src/api/web.ts") : resolve("./src/api/desktop.ts"),
      },
    },
    build: {
      // Siblings under one parent so `dist/` stays a single ignored directory.
      outDir: web ? "dist/web" : "dist/desktop",
      emptyOutDir: true,
    },
    clearScreen: false,
    server: {
      // Tauri pins 1420 as its devUrl, so the web target takes its own port. Otherwise a
      // stale web dev server blocks `npm run tauri dev`, and the two can never run together.
      port: web ? 1430 : 1420,
      strictPort: true,
      host: host || false,
      hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
      watch: {
        ignored: ["**/src-tauri/**", "**/core/**", "**/src-web/**", "**/target/**", "**/dist/**"],
      },
      // Development runs behind Vite's origin so the browser sees one origin for assets and
      // API alike; cookies, CSRF and SSE then behave as they do in production.
      proxy: web
        ? {
            "/api": {
              target: "http://127.0.0.1:3000",
              changeOrigin: false,
              // SSE must stream rather than buffer.
              ws: false,
            },
          }
        : undefined,
    },
  };
});
