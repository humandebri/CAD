import { expect, test, type Page } from "@playwright/test";

// Browser-only tests assert the UI/IPC contract. Real source persistence is covered by Desktop E2E.
async function openDraftingHarness(page: Page) {
  await page.addInitScript(() => {
    const host = window as unknown as Record<string, unknown>;
    const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
    host.__CAD_TEST_CALLS__ = calls;
    const entity = { schema_version: "0.3", id: "ent_01JZ0000000000000000000000", type: "line", layer: "0-1", p1: [10, 10], p2: [110, 10] };
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -150 200 200"><g data-entity-id="ent_01JZ0000000000000000000000" data-layer="0-1" data-bbox="10,10,110,10"><line x1="10" y1="-10" x2="110" y2="-10" stroke="black" stroke-width="2"/></g></svg>';
    const review = { project_name: "drafting-test", drawing_names: ["plan"], current_drawing: "plan", sheet_svg: svg, diff_svg: svg, check: { schema_version: "0.3", status: "ok", diagnostics: [] }, diff: { schema_version: "0.3", status: "ok", changes: [], warnings: [], configuration_changes: [] }, comments: [], comments_revision: "comments", diff_unavailable: null,
      layers: { revision: "layers", active_layer: "0-1", groups: [{ id: "default", name: "Default", visible: true, locked: false, order: 0, scale_denominator: 100 }], layers: [{ id: "0-1", name: "Lines", group: "default", visible: true, locked: false, printable: true, color: "black", line_type: "solid", line_width: 0.25, order: 0, used_entities: 1 }] },
      editor: { drawing: "plan", revision: "before", entities: [entity], text_styles: ["note"], dimension_styles: ["dim"], pens: [], fills: ["black"] }, blocks: [], layouts: [] };
    let callback = 0;
    host.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: () => ++callback, unregisterCallback: () => undefined,
      invoke: async (command: string, args: Record<string, unknown> = {}) => {
        calls.push({ command, args });
        if (command === "load_last_project") return "/test-project";
        if (command === "open_project") return { project_path: "/test-project", project_name: "drafting-test", drawings: ["plan"], editable: true, is_git_project: false };
        if (command === "run_review") return review;
        if (command === "list_drawing_history") return { drawing: "plan", undo: [], redo: [], limit: 100, current_files: [] };
        if (command === "write_ai_context") return { status: "ready", markdown_path: "/test-project/build/context.md" };
        if (command === "query_snap") return null;
        if (command === "plugin:event|listen") return 1;
        if (command === "preview_drawing_edit") return { drawing: "plan", expected_revision: "before", entities: [entity], entity_ids: [entity.id], warnings: [], dimension_impacts: [], source_files: [], svg };
        if (command === "apply_drawing_edit") return { drawing: "plan", revision: "before", entity_ids: [entity.id], operation: "rectangle", history_id: "history" };
        if (command === "start_project_watch") return { project_path: "/test-project", mode: "native" };
        return null;
      },
    };
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "drafting-test" })).toBeVisible();
  await page.getByRole("button", { name: "Sheet", exact: true }).click();
}

async function command(page: Page, value: string) {
  const input = page.getByRole("textbox", { name: "Command or coordinate", exact: true });
  await input.fill(value); await input.press("Enter");
}

async function writes(page: Page) {
  return page.evaluate(() => ((window as unknown as Record<string, unknown>).__CAD_TEST_CALLS__ as Array<{ command: string; args: Record<string, unknown> }>).filter(call => call.command === "apply_drawing_edit"));
}

test("rectangle parameters preview without saving; cancel and repeat remain separate", async ({ page }) => {
  await openDraftingHarness(page);
  await command(page, "rectangle");
  await page.getByRole("button", { name: "Re-run Review", exact: true }).click();
  await expect(page.locator(".live-review-status.is-refreshing")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Drafting parameters" })).toContainText("rectangle");
  await command(page, "0,0");
  await page.getByRole("button", { name: "Re-run Review", exact: true }).click();
  await expect(page.locator(".live-review-status.is-refreshing")).toHaveCount(0);
  await expect(page.locator(".draft-overlay > circle")).toHaveCount(1);
  const panel = page.getByRole("region", { name: "Drafting parameters" });
  await panel.getByRole("spinbutton", { name: "Width", exact: true }).fill("300");
  await panel.getByRole("spinbutton", { name: "Height", exact: true }).fill("200");
  await panel.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(panel).toContainText("Preview ready");
  expect(await writes(page)).toHaveLength(0);
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await writes(page)).toHaveLength(0);
  await command(page, "rectangle"); await command(page, "0,0"); await command(page, "300,200");
  await expect(panel).toContainText("Preview ready");
  await panel.getByRole("button", { name: "Apply", exact: true }).click();
  await expect.poll(async () => (await writes(page)).length).toBe(1);
  const request = (await writes(page))[0].args.request as { operation: { kind: string; operation: unknown }; expected_revision: string };
  expect(request.expected_revision).toBe("before");
  expect(request.operation.kind).toBe("source_checked");
  expect(request.operation.operation).toMatchObject({ kind: "rectangle", p1: [0, 0], p2: [300, 200] });
  await page.getByRole("heading", { name: "drafting-test" }).click();
  await page.keyboard.press("Space");
  await expect(panel).toContainText("rectangle");
});

test("hatch accepts more than three vertices and typing never starts tools", async ({ page }) => {
  await openDraftingHarness(page);
  await command(page, "hatch");
  for (const point of ["0,0", "100,0", "100,100", "0,100"]) await command(page, point);
  const panel = page.getByRole("region", { name: "Drafting parameters" });
  await expect(panel).toContainText("4 boundary vertices");
  await page.getByRole("textbox", { name: "Command or coordinate", exact: true }).press("Backspace");
  await command(page, "0,100");
  await expect(panel).toContainText("4 boundary vertices");
  expect(await writes(page)).toHaveLength(0);
  await panel.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(panel).toContainText("Preview ready");
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  await command(page, "text");
  await panel.getByRole("textbox", { name: "Text value" }).fill("line\nrectangle");
  await expect(panel.getByRole("textbox", { name: "Text value" })).toHaveValue("line\nrectangle");
  expect(await writes(page)).toHaveLength(0);
});

test("invalid preview parameters block keyboard save without blocking cancellation", async ({ page }) => {
  await openDraftingHarness(page);
  await command(page, "rectangle"); await command(page, "0,0"); await command(page, "300,200");
  const panel = page.getByRole("region", { name: "Drafting parameters" });
  await expect(panel).toContainText("Preview ready");
  await panel.getByRole("spinbutton", { name: "Width", exact: true }).fill("0");
  await expect(panel.getByRole("button", { name: "Apply", exact: true })).toBeDisabled();
  await page.getByRole("heading", { name: "drafting-test" }).click();
  await page.keyboard.press("Enter");
  expect(await writes(page)).toHaveLength(0);
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await writes(page)).toHaveLength(0);
});

test("changing dimension association discards the old preview and uses fresh anchors", async ({ page }) => {
  await openDraftingHarness(page);
  await command(page, "dimension");
  for (const point of ["10,10", "110,10", "110,30"]) await command(page, point);
  const panel = page.getByRole("region", { name: "Drafting parameters" });
  await expect(panel).toContainText("Preview ready");
  await panel.getByRole("checkbox", { name: "Associate with geometry" }).uncheck();
  await expect(panel).toContainText("Pick the measurement points again");
  await panel.getByRole("button", { name: "Apply", exact: true }).click();
  expect(await writes(page)).toHaveLength(0);
  for (const point of ["10,10", "110,10", "110,30"]) await command(page, point);
  await expect(panel).toContainText("Preview ready");
  await panel.getByRole("button", { name: "Apply", exact: true }).click();
  await expect.poll(async () => (await writes(page)).length).toBe(1);
  expect((await writes(page))[0].args.request).toMatchObject({ operation: { kind: "source_checked", operation: { kind: "create", entity: { measurement: { first: { kind: "fixed" }, second: { kind: "fixed" } } } } } });
});
