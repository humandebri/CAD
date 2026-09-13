import { strict as assert } from "node:assert";
import { existsSync, readFileSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";

async function activate(element) {
  await browser.execute((target) => setTimeout(() => {
    if (target instanceof SVGElement) target.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    else target.click();
  }, 0), element);
}

describe("CAD Review desktop", () => {
  afterEach(async function () {
    if (this.currentTest?.state === "failed") {
      console.error("Desktop draft failure state", await browser.execute(() => ({
        command: document.querySelector(".command-prompt")?.textContent,
        input: document.querySelector("input[aria-label='Command or coordinate']")?.value,
        panel: document.querySelector(".drafting-panel")?.textContent,
        review: document.querySelector(".live-review-status")?.textContent,
        message: document.querySelector(".editor-message")?.textContent,
        points: document.querySelectorAll(".draft-overlay > circle").length,
        focus: document.activeElement?.getAttribute("aria-label"),
        blockMessage: document.querySelector(".block-edit-dialog [role='status']")?.textContent,
        blockResolution: document.querySelector("section[aria-label='Resolve block dimension references']")?.textContent,
      })));
    }
  });
  it("edits, comments, and observes a native layer change", async () => {
    const projectPath = process.env.CAD_E2E_PROJECT_PATH;
    const pdfPath = process.env.CAD_E2E_PDF_PATH;
    assert(projectPath && pdfPath, "desktop E2E paths must be configured");
    const entitiesPath = `${projectPath}/drawings/plan_1f/entities.ndjson`;
    const initialEntities = readFileSync(entitiesPath, "utf8");

    await $("h1=house-small").waitForDisplayed({ timeout: 30_000 });
    const sheet = await $("button=Sheet");
    await activate(sheet);
    await browser.waitUntil(
      async () => await browser.execute(
        () => document.querySelectorAll(".drawing-stage [data-entity-id]").length > 0,
      ),
      { timeout: 10_000, timeoutMsg: "sheet geometry was not rendered" },
    );
    const entityCount = await browser.execute(
      () => document.querySelectorAll(".drawing-stage [data-entity-id]").length,
    );
    assert(entityCount > 0, `rendered drawing contains no entities: ${await browser.getPageSource()}`);
    await activate(await $(".comment-select-button"));

    const properties = await $("section[aria-label='Entity properties']");
    await properties.waitForDisplayed();
    const dx = await properties.$(".translate-controls input");
    await dx.setValue("5");
    await activate(await properties.$("button=Move"));
    await browser.waitUntil(() => readFileSync(entitiesPath, "utf8") !== initialEntities, {
      timeout: 10_000,
      timeoutMsg: "entity edit was not persisted",
    });

    const addComment = await $("button[aria-label='Add comment']");
    const openCommentPrompt = activate(addComment);
    await browser.waitUntil(async () => (await browser.getAlertText()) === "Comment", {
      timeout: 5_000,
    });
    await browser.sendAlertText("desktop smoke");
    await browser.acceptAlert();
    await openCommentPrompt;
    const commentList = await $(".comment-list");
    await browser.waitUntil(async () => (await commentList.getText()).includes("desktop smoke"), {
      timeout: 10_000,
      timeoutMsg: "created comment was not rendered",
    });

    await browser.execute(() => {
      globalThis.__cadWatchCycleComplete = false;
      let sawRefreshing = false;
      const status = document.querySelector(".live-review-status");
      const record = () => {
        const text = status?.textContent ?? "";
        if (text.includes("Live: refreshing")) sawRefreshing = true;
        if (sawRefreshing && text.includes("Live: watching")) {
          globalThis.__cadWatchCycleComplete = true;
          observer.disconnect();
        }
      };
      const observer = new MutationObserver(record);
      if (status !== null) {
        observer.observe(status, {
          attributes: true,
          childList: true,
          characterData: true,
          subtree: true,
        });
      }
    });
    const hideLayer = await $("button[aria-label^='Hide ']");
    await hideLayer.waitForClickable();
    await activate(hideLayer);
    await browser.waitUntil(
      () => browser.execute(() => globalThis.__cadWatchCycleComplete === true),
      { timeout: 10_000, timeoutMsg: "native watcher catch-up did not complete" },
    );


  });

  it("persists a dimensioned room stretch, shared block content, history, and output", async function () {
    this.timeout(240_000);
    const projectPath = process.env.CAD_E2E_PROJECT_PATH;
    const pdfPath = process.env.CAD_E2E_PDF_PATH;
    assert(projectPath && pdfPath, "desktop E2E paths must be configured");
    const entitiesPath = `${projectPath}/drawings/plan_1f/entities.ndjson`;
    const source = () => readFileSync(entitiesPath, "utf8");
    const entities = () => source().trim().split("\n").filter(Boolean).map(JSON.parse);
    const command = async (text) => {
      const input = await $("input[aria-label='Command or coordinate']");
      await input.waitForEnabled({ timeout: 15_000 });
      await browser.execute(element => element.focus(), input);
      await input.setValue(text);
      await browser.keys("Enter");
      await browser.waitUntil(async () => (await input.getValue()) === "", { timeout: 10_000 });
    };
    const applyPreview = async () => {
      const panel = await $("section[aria-label='Drafting parameters']");
      await browser.waitUntil(async () => (await panel.getText()).includes("Preview ready"), {
        timeout: 15_000, timeoutMsg: "geometry preview did not become ready",
      });
      await activate(await panel.$("button=Apply"));
      await panel.waitForExist({ reverse: true, timeout: 15_000 });
    };
    const waitEntity = async (id) => {
      await browser.waitUntil(() => browser.execute(
        (entityId) => document.querySelector(`.drawing-stage [data-entity-id='${entityId}']`) !== null, id,
      ), { timeout: 15_000, timeoutMsg: `entity ${id} was not rendered after saving` });
    };
    const waitDimension = async (id, label) => {
      await browser.waitUntil(() => browser.execute(
        (entityId, text) => Array.from(document.querySelectorAll(`.drawing-stage [data-entity-id='${entityId}']`))
          .some(element => (element.textContent ?? "").includes(text)), id, label,
      ), { timeout: 15_000, timeoutMsg: `dimension did not evaluate to ${label}` });
    };
    await activate(await $("button=Sheet"));
    await browser.waitUntil(async () => !(await $(".live-review-status.is-refreshing").isExisting()), { timeout: 15_000 });
    await activate(await $("button.layer-name[title='0-1']"));
    await $(".layer-row.is-active button.layer-name[title='0-1']").waitForExist({ timeout: 10_000 });
    await browser.waitUntil(async () => !(await $(".live-review-status.is-refreshing").isExisting()), { timeout: 15_000 });
    const initialIds = new Set(entities().map(entity => entity.id));
    await command("rectangle");
    await $("input[aria-label='Width']").waitForExist({ timeout: 10_000 });
    await $("input[aria-label='Width']").setValue("1000");
    await $("input[aria-label='Height']").setValue("1000");
    await command("20000,20000");
    await activate(await (await $("section[aria-label='Drafting parameters']")).$("button=Apply"));
    await applyPreview();
    const rectangle = entities().find(entity => !initialIds.has(entity.id) && entity.type === "polyline");
    assert(rectangle, "rectangle did not become a canonical polyline");
    await waitEntity(rectangle.id);

    const beforeDimensionIds = new Set(entities().map(entity => entity.id));
    await command("dimension");
    await command("20000,20000");
    await command("21000,20000");
    await command("20000,19800");
    await applyPreview();
    const dimension = entities().find(entity => !beforeDimensionIds.has(entity.id) && entity.type === "dimension");
    assert(dimension, "dimension was not saved");
    assert.equal(dimension.measurement.first.entity_id, rectangle.id);
    assert.equal(dimension.measurement.second.entity_id, rectangle.id);
    await waitDimension(dimension.id, "1000");
    const beforeStretch = source();

    await command("stretch");
    for (const point of ["20999,19999", "21001,21001", "0,0", "300,0"]) await command(point);
    await applyPreview();
    const stretched = entities().find(entity => entity.id === rectangle.id);
    assert(stretched.points.some(point => point[0] === 21300), "stretch did not move the right-hand vertices by 300");
    assert(stretched.points.some(point => point[0] === 20000), "stretch moved vertices outside its rectangle");
    await waitDimension(dimension.id, "1300");
    const afterStretch = source();
    await $("button[aria-label='Undo']").waitForEnabled({ timeout: 10_000 });
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(() => source() === beforeStretch, { timeout: 10_000 });
    await waitDimension(dimension.id, "1000");
    await $("button[aria-label='Redo']").waitForEnabled({ timeout: 10_000 });
    await activate(await $("button[aria-label='Redo']"));
    await browser.waitUntil(() => source() === afterStretch, { timeout: 10_000 });
    await waitDimension(dimension.id, "1300");

    await command("select");
    await activate(await $(`.drawing-stage [data-entity-id='${rectangle.id}']`));
    await browser.execute((id) => document.querySelector(`.drawing-stage [data-entity-id='${id}']`)
      .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })), dimension.id);
    await browser.waitUntil(() => browser.execute((ids) => ids.every(id => document.querySelector(`.drawing-stage [data-entity-id='${id}'].is-selected`)), [rectangle.id, dimension.id]), { timeout: 10_000 });
    await command("create_block");
    await $("input[aria-label='Block name']").setValue("E2E room");
    await command("20000,20000");
    await activate(await (await $("section[aria-label='Drafting parameters']")).$("button=Apply"));
    await browser.waitUntil(() => !entities().some(entity => entity.id === rectangle.id), { timeout: 15_000 });
    const reference = entities().find(entity => entity.type === "block_ref" && entity.block.startsWith("block_"));
    assert(reference, "block reference was not saved");
    const blockPath = `${projectPath}/blocks/${reference.block}/entities.ndjson`;
    const blockBefore = readFileSync(blockPath, "utf8");
    const blockEntities = blockBefore.trim().split("\n").map(JSON.parse);
    const blockRectangle = blockEntities.find(entity => entity.type === "polyline");
    const blockDimension = blockEntities.find(entity => entity.type === "dimension");
    assert.equal(blockDimension.measurement.first.entity_id, blockRectangle.id);
    assert.equal(blockDimension.measurement.second.entity_id, blockRectangle.id);
    await waitEntity(reference.id);
    await activate(await $(`.drawing-stage [data-entity-id='${reference.id}'] [data-block-child-id]`));
    await browser.waitUntil(() => browser.execute(id => document.querySelector(`.drawing-stage [data-entity-id='${id}'].is-selected`) !== null, reference.id), { timeout: 10_000 });
    await command("edit_block");
    let dialog = await $("section[aria-label='Edit block contents']");
    await dialog.waitForDisplayed({ timeout: 10_000 });
    await activate(await dialog.$("button=Delete"));
    const resolutions = await dialog.$("section[aria-label='Resolve block dimension references']");
    await resolutions.waitForDisplayed();
    await browser.execute(element => { element.value = "detach"; element.dispatchEvent(new Event("change", { bubbles: true })); }, await resolutions.$("select"));
    await resolutions.$("button=Resolve in draft").waitForEnabled({ timeout: 10_000 });
    await activate(await resolutions.$("button=Resolve in draft"));
    await resolutions.waitForExist({ reverse: true, timeout: 10_000 });
    assert.equal(readFileSync(blockPath, "utf8"), blockBefore, "dimension resolution modified source before Save");
    await activate(await dialog.$("button=Cancel"));
    await dialog.waitForExist({ reverse: true });
    await command("edit_block");
    dialog = await $("section[aria-label='Edit block contents']");
    await dialog.waitForDisplayed({ timeout: 10_000 });
    await dialog.$(".translate-controls input").setValue("5");
    await activate(await dialog.$("button=Move"));
    assert.equal(readFileSync(blockPath, "utf8"), blockBefore, "block preview modified canonical source before Save");
    await activate(await dialog.$("button=Save contents"));
    await dialog.waitForExist({ reverse: true, timeout: 15_000 });
    assert.notEqual(readFileSync(blockPath, "utf8"), blockBefore);
    await command("rectangle");
    await command("23000,20000"); await command("24000,21000");
    await applyPreview();
    const circleBefore = source();
    await command("circle"); await command("23500,20500"); await command("23700,20500");
    await browser.waitUntil(() => source() !== circleBefore, { timeout: 15_000 });
    await $("section[aria-label='Drafting parameters']").waitForExist({ reverse: true, timeout: 15_000 });
    await command("hatch");
    await activate(await $("section[aria-label='Drafting parameters'] input[type='checkbox']"));
    await command("23100,20100");
    await applyPreview();
    assert(entities().some(entity => entity.type === "hatch" && entity.loops.length === 2), "region hatch lost its inner hole");
    await activate(await $("button[aria-label='Export PDF']"));
    await browser.waitUntil(() => existsSync(pdfPath), { timeout: 15_000 });
    assert.equal(readFileSync(pdfPath).subarray(0, 5).toString(), "%PDF-");
    const cadc = resolve(import.meta.dirname, "../../../../target/debug/cadc");
    execFileSync("cargo", ["build", "-p", "cad-cli"], { cwd: resolve(import.meta.dirname, "../../../.."), stdio: "pipe" });
    const jwwPath = `${pdfPath}.jww`, reportPath = `${jwwPath}.report.json`;
    execFileSync(cadc, ["check", projectPath, "--drawing", "plan_1f", "--target", "jww-v600", "--format", "json", "--out", "-"], { stdio: "pipe" });
    execFileSync(cadc, ["export-jww", projectPath, "--drawing", "plan_1f", "--out", jwwPath, "--report", reportPath], { stdio: "pipe" });
    const report = JSON.parse(readFileSync(reportPath, "utf8"));
    assert.equal(report.status, "exported");
    assert(report.warnings.some(warning => warning.code === "dimension_geometry_expanded"));
    assert(statSync(jwwPath).size > 0);
  });
});
