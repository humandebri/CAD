/**
 * apps/viewer: verifies the Phase 0 shell renders in a browser.
 * Later phases extend this around SVG, diff, and check-result panels.
 */
import { expect, test } from "@playwright/test";

test("renders the scaffold viewer", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("CAD Viewer")).toBeVisible();
  await expect(page.getByText("Phase 0 scaffold")).toBeVisible();
});
