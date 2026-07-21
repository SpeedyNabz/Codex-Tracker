import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // Keep CSS-referenced SVGs as bundle assets. Tauri's production CSP does
  // not allow the data URLs Vite would otherwise generate for these icons.
  build: {
    assetsInlineLimit: 0,
  },
  clearScreen: false,
  server: {
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    css: true,
    exclude: ["**/node_modules/**", "**/dist/**", "playwright/**"],
  },
});
