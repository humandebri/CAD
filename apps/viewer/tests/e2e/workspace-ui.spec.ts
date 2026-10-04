import { expect, test } from "@playwright/test";

test("navigator tabs support keyboard navigation and retain layer filters", async ({ page }) => {
  await page.goto("/");
  const review = page.getByRole("tab", { name: "Layers & review" });
  const project = page.getByRole("tab", { name: "Project", exact: true });
  const filter = page.getByRole("searchbox", { name: "Filter layers" });
  await filter.fill("missing");
  await review.focus();
  await page.keyboard.press("ArrowRight");
  await expect(project).toBeFocused();
  await expect(project).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tabpanel", { name: "Project" })).toBeVisible();
  await expect(filter).toBeHidden();
  await page.keyboard.press("Home");
  await expect(review).toBeFocused();
  await expect(filter).toHaveValue("missing");
  await expect(page.getByRole("button", { name: "Hide 0-1", exact: true })).toHaveCount(0);
});

test("focused drawing workspace retains zoom, selection and panel state", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet", exact: true }).click();
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  const entity = page.locator('.drawing-stage [data-entity-id="ent_01JZ0000000000000000000000"]').first();
  const beforeFocus = await page.locator(".svg-surface > svg").getAttribute("viewBox");
  await page.getByText("ent_01JZ0000000000000000000000", { exact: true }).first().click();
  await expect(page.locator(".svg-surface > svg")).not.toHaveAttribute("viewBox", beforeFocus!);
  const viewBox = await page.locator(".svg-surface > svg").getAttribute("viewBox");
  const before = await page.locator(".drawing-stage").boundingBox();
  await page.getByRole("button", { name: "Toggle navigator", exact: true }).click();
  await page.getByRole("button", { name: "Toggle workspace", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "Review results" })).toBeHidden();
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toBeHidden();
  const focused = await page.locator(".drawing-stage").boundingBox();
  expect(focused!.width).toBeGreaterThan(before!.width + 500);
  await expect(page.locator(".svg-surface > svg")).toHaveAttribute("viewBox", viewBox!);
  await expect(entity).toHaveClass(/is-selected/);
  await page.getByRole("button", { name: "Toggle navigator", exact: true }).click();
  await page.getByRole("button", { name: "Toggle workspace", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText("geometry_changed");
});

test("CAD shortcuts work after toolbar and tab focus without stealing button activation", async ({ page }) => {
  await page.goto("/");
  const zoom = page.getByRole("button", { name: "Zoom in", exact: true });
  const prompt = page.locator(".command-prompt");
  await zoom.click();
  await expect(zoom).toBeFocused();
  await page.keyboard.press("l");
  await expect(prompt).toHaveText("Line");
  await page.keyboard.press("Meta+z");
  await expect(prompt).toHaveText("Undo");
  await page.keyboard.press("Control+Shift+z");
  await expect(prompt).toHaveText("Redo");
  await page.keyboard.press("Delete");
  await expect(prompt).toHaveText("Delete");
  const tab = page.getByRole("tab", { name: "Layers & review" });
  await tab.focus();
  await page.keyboard.press("l");
  await expect(prompt).toHaveText("Line");
  await zoom.focus();
  const svg = page.locator(".svg-surface > svg");
  const before = await svg.getAttribute("viewBox");
  await page.keyboard.press("Enter");
  await expect(svg).not.toHaveAttribute("viewBox", before!);
  await expect(prompt).toHaveText("Line");
  await page.getByRole("button", { name: "F3 SNAP", exact: true }).focus();
  const snap = page.getByRole("button", { name: "F3 SNAP", exact: true });
  const pressed = await snap.getAttribute("aria-pressed");
  await page.keyboard.press("Space");
  await expect(snap).toHaveAttribute("aria-pressed", pressed === "true" ? "false" : "true");
  await expect(prompt).toHaveText("Line");
});

test("Desktop minimum viewport keeps the drawing and command bar visible", async ({ page }) => {
  await page.setViewportSize({ width: 980, height: 700 });
  await page.goto("/");
  for (const selector of [".drawing-stage", ".command-bar", ".command-bar input"]) {
    const bounds = await page.locator(selector).boundingBox();
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(700);
  }
  expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeLessThanOrEqual(700);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(980);
});

for (const width of [375, 768, 1024, 1440]) {
  test(`workspace fits ${width}px and keeps drawing and command controls usable`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");
    const stage = await page.locator(".drawing-stage").boundingBox();
    expect(stage!.width).toBeGreaterThan(width >= 768 ? 400 : 300);
    expect(stage!.height).toBeGreaterThan(300);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    for (const name of ["F3 SNAP", "F8 ORTHO"]) {
      const control = page.getByRole("button", { name, exact: true });
      await control.scrollIntoViewIfNeeded();
      await control.focus();
      const before = await control.getAttribute("aria-pressed");
      await page.keyboard.press("Space");
      await expect(control).toHaveAttribute("aria-pressed", before === "true" ? "false" : "true");
    }
  });
}

test("modal confines focus, dismisses with Escape and restores its launcher", async ({ page }) => {
  await page.goto("/tests/fixtures/workspace-ui.html");
  const launch = page.getByRole("button", { name: "New Project", exact: true });
  await launch.click();
  await expect(page.getByRole("textbox", { name: "Project name", exact: true })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(page.getByRole("button", { name: "Close", exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("textbox", { name: "Project name", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(launch).toBeFocused();
});

test("canvas keyboard navigation pans, restores view and skips hidden geometry", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet", exact: true }).click();
  const canvas = page.getByRole("group", { name: "Drawing canvas", exact: true });
  const svg = page.locator(".svg-surface > svg");
  const original = await svg.getAttribute("viewBox");
  await canvas.focus();
  await page.keyboard.press("ArrowRight");
  await expect(svg).not.toHaveAttribute("viewBox", original!);
  await page.keyboard.press("Home");
  await expect(svg).toHaveAttribute("viewBox", original!);
  await page.keyboard.press("PageDown");
  await expect(page.locator(".drawing-stage .is-selected")).not.toHaveCount(0);
  await page.getByRole("button", { name: "Hide 0-1", exact: true }).click();
  await canvas.focus();
  await page.keyboard.press("PageDown");
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText("Select an entity on the paper");
});

test("canvas selection starts at the last entity backwards and wraps both ways", async ({ page }) => {
  const ids = [0, 1, 2].map(index => `ent_01JZ000000000000000000000${index}`);
  await page.route("**/build/plan_1f.svg", route => route.fulfill({
    contentType: "image/svg+xml",
    body: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">${ids.map((id, index) =>
      `<line data-entity-id="${id}" data-layer="0-1" x1="10" y1="${10 + index * 20}" x2="90" y2="${10 + index * 20}" stroke="black"/>`).join("")}</svg>`,
  }));
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet", exact: true }).click();
  const canvas = page.getByRole("group", { name: "Drawing canvas", exact: true });
  const selected = page.locator(".drawing-stage .is-selected");
  await canvas.focus();
  await page.keyboard.press("PageUp");
  await expect(selected).toHaveAttribute("data-entity-id", ids[2]);
  await page.keyboard.press("PageDown");
  await expect(selected).toHaveAttribute("data-entity-id", ids[0]);
  await page.keyboard.press("PageUp");
  await expect(selected).toHaveAttribute("data-entity-id", ids[2]);
  await page.reload();
  await page.getByRole("button", { name: "Sheet", exact: true }).click();
  await canvas.focus();
  await page.keyboard.press("PageDown");
  await expect(selected).toHaveAttribute("data-entity-id", ids[0]);
});
