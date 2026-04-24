import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],

  // Prevent Vite from printing the URL — Tauri handles the window.
  clearScreen: false,

  server: {
    port: 1420,
    strictPort: true,
    // Don't watch src-tauri — Rust rebuilds are handled by the Tauri CLI.
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },

  envPrefix: ["VITE_", "TAURI_ENV_*"],

  build: {
    // Tauri supports Chromium 105+.
    target: "chrome105",
    // Keep sourcemaps only in debug builds.
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    minify: !process.env.TAURI_ENV_DEBUG ? ("esbuild" as const) : false,
  },
});
