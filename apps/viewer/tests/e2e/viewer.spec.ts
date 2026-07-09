/**
 * apps/viewer: verifies generated SVG, diff JSON, check JSON, and comments
 * are usable from the review interface.
 */
import { expect, test } from "@playwright/test";

test("reviews generated artifacts", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByText("CAD Review")).toBeVisible();
  await expect(page.locator(".drawing-stage svg")).toBeVisible();
  await expect(page.getByText("line_width_changed")).toBeVisible();

  await page.getByRole("button", { name: "Sheet" }).click();
  await expect(page.locator(".drawing-stage svg [data-entity-id]").first()).toBeAttached();

  await page.getByRole("button", { name: "Diff" }).click();
  await page.getByText("ent_01JZ0000000000000000000000").first().click();

  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText(
    "geometry_changed",
  );
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText(
    "Phase 0 sample comment",
  );
});
