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
  await page.locator(".drawing-stage svg text").first().click();
  await expect(
    page.locator('.drawing-stage [data-entity-id="ent_01JZ0000000000000000000001"].is-selected'),
  ).toHaveCount(1);

  await page.getByRole("button", { name: "Sheet" }).click();
  await expect(page.locator(".drawing-stage svg [data-entity-id]").first()).toBeAttached();

  await page.getByRole("button", { name: "Diff" }).click();
  const svg = page.locator(".drawing-stage svg");
  const initialViewBox = await svg.getAttribute("viewBox");
  await page.getByText("ent_01JZ0000000000000000000000").first().click();

  await expect.poll(async () => svg.getAttribute("viewBox")).not.toBe(initialViewBox);
  await expect(
    page.locator('.drawing-stage [data-entity-id="ent_01JZ0000000000000000000000"].is-selected'),
  ).toHaveCount(2);
  const selectedLine = page.locator(".drawing-stage .is-selected line").first();
  await expect.poll(() => computedSvgStyle(selectedLine, "strokeWidth")).toBe("2px");
  await expect.poll(() => computedSvgStyle(selectedLine, "vectorEffect")).toBe(
    "non-scaling-stroke",
  );
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText(
    "geometry_changed",
  );
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText(
    "Phase 0 sample comment",
  );
});

test("layer workspace hides and restores visible geometry", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet" }).click();
  const entity = page.locator('.drawing-stage [data-layer="0-1"]').first();
  await expect(entity).toBeAttached();
  await expect.poll(() => entity.evaluate((element) => (element as SVGElement).style.display)).toBe("");
  await page.getByRole("button", { name: "Hide 0-1" }).click();
  await expect.poll(() => entity.evaluate((element) => (element as SVGElement).style.display)).toBe("none");
  await page.getByRole("button", { name: "Show 0-1" }).click();
  await expect.poll(() => entity.evaluate((element) => (element as SVGElement).style.display)).toBe("");
  await page.getByRole("searchbox", { name: "Filter layers" }).fill("missing");
  await expect(page.getByRole("button", { name: "Hide 0-1" })).toHaveCount(0);
});

test("Shift-click toggles selection without becoming a zero-area drag", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet" }).click();
  const first = page.locator('.drawing-stage [data-entity-id="ent_01JZ0000000000000000000000"]').first();
  const clickLine = async (shift: boolean) => {
    const point = await first.locator("line").evaluate((line) => {
      const box = line.getBoundingClientRect();
      return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
    });
    if (shift) await page.keyboard.down("Shift");
    await page.mouse.click(point.x, point.y);
    if (shift) await page.keyboard.up("Shift");
  };

  await clickLine(false);
  await expect(first).toHaveClass(/is-selected/);
  await clickLine(true);
  await expect(first).not.toHaveClass(/is-selected/);
  await page.waitForTimeout(300);
  await clickLine(true);
  await expect(first).toHaveClass(/is-selected/);
  await expect(page.locator(".zoom-area-rect")).toHaveCount(0);
});

for (const crossing of [false]) {
  test(`area selection excludes hidden layers and groups (${crossing ? "crossing" : "window"})`, async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Sheet" }).click();
    const entity = page.locator('.drawing-stage [data-entity-id="ent_01JZ0000000000000000000000"]').first();
    const selectArea = async () => {
      await expect(page.locator(".drawing-stage svg")).toBeVisible();
      const stage = page.locator(".drawing-stage");
      await stage.scrollIntoViewIfNeeded();
      const box = await stage.boundingBox();
      if (box === null) throw new Error("missing drawing stage");
      const left = box.x + 20;
      const right = box.x + box.width - 20;
      await page.keyboard.down("Shift");
      await page.mouse.move(crossing ? right : left, box.y + 20);
      await page.mouse.down();
      await page.mouse.move(crossing ? left : right, box.y + box.height - 20, { steps: 4 });
      await expect(page.locator(".zoom-area-rect")).toBeVisible();
      await page.mouse.up();
      await page.keyboard.up("Shift");
      await expect(page.locator(".zoom-area-rect")).toHaveCount(0);
    };
    await selectArea();
    await expect(entity).toHaveClass(/is-selected/);
    await page.getByRole("button", { name: "Hide 0-1", exact: true }).click();
    await selectArea();
    await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText("Select an entity on the paper");
    await expect(entity).not.toHaveClass(/is-selected/);
    await page.getByRole("button", { name: "Show 0-1", exact: true }).click();
    await selectArea();
    await expect(entity).toHaveClass(/is-selected/);
    const hideGroup = page.getByRole("button", { name: /^Hide group / });
    await hideGroup.click();
    await selectArea();
    await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText("Select an entity on the paper");
    await expect(entity).not.toHaveClass(/is-selected/);
    await page.getByRole("button", { name: /^Show group / }).click();
    await selectArea();
    await expect(entity).toHaveClass(/is-selected/);
  });
}

test("window excludes a partially intersected entity while crossing selects it", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet" }).click();
  const entity = page.locator('.drawing-stage [data-entity-id="ent_01JZ0000000000000000000000"]').first();
  const line = await entity.locator("line").boundingBox();
  if (!line || line.width <= 0) throw new Error("expected a horizontal line");
  const left = line.x - 3, right = line.x + line.width / 2;
  for (const crossing of [false, true]) {
    await page.keyboard.down("Shift");
    await page.mouse.move(crossing ? right : left, line.y - 8);
    await page.mouse.down();
    await page.mouse.move(crossing ? left : right, line.y + 8, { steps: 4 });
    await page.mouse.up();
    await page.keyboard.up("Shift");
    if (crossing) await expect(entity).toHaveClass(/is-selected/);
    else await expect(entity).not.toHaveClass(/is-selected/);
  }
});

test("wheel zooms beyond 800 percent and previous view restores the burst", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet" }).click();

  const svg = page.locator(".drawing-stage svg");
  const stage = page.locator(".drawing-stage");
  const stageBox = await stage.boundingBox();
  if (stageBox === null) {
    throw new Error("drawing stage has no bounding box");
  }
  const initialWidth = viewBoxWidth(await svg.getAttribute("viewBox"));
  const line = page.locator(".drawing-stage svg line").first();
  const initialStrokeWidth = await computedSvgStyle(line, "strokeWidth");
  expect(await computedSvgStyle(line, "vectorEffect")).toBe("non-scaling-stroke");
  await page.mouse.move(stageBox.x + stageBox.width / 2, stageBox.y + stageBox.height / 2);
  for (let index = 0; index < 7; index += 1) {
    await page.mouse.wheel(0, -1000);
  }

  await expect.poll(async () => viewBoxWidth(await svg.getAttribute("viewBox"))).toBeLessThan(
    initialWidth / 100,
  );
  await expect(page.locator("output.zoom-readout")).toContainText("x");
  await expect.poll(() => computedSvgStyle(line, "strokeWidth")).toBe(initialStrokeWidth);
  await expect.poll(() => computedSvgStyle(line, "vectorEffect")).toBe("non-scaling-stroke");
  await page.waitForTimeout(200);
  await page.getByRole("button", { name: "Previous view" }).click();
  await expect
    .poll(async () => viewBoxWidth(await svg.getAttribute("viewBox")))
    .toBeCloseTo(initialWidth, 6);
});

test("toolbar area zoom draws a marquee without selecting an entity", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet" }).click();
  await page.getByRole("button", { name: "Zoom area" }).click();

  const stage = page.locator(".drawing-stage");
  const svg = page.locator(".drawing-stage svg");
  const stageBox = await stage.boundingBox();
  if (stageBox === null) {
    throw new Error("drawing stage has no bounding box");
  }
  const initialWidth = viewBoxWidth(await svg.getAttribute("viewBox"));
  const start = { x: stageBox.x + stageBox.width * 0.3, y: stageBox.y + stageBox.height * 0.3 };
  const end = { x: stageBox.x + stageBox.width * 0.7, y: stageBox.y + stageBox.height * 0.7 };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 4 });
  await expect(page.locator(".zoom-area-rect")).toBeVisible();
  await page.mouse.up();

  await expect.poll(async () => viewBoxWidth(await svg.getAttribute("viewBox"))).toBeLessThan(
    initialWidth,
  );
  await expect(page.getByRole("button", { name: "Zoom area" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await expect(page.getByRole("complementary", { name: "Selected entity" })).toContainText(
    "Select an entity",
  );

  await page.getByRole("button", { name: "Zoom area" }).click();
  await expect(page.getByRole("button", { name: "Zoom area" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Zoom area" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
});

test("Jw_cad two-button diagonals zoom an area and restore the previous view", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Sheet" }).click();

  const stage = page.locator(".drawing-stage");
  const svg = page.locator(".drawing-stage svg");
  const stageBox = await stage.boundingBox();
  if (stageBox === null) {
    throw new Error("drawing stage has no bounding box");
  }
  const initialWidth = viewBoxWidth(await svg.getAttribute("viewBox"));
  const start = { x: stageBox.x + stageBox.width * 0.25, y: stageBox.y + stageBox.height * 0.25 };
  const end = { x: stageBox.x + stageBox.width * 0.65, y: stageBox.y + stageBox.height * 0.65 };
  await jwCadDrag(page, start, end);
  await expect.poll(async () => viewBoxWidth(await svg.getAttribute("viewBox"))).toBeLessThan(
    initialWidth,
  );

  const previousStart = {
    x: stageBox.x + stageBox.width * 0.6,
    y: stageBox.y + stageBox.height * 0.4,
  };
  const previousEnd = { x: previousStart.x - 80, y: previousStart.y + 80 };
  await jwCadDrag(page, previousStart, previousEnd);
  await expect
    .poll(async () => viewBoxWidth(await svg.getAttribute("viewBox")))
    .toBeCloseTo(initialWidth, 6);
});

async function jwCadDrag(
  page: import("@playwright/test").Page,
  start: { x: number; y: number },
  end: { x: number; y: number },
) {
  await page.mouse.move(start.x, start.y);
  await page.mouse.down({ button: "left" });
  await page.mouse.down({ button: "right" });
  await page.mouse.move(end.x, end.y, { steps: 4 });
  await page.mouse.up({ button: "right" });
  await page.mouse.up({ button: "left" });
}

function viewBoxWidth(value: string | null): number {
  if (value === null) {
    throw new Error("SVG viewBox is missing");
  }
  const width = Number(value.trim().split(/[\s,]+/)[2]);
  if (!Number.isFinite(width) || width <= 0) {
    throw new Error(`invalid SVG viewBox: ${value}`);
  }
  return width;
}

async function computedSvgStyle(
  locator: import("@playwright/test").Locator,
  property: "strokeWidth" | "vectorEffect",
): Promise<string> {
  return locator.evaluate((element, styleProperty) => getComputedStyle(element)[styleProperty], property);
}
