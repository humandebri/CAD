/**
 * apps/viewer: Vite configuration for the Phase 0 Preact shell.
 * Later phases keep this app static and read generated build artifacts.
 */
import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
});
