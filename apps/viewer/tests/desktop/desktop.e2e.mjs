import { strict as assert } from "node:assert";
import { existsSync, readFileSync } from "node:fs";

async function activate(element) {
  await browser.execute((target) => setTimeout(() => target.click(), 0), element);
}

describe("CAD Review desktop", () => {
  it("edits, comments, changes a layer, restores history, and exports PDF", async () => {
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

    const hideLayer = await $("button[aria-label^='Hide ']");
    await hideLayer.waitForClickable();
    await activate(hideLayer);

    const undo = await $("button[aria-label='Undo']");
    await undo.waitForEnabled({ timeout: 10_000 });
    await activate(undo);
    const redo = await $("button[aria-label='Redo']");
    await redo.waitForEnabled({ timeout: 10_000 });
    await activate(redo);

    await activate(await $("button[aria-label='Export PDF']"));
    await browser.waitUntil(() => existsSync(pdfPath), {
      timeout: 10_000,
      timeoutMsg: "PDF was not published",
    });
    assert.equal(readFileSync(pdfPath).subarray(0, 5).toString(), "%PDF-");
  });
});
