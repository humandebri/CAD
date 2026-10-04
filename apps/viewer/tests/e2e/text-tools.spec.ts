import { test, expect } from "@playwright/test";

test("upright direction uses matching annotation scope, invalidates stale previews and keeps source untouched", async ({ page }) => {
  await page.setViewportSize({width:375,height:812});
  await page.goto("/tests/fixtures/text-tools.html");
  await page.getByLabel("Text search scope").selectOption("selection");
  await page.getByRole("button",{name:"Preview writing direction"}).click();
  const apply=page.getByRole("button",{name:"Apply text preview"});
  await expect(apply).toBeEnabled();
  await expect(page.locator("output")).toHaveText("Source unchanged.");
  await page.getByLabel("Batch text writing direction").selectOption("horizontal");
  await expect(apply).toBeDisabled();
  await expect(page.getByRole("button",{name:"Preview writing direction"})).toBeDisabled();
  await page.getByLabel("Batch text writing direction").selectOption("vertical_upright");
  await page.getByRole("button",{name:"Preview writing direction"}).click();
  await apply.focus();await page.keyboard.press("Enter");
  expect(JSON.parse(await page.locator("output").textContent()||"{}")).toEqual({kind:"set_writing_mode",entity_ids:["text-wall"],writing_mode:"vertical_upright"});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  await page.goto("/tests/fixtures/text-tools.html?empty");
  await page.getByLabel("Text search scope").selectOption("selection");
  await expect(page.getByRole("button",{name:"Preview writing direction"})).toBeDisabled();
});

test("literal text replacement previews the count and excludes dimension labels", async ({ page }) => {
  await page.goto("/tests/fixtures/text-tools.html");
  await page.getByLabel("Find annotation text").fill("既存");
  await page.getByLabel("Replacement text").fill("改修");
  const results = page.getByLabel("Matching annotation texts");
  await expect(results.locator("li")).toHaveCount(2);
  await expect(results).toContainText("Replacement: 改修壁と改修壁");
  await expect(results).not.toContainText("既存寸法");
  await page.getByRole("button", {name:"Preview text replacement"}).click();
  await expect(page.locator(".text-tools-panel p[role=status]")).toContainText("2 text entities in the preview");
  await expect(page.locator("output")).toHaveText("Source unchanged.");
  const apply = page.getByRole("button", {name:"Apply text preview"});
  await apply.focus(); await page.keyboard.press("Enter");
  expect(JSON.parse(await page.locator("output").textContent() || "{}")).toEqual({kind:"replace",find:"既存",replace:"改修",entity_ids:["text-wall","text-door"],style:null});
});

test("style-only edits retain text and selected scope rejects empty selections", async ({ page }) => {
  await page.setViewportSize({width:375,height:812});
  await page.goto("/tests/fixtures/text-tools.html");
  await page.getByLabel("Text search scope").selectOption("selection");
  await page.getByLabel("Batch text style").selectOption("title");
  await expect(page.getByLabel("Matching annotation texts").locator("li")).toHaveCount(1);
  await page.getByRole("button", {name:"Preview style change"}).click();
  const apply = page.getByRole("button", {name:"Apply text preview"});
  await expect(apply).toBeEnabled();
  await page.getByLabel("Find annotation text").fill("missing");
  await expect(page.getByLabel("Find annotation text")).toHaveValue("missing");
  await expect(apply).toBeDisabled();
  await expect(page.getByRole("button", {name:"Preview style change"})).toBeDisabled();
  await page.getByLabel("Find annotation text").fill("");
  await page.getByRole("button", {name:"Preview style change"}).click();
  await apply.click();
  expect(JSON.parse(await page.locator("output").textContent() || "{}")).toEqual({kind:"set_style",entity_ids:["text-wall"],style:"title"});
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  await page.goto("/tests/fixtures/text-tools.html?empty");
  await page.getByLabel("Text search scope").selectOption("selection");
  await page.getByLabel("Batch text style").selectOption("title");
  await expect(page.getByLabel("Matching annotation texts").locator("li")).toHaveCount(0);
  await expect(page.getByRole("button", {name:"Preview style change"})).toBeDisabled();
});

test("source revisions and rejected generation cannot leave an applicable text preview", async ({ page }) => {
  await page.goto("/tests/fixtures/text-tools.html?slow");
  await page.getByLabel("Batch text style").selectOption("title");
  await page.getByRole("button", {name:"Preview style change"}).click();
  await page.getByRole("button", {name:"Refresh sources"}).click();
  await expect(page.getByRole("button", {name:"Preview style change"})).toBeEnabled();
  await expect(page.getByRole("button", {name:"Apply text preview"})).toBeDisabled();
  await expect(page.locator("output")).toHaveText("Source unchanged.");
  await page.goto("/tests/fixtures/text-tools.html?failure");
  await page.getByLabel("Batch text style").selectOption("title");
  await page.getByRole("button", {name:"Preview style change"}).click();
  await expect(page.locator(".text-tools-panel p[role=status]")).toContainText("revision_conflict");
  await expect(page.getByRole("button", {name:"Apply text preview"})).toBeDisabled();
});
