import { test, expect } from "@playwright/test";

test("DXF export selects the drawing, strict policy and report and preserves keyboard navigation", async ({ page }) => {
  await page.goto("/tests/fixtures/dxf-exchange.html");
  await expect(page.getByLabel("DXF drawing")).toBeFocused();
  await page.getByLabel("DXF drawing").selectOption("detail");
  await page.getByRole("button", { name: "Choose destination" }).click();
  await expect(page.getByLabel("DXF report")).toHaveValue("/fixture/output.dxf.report.json");
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Export DXF", exact: true }).click();
  await expect(page.getByRole("dialog").getByRole("status")).toContainText("DXF and compatibility report saved");
  const result = JSON.parse(await page.locator("#result").textContent() || "{}");
  expect(result).toMatchObject({drawing:"detail",strict:true,report:"/fixture/output.dxf.report.json"});
  await expect(page.getByRole("dialog")).toContainText("reference relationships are not preserved");
  await page.getByRole("button", { name: "Close", exact: true }).focus();
  await page.keyboard.press("Tab");
  await expect(page.getByLabel("DXF drawing")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator("#open")).toBeFocused();
});

test("DXF import validates the unit override and exposes blocked records on a narrow screen", async ({ page }) => {
  await page.setViewportSize({width:375,height:812});
  await page.goto("/tests/fixtures/dxf-exchange.html?import&blocked");
  await page.getByRole("button", { name: "Choose DXF", exact: true }).click();
  await page.getByRole("button", { name: "Choose destination" }).click();
  await expect(page.getByLabel("DXF report")).toHaveValue("/fixture/new-project.import-report.json");
  await page.getByLabel("DXF unit override").fill("0");
  await page.getByRole("button", { name: "Import DXF", exact: true }).click();
  await expect(page.getByRole("dialog").getByRole("status")).toContainText("positive number");
  await expect(page.locator("#result")).toBeEmpty();
  await page.getByLabel("DXF unit override").fill("25.4");
  await page.getByRole("button", { name: "Import DXF", exact: true }).click();
  await expect(page.getByRole("dialog").getByRole("status")).toContainText("Conversion blocked");
  expect(JSON.parse(await page.locator("#result").textContent() || "{}").unitMm).toBe(25.4);
  await expect(page.getByRole("dialog")).toContainText("unsupported_record");
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});
