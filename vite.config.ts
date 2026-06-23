import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the built frontend from the dist dir in production and proxies
// to this dev server during `cargo tauri dev`.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 65173,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
