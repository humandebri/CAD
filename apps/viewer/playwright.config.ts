/**
 * apps/viewer: Playwright smoke configuration for the static viewer.
 * It starts Vite locally and verifies the scaffold page renders.
 */
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.CAD_E2E_PORT ?? 5173);
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./tests/e2e",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  webServer: {
    command: `pnpm dev --port ${port} --strictPort`,
    url: baseURL,
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
