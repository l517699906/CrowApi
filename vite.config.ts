import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// E2E runs can opt into an isolated Vite port without changing normal development defaults.
// @ts-expect-error process is a nodejs global
const configuredPort = Number(process.env.TAURI_E2E_PORT ?? 1422);
// @ts-expect-error process is a nodejs global
const configuredHmrPort = Number(process.env.TAURI_E2E_HMR_PORT ?? configuredPort + 1);

function validPort(value: number, fallback: number) {
  return Number.isInteger(value) && value >= 1024 && value <= 65_535 ? value : fallback;
}

const port = validPort(configuredPort, 1422);
const hmrPort = validPort(configuredHmrPort, port + 1);

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    // Keep lazy routes and React Query on the same React instance, including
    // after Vite refreshes its optimized dependency graph during development.
    dedupe: ["react", "react-dom"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: hmrPort,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
