/**
 * apps/viewer: unit-test configuration that avoids starting the Vite server.
 * It covers TypeScript-only loader contracts used by the desktop bridge.
 */
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/unit",
});
