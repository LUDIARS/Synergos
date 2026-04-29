import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 2 dev server: dev URL は tauri.conf.json と一致させる必要がある (1420)
export default defineConfig(async () => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: false,
  },
}));
