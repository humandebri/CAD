import { test, expect } from "@playwright/test";

test("scale parameters reject zero and preserve the selected base point", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/tests/fixtures/scale.html");
  const factor = page.getByRole("spinbutton", { name: "Scale factor" });
  const apply = page.getByRole("button", { name: "Apply", exact: true });
  await factor.fill("0");
  await expect(apply).toBeDisabled();
  await expect(page.locator("#result")).toHaveText("Selection unchanged.");
  await factor.fill("-1");
  await expect(apply).toBeDisabled();
  await factor.fill("");
  await expect(apply).toBeDisabled();
  await factor.fill("0.5");
  await apply.focus();
  await page.keyboard.press("Enter");
  expect(JSON.parse(await page.locator("#result").textContent() || "{}")).toMatchObject({
    kind: "scale", center: [20000, 20000], factor: 0.5,
    entity_ids: ["ent_01JZ0000000000000000000000"],
  });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator("#result")).toHaveText("Cancelled; selection unchanged.");
});
