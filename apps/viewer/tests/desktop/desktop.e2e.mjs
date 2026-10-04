import { strict as assert } from "node:assert";
import { existsSync, readFileSync, statSync, mkdirSync, writeFileSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve,dirname,basename } from "node:path";

async function reveal(element) {
  const route = await browser.execute(target => ({
    tab: target.closest("[role='tabpanel']")?.getAttribute("aria-labelledby"),
    file: target.closest(".file-menu") !== null && !target.closest(".file-menu").open,
  }), element);
  if (route.tab) {
    const tab = await $(`#${route.tab}`);
    if (await tab.getAttribute("aria-selected") !== "true") {
      await browser.execute(target => target.click(), tab);
      await browser.waitUntil(async () => await tab.getAttribute("aria-selected") === "true");
    }
  }
  if (route.file) await browser.execute(target => target.click(), await $(".file-menu > summary"));
}

async function activate(element) {
  await reveal(element);
  await browser.execute((target) => setTimeout(() => {
    if (target instanceof SVGElement) target.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    else target.click();
  }, 0), element);
}

function jwsLineFixture() {
  const chunks=[];
  const u16=value=>{const b=Buffer.alloc(2);b.writeUInt16LE(value);chunks.push(b);};
  const u32=value=>{const b=Buffer.alloc(4);b.writeUInt32LE(value);chunks.push(b);};
  const f64=value=>{const b=Buffer.alloc(8);b.writeDoubleLE(value);chunks.push(b);};
  chunks.push(Buffer.from("JwsData."),Buffer.alloc(192,46));u32(600);
  [2,3].forEach(f64);for(let i=0;i<16;i++)f64(100);
  chunks.push(Buffer.alloc(72));[2,3,12,23].forEach(f64);
  u16(1);u16(0xffff);u16(600);u16(8);chunks.push(Buffer.from("CDataSen"));
  u32(0);chunks.push(Buffer.from([1]));u16(2);for(let i=0;i<4;i++)u16(0);
  [2,3,12,23].forEach(f64);u16(0);return Buffer.concat(chunks);
}

describe("CAD Review desktop", () => {
  afterEach(async function () {
    if (this.currentTest?.state === "failed") {
      console.error("Desktop draft failure state", await browser.execute(() => ({
        command: document.querySelector(".command-prompt")?.textContent,
        input: document.querySelector("input[aria-label='Command or coordinate']")?.value,
        panel: document.querySelector(".drafting-panel")?.textContent,
        toolkit: document.querySelector(".toolkit-panel")?.textContent,
        entityHistory: document.querySelector(".entity-history-panel")?.textContent,
        review: document.querySelector(".live-review-status")?.textContent,
        message: document.querySelector(".editor-message")?.textContent,
        printPreview: document.querySelector(".print-preview")?.textContent,
        printWidth: document.querySelector(".print-preview")?.getBoundingClientRect().width,
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
    const minimumViewport = await browser.execute(() => ({
      width: innerWidth, height: innerHeight, pageWidth: document.documentElement.scrollWidth,
      pageHeight: document.documentElement.scrollHeight,
      bounds: [".drawing-stage", ".command-bar", ".command-bar input"].map(selector => {
        const box = document.querySelector(selector).getBoundingClientRect();
        return { selector, top: box.top, bottom: box.bottom };
      }),
    }));
    assert.equal(minimumViewport.width, 980, "Desktop regression scenario must run at minimum width");
    assert(minimumViewport.pageWidth <= minimumViewport.width, "minimum Desktop viewport overflows horizontally");
    assert(minimumViewport.pageHeight <= minimumViewport.height, "minimum Desktop viewport requires page scrolling");
    for (const box of minimumViewport.bounds) {
      assert(box.top >= 0 && box.bottom <= minimumViewport.height, `${box.selector} is outside the minimum Desktop viewport: ${JSON.stringify(minimumViewport)}`);
    }
    await activate(await $(".comment-select-button"));

    const properties = await $("section[aria-label='Entity properties']");
    await reveal(properties);
    await properties.waitForDisplayed();
    const dx = await properties.$(".translate-controls input");
    await dx.setValue("5");
    await activate(await properties.$("button=Move"));
    await browser.waitUntil(() => readFileSync(entitiesPath, "utf8") !== initialEntities, {
      timeout: 10_000,
      timeoutMsg: "entity edit was not persisted",
    });

    const movedEntities = readFileSync(entitiesPath, "utf8");
    await $("button[aria-label='Undo']").waitForEnabled({ timeout: 10_000 });
    await browser.execute(() => {
      const button = document.querySelector("button[aria-label='Zoom in']");
      button.focus();
      button.dispatchEvent(new KeyboardEvent("keydown", { key: "z", metaKey: true, bubbles: true, cancelable: true }));
    });
    await browser.waitUntil(() => readFileSync(entitiesPath, "utf8") === initialEntities, {
      timeout: 10_000, timeoutMsg: "Undo shortcut on a focused toolbar button did not restore geometry",
    });
    await $("button[aria-label='Redo']").waitForEnabled({ timeout: 10_000 });
    await browser.execute(() => {
      const button = document.querySelector("button[aria-label='Zoom in']");
      button.focus();
      button.dispatchEvent(new KeyboardEvent("keydown", { key: "z", metaKey: true, shiftKey: true, bubbles: true, cancelable: true }));
    });
    await browser.waitUntil(() => readFileSync(entitiesPath, "utf8") === movedEntities, {
      timeout: 10_000, timeoutMsg: "Redo shortcut on a focused toolbar button did not restore the edit",
    });

    await activate(await $("button=Compare revisions"));
    await $(".snapshot-identity").waitForDisplayed({timeout:10_000});
    const identityBefore=await $(".snapshot-identity").getText();
    const indexBefore=execFileSync("git",["-C",projectPath,"show",":drawings/plan_1f/entities.ndjson"],{encoding:"utf8"});
    const workBeforeStage=readFileSync(entitiesPath,"utf8");
    await activate(await $(".selective-stage-panel summary"));
    await activate(await $("button=Preview staging"));
    await $("button=Stage reviewed candidate").waitForDisplayed({timeout:10_000});
    await $("button=Stage reviewed candidate").waitForEnabled({timeout:10_000});
    assert.equal(execFileSync("git",["-C",projectPath,"show",":drawings/plan_1f/entities.ndjson"],{encoding:"utf8"}),indexBefore,"stage preview modified index");
    await activate(await $("button=Stage reviewed candidate"));
    await browser.waitUntil(()=>execFileSync("git",["-C",projectPath,"show",":drawings/plan_1f/entities.ndjson"],{encoding:"utf8"})!==indexBefore,{timeout:10_000});
    assert.equal(readFileSync(entitiesPath,"utf8"),workBeforeStage,"staging changed working drawing");
    const commitIndexBefore=readFileSync(resolve(projectPath,".git/index"));
    await activate(await $(".git-commit-panel summary"));
    await $("textarea[aria-label='Commit message']").setValue("staged move");
    await activate(await $("button=Preview complete index"));
    await $("section[aria-label='Complete commit candidate']").waitForDisplayed({timeout:10_000});
    assert.match(await $("section[aria-label='Complete commit candidate']").getText(),/CAD desktop test/);
    assert.equal(readFileSync(entitiesPath,"utf8"),workBeforeStage,"commit preview changed working drawing");
    assert.deepEqual(readFileSync(resolve(projectPath,".git/index")),commitIndexBefore,"commit preview changed index");
    await activate(await $(".commit-acknowledgement input"));
    await $("button=Create reviewed commit").waitForEnabled({timeout:10_000});
    await activate(await $("button=Create reviewed commit"));
    await browser.waitUntil(async()=> (await $(".git-commit-panel > p[role='status']").getText()).includes("Commit created:"), {timeout:10_000});
    assert.equal(execFileSync("git",["-C",projectPath,"log","-1","--format=%s"],{encoding:"utf8"}).trim(),"staged move");
    assert.equal(readFileSync(entitiesPath,"utf8"),workBeforeStage,"reviewed commit changed working drawing");
    assert.deepEqual(readFileSync(resolve(projectPath,".git/index")),commitIndexBefore,"reviewed commit rewrote index");
    await browser.waitUntil(async()=>await $(".snapshot-identity").getText()!==identityBefore,{timeout:10_000,timeoutMsg:"native Git HEAD watcher did not refresh comparison"});

    await activate(await $(".entity-history-panel summary"));
    await activate(await $("button=Load entity history"));
    await $("section[aria-label='Entity Git history']").waitForDisplayed({timeout:10_000});
    assert.match(await $("section[aria-label='Entity Git history']").getText(),/staged move/);
    const indexBeforeBlame=readFileSync(resolve(projectPath,".git/index"));
    const sourceBeforeBlame=readFileSync(entitiesPath,"utf8");
    await activate(await $("button=Load field origins"));
    await $("section[aria-label='Entity field origins']").waitForDisplayed({timeout:10_000});
    assert.match(await $("section[aria-label='Entity field origins']").getText(),/staged move/);
    assert.match(await $("section[aria-label='Entity field origins']").getText(),/CAD desktop test/);
    assert.deepEqual(readFileSync(resolve(projectPath,".git/index")),indexBeforeBlame,"field origins modified index");
    assert.equal(readFileSync(entitiesPath,"utf8"),sourceBeforeBlame,"field origins modified canonical source");

    await activate(await $("button.layer-name[title='0-1']"));
    await $(".layer-row.is-active button.layer-name[title='0-1']").waitForExist({ timeout:10_000 });
    const beforeBuilding = readFileSync(entitiesPath, "utf8");
    const beforeCount = beforeBuilding.trim().split("\n").filter(Boolean).length;
    await $("input[aria-label='Command or coordinate']").waitForEnabled({timeout: 10_000});
    await activate(await $(".toolkit-panel summary"));
    const workspaceBounds = await browser.execute(() => {
      const rect = selector => { const b = document.querySelector(selector).getBoundingClientRect(); return {x:b.x,y:b.y,width:b.width,height:b.height}; };
      return {viewport:innerWidth,page:document.documentElement.scrollWidth,inspector:rect(".detail-panel"),drawing:rect(".svg-surface > svg"),overlay:rect(".vertex-overlay")};
    });
    assert(workspaceBounds.page <= workspaceBounds.viewport, `Desktop workspace overflow: ${JSON.stringify(workspaceBounds)}`);
    assert(workspaceBounds.inspector.x + workspaceBounds.inspector.width <= workspaceBounds.viewport, "workspace panel is outside the window");
    for (const dimension of ["x","y","width","height"]) assert(Math.abs(workspaceBounds.drawing[dimension]-workspaceBounds.overlay[dimension])<1, `vertex overlay ${dimension} does not match drawing: ${JSON.stringify(workspaceBounds)}`);
    mkdirSync(resolve(import.meta.dirname, "../../../../build"), {recursive:true});
    await browser.saveScreenshot(resolve(import.meta.dirname, "../../../../build/ui-workspace.png"));
    const drawingSpace = await browser.execute(() => { const b = document.querySelector(".drawing-stage").getBoundingClientRect(); return {width:b.width,height:b.height}; });
    await $("input[aria-label='First point x,y']").setValue("123,456");
    await activate(await $("#workspace-tab-text"));
    await activate(await $("#workspace-tab-build"));
    assert.equal(await $("input[aria-label='First point x,y']").getValue(), "123,456", "task switching discarded a draft");
    assert.deepEqual(await browser.execute(() => { const b = document.querySelector(".drawing-stage").getBoundingClientRect(); return {width:b.width,height:b.height}; }), drawingSpace, "task panel resized the drawing");

    await $("input[aria-label='First point x,y']").setValue("1000,1000");
    await $("input[aria-label='Second point x,y']").setValue("2000,1000");
    await $("input[aria-label='Width']").setValue("100");
    await activate(await $("button=Preview building tool"));
    await $("button=Apply building preview").waitForEnabled({ timeout: 10_000 });
    assert.equal(readFileSync(entitiesPath, "utf8"), beforeBuilding, "building preview changed canonical source");
    await activate(await $("button=Apply building preview"));
    await browser.waitUntil(() => readFileSync(entitiesPath, "utf8").trim().split("\n").filter(Boolean).length === beforeCount + 2, {timeout:10_000});
    // A revision refresh remounts the panel; reopen it for read-only measurement.
    await activate(await $(".toolkit-panel summary"));
    await activate(await $(".toolkit-panel .toolkit-actions button:last-child"));
    await $("div[aria-label='Measured geometry']").waitForDisplayed({ timeout: 10_000 });
    assert.match(await $("div[aria-label='Measured geometry']").getText(), /Length:.* m/);
    const beforeMeasurement = readFileSync(entitiesPath, "utf8");
    await browser.execute(element => { element.value = "union"; element.dispatchEvent(new Event("change", {bubbles:true})); }, await $("select[aria-label='Area calculation']"));
    await $("input[aria-label='Area curve tolerance mm']").waitForDisplayed({timeout:10_000});
    await $("input[aria-label='Area curve tolerance mm']").setValue("0.1");
    await activate(await $(".toolkit-panel .toolkit-actions button:last-child"));
    await $("div[aria-label='Measured geometry']").waitForDisplayed({timeout:10_000});
    assert.match(await $("div[aria-label='Measured geometry']").getText(), /Overlapping areas are excluded/);
    assert.equal(readFileSync(entitiesPath, "utf8"), beforeMeasurement, "measurement changed canonical source");
    await $("button[aria-label='Undo']").waitForEnabled({ timeout: 10_000 });
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(() => readFileSync(entitiesPath, "utf8").trim().split("\n").filter(Boolean).length === beforeCount, {timeout:10_000});

    await $(".comment-select-button").waitForEnabled({ timeout:10_000 });
    await activate(await $(".comment-select-button"));
    const addComment = await $("button[aria-label='Add comment']");
    await addComment.waitForEnabled({ timeout:10_000 });
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
    await browser.waitUntil(async () => (await commentList.getText()).includes("Original entity unchanged"), {timeout:10_000});
    const commentsPath=resolve(projectPath,"comments/plan_1f.ndjson");
    const boundCommentBytes=readFileSync(commentsPath,"utf8");
    const newComment=boundCommentBytes.trim().split("\n").map(JSON.parse).find(c=>c.text==="desktop smoke");
    assert(newComment?.binding?.entity && newComment.binding.source_blake3,"comment did not record its original source");
    assert(!Object.hasOwn(newComment,"binding_state"),"transient comment state leaked into canonical data");

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
    await browser.waitUntil(async () => (await commentList.getText()).includes("Canonical source changed; review context"), {timeout:10_000});
    assert.equal(readFileSync(commentsPath,"utf8"),boundCommentBytes,"review rebound or rewrote a comment");
    const changedEntities=readFileSync(entitiesPath,"utf8").trim().split("\n").map(JSON.parse);
    const target=changedEntities.find(e=>e.id===newComment.entity_ids[0]);
    assert.equal(target.type,"line");target.p2[0]+=1;
    writeFileSync(entitiesPath,changedEntities.map(e=>JSON.stringify(e)).join("\n")+"\n");
    await browser.waitUntil(async () => (await commentList.getText()).includes("Entity changed since comment"), {timeout:10_000});
    assert.equal(readFileSync(commentsPath,"utf8"),boundCommentBytes,"geometry update rebound a comment");
    const projectSourcePath = resolve(projectPath,"cad.project.toml");
    const projectSourceBefore = readFileSync(projectSourcePath,"utf8");
    const mergeBase = execFileSync("git",["-C",projectPath,"rev-parse","HEAD"],{encoding:"utf8"}).trim();
    const incomingProject = projectSourceBefore.replace('name = "house-small"','name = "house-small-merged"');
    assert.notEqual(incomingProject,projectSourceBefore);
    writeFileSync(projectSourcePath,incomingProject);
    execFileSync("git",["-C",projectPath,"add","cad.project.toml"]);
    execFileSync("git",["-C",projectPath,"commit","-m","incoming project name"]);
    writeFileSync(projectSourcePath,projectSourceBefore);
    execFileSync(resolve(import.meta.dirname,"../../../../target/debug/cadc"),["check",projectPath,"--target","cad","--format","json","--out","-"],{encoding:"utf8"});
    const mergeIndexBefore = readFileSync(resolve(projectPath,".git/index"));
    const mergeEntitiesBefore = readFileSync(entitiesPath,"utf8");
    const mergeHeadBefore = execFileSync("git",["-C",projectPath,"rev-parse","HEAD"],{encoding:"utf8"});
    await activate(await $(".git-merge-panel summary"));
    await $("input[aria-label='Merge base commit']").setValue(mergeBase);
    await $("button=Preview CAD merge").waitForEnabled({timeout:15_000});
    await activate(await $("button=Preview CAD merge"));
    await $("button=Apply reviewed CAD merge").waitForEnabled({timeout:15_000});
    assert.equal(readFileSync(projectSourcePath,"utf8"),projectSourceBefore,"merge preview changed source");
    assert.match(await $("section[aria-label='CAD merge candidate']").getText(),/cad.project.toml/);
    await activate(await $("button=Apply reviewed CAD merge"));
    await browser.waitUntil(()=>readFileSync(projectSourcePath,"utf8")===incomingProject,{timeout:15_000});
    await $("h1=house-small-merged").waitForDisplayed({timeout:15_000});
    assert.equal(readFileSync(entitiesPath,"utf8"),mergeEntitiesBefore,"merge overwrote local geometry");
    assert.deepEqual(readFileSync(resolve(projectPath,".git/index")),mergeIndexBefore,"merge changed Git index");
    assert.equal(execFileSync("git",["-C",projectPath,"rev-parse","HEAD"],{encoding:"utf8"}),mergeHeadBefore,"merge moved HEAD");
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>readFileSync(projectSourcePath,"utf8")===projectSourceBefore,{timeout:15_000});
    await $("h1=house-small").waitForDisplayed({timeout:15_000});
    assert.equal(readFileSync(entitiesPath,"utf8"),mergeEntitiesBefore);
    await activate(await $(".comment-select-button"));
    await activate(await $(".clipboard-panel summary"));
    const clipboardSourceBefore = readFileSync(entitiesPath,"utf8");
    const clipboardIndexBefore = readFileSync(resolve(projectPath,".git/index"));
    await $("button=Copy selected to CAD clipboard").waitForEnabled({timeout:15_000});
    await activate(await $("button=Copy selected to CAD clipboard"));
    await $("section[aria-label='CAD clipboard contents']").waitForDisplayed({timeout:15_000});
    const partPath = `${pdfPath}.cadpart.json`;
    await $("input[aria-label='CAD part file']").setValue(partPath);
    await activate(await $("button=Save new CAD part"));
    await browser.waitUntil(()=>existsSync(partPath),{timeout:15_000});
    const portablePart=JSON.parse(readFileSync(partPath,"utf8"));
    assert.equal(portablePart.entities.length,1);
    assert.equal(portablePart.entities[0].type,"line");
    assert.equal(readFileSync(entitiesPath,"utf8"),clipboardSourceBefore,"copy or save part changed drawing");
    assert.deepEqual(readFileSync(resolve(projectPath,".git/index")),clipboardIndexBefore,"copy or save part changed index");
    await activate(await $("button=Clear CAD clipboard"));
    await $("section[aria-label='CAD clipboard contents']").waitForExist({reverse:true});
    await activate(await $("button=Load CAD part"));
    await $("section[aria-label='CAD clipboard contents']").waitForDisplayed({timeout:15_000});
    const exchangeOriginal = [entitiesPath, resolve(projectPath,"rules/layers.toml"), commentsPath].map(path => [path, readFileSync(path,"utf8")]);
    const dxfPath = `${pdfPath}.dxf`;
    const dxfReportPath = `${dxfPath}.report.json`;
    await activate(await $("button=Export DXF"));
    const exportDialog = await $("section[aria-label='Export DXF']");
    await exportDialog.waitForDisplayed();
    await exportDialog.$("input[aria-label='DXF output']").setValue(dxfPath);
    await exportDialog.$("input[aria-label='DXF report']").setValue(dxfReportPath);
    await activate(await exportDialog.$("button=Export DXF"));
    await browser.waitUntil(async () => (await exportDialog.$("p[role='status']").getText()).includes("DXF and compatibility report saved"), {timeout:10_000});
    assert(existsSync(dxfPath) && existsSync(dxfReportPath),"Desktop DXF output or report missing");
    const exported = JSON.parse(readFileSync(dxfReportPath,"utf8"));
    assert.equal(exported.status,"ready");
    assert(exported.source.snapshot_blake3,"DXF source was not pinned");
    assert(exported.exchange.warnings.length>0,"DXF semantic reductions were not reported");
    for (const [path,bytes] of exchangeOriginal) assert.equal(readFileSync(path,"utf8"),bytes,"DXF export changed original source");
    await activate(await exportDialog.$("button=Close"));
    await activate(await $("button=Import DXF"));
    const importDialog = await $("section[aria-label='Import DXF']");
    await importDialog.waitForDisplayed();
    const importedPath = `${projectPath}-dxf`;
    const importedReport = `${importedPath}.report.json`;
    await importDialog.$("input[aria-label='DXF input']").setValue(dxfPath);
    await importDialog.$("input[aria-label='DXF output']").setValue(importedPath);
    await importDialog.$("input[aria-label='DXF report']").setValue(importedReport);
    await activate(await importDialog.$("button=Import DXF"));
    await browser.waitUntil(async () => (await importDialog.$("p[role='status']").getText()).includes("Checked CAD project saved and opened"), {timeout:15_000});
    assert(existsSync(resolve(importedPath,"cad.project.toml")),"DXF project not published");
    assert.equal(JSON.parse(readFileSync(importedReport,"utf8")).status,"ready");
    const checked = JSON.parse(execFileSync(resolve(import.meta.dirname,"../../../../target/debug/cadc"),["check",importedPath,"--target","cad","--format","json","--out","-"],{encoding:"utf8"}));
    assert.equal(checked.status,"ok");
    for (const [path,bytes] of exchangeOriginal) assert.equal(readFileSync(path,"utf8"),bytes,"DXF import changed original project");
    await activate(await importDialog.$("button=Close"));
    await $(".drawing-stage svg").waitForDisplayed();
    await activate(await $(".clipboard-panel summary"));
    await $("section[aria-label='CAD clipboard contents']").waitForDisplayed({timeout:15_000});
    const importedDrawings=readdirSync(resolve(importedPath,"drawings"));
    assert.equal(importedDrawings.length,1);
    const importedDrawing=importedDrawings[0];
    const importedEntitiesPath=resolve(importedPath,`drawings/${importedDrawing}/entities.ndjson`);
    const pastedBefore=readFileSync(importedEntitiesPath,"utf8");
    const stylesBeforePaste=readFileSync(resolve(importedPath,"rules/styles.toml"),"utf8");
    const layersBeforePaste=readFileSync(resolve(importedPath,"rules/layers.toml"),"utf8");
    const originalPastedIds=new Set(pastedBefore.trim().split("\n").map(JSON.parse).map(entity=>entity.id));
    await $("input[aria-label='Clipboard paste point']").setValue("10000,10000");
    await $("button=Preview CAD paste").waitForEnabled({timeout:15_000});
    await activate(await $("button=Preview CAD paste"));
    await $("button=Apply reviewed CAD paste").waitForEnabled({timeout:15_000});
    assert.equal(readFileSync(importedEntitiesPath,"utf8"),pastedBefore,"paste preview changed drawing");
    await activate(await $("button=Apply reviewed CAD paste"));
    await browser.waitUntil(()=>readFileSync(importedEntitiesPath,"utf8")!==pastedBefore,{timeout:15_000});
    const added=readFileSync(importedEntitiesPath,"utf8").trim().split("\n").map(JSON.parse).filter(entity=>!originalPastedIds.has(entity.id));
    assert.equal(added.length,1);
    assert.notEqual(added[0].id,portablePart.entities[0].id);
    assert.deepEqual(added[0].p1,portablePart.entities[0].p1.map(coordinate=>coordinate+10000));
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    assert.equal(JSON.parse(execFileSync(resolve(import.meta.dirname,"../../../../target/debug/cadc"),["check",importedPath,"--target","cad","--format","json","--out","-"],{encoding:"utf8"})).status,"ok");
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>readFileSync(importedEntitiesPath,"utf8")===pastedBefore,{timeout:15_000});
    assert.equal(readFileSync(resolve(importedPath,"rules/styles.toml"),"utf8"),stylesBeforePaste);
    assert.equal(readFileSync(resolve(importedPath,"rules/layers.toml"),"utf8"),layersBeforePaste);
    for (const [path,bytes] of exchangeOriginal) assert.equal(readFileSync(path,"utf8"),bytes,"cross-project paste changed source project");
    await activate(await $(".clipboard-panel summary"));
    const jwsPath=`${pdfPath}.part.jws`;const jwsBytes=jwsLineFixture();writeFileSync(jwsPath,jwsBytes);
    await $("input[aria-label='JWS part file']").setValue(jwsPath);
    await $("input[aria-label='JWS coordinate scale']").setValue("100");
    await $("button=Load checked JWS part").waitForEnabled({timeout:15_000});
    await activate(await $("button=Load checked JWS part"));
    await $("textarea[aria-label='JWS compatibility report JSON']").waitForDisplayed({timeout:15_000});
    const jwsReport=JSON.parse(await $("textarea[aria-label='JWS compatibility report JSON']").getValue());
    assert.equal(jwsReport.status,"converted");assert.equal(jwsReport.exact_round_trip,false);
    assert.deepEqual(jwsReport.placement_origin_mm,[200,300]);
    assert.equal(readFileSync(importedEntitiesPath,"utf8"),pastedBefore,"JWS read changed project");
    assert.deepEqual(readFileSync(jwsPath),jwsBytes);
    await $("input[aria-label='Clipboard paste point']").setValue("5000,6000");
    await $("input[aria-label='Clipboard paste rotation']").setValue("90");
    await $("input[aria-label='Clipboard paste scale']").setValue("2");
    await activate(await $("button=Preview CAD paste"));
    await $("button=Apply reviewed CAD paste").waitForEnabled({timeout:15_000});
    await activate(await $("button=Apply reviewed CAD paste"));
    await browser.waitUntil(()=>readFileSync(importedEntitiesPath,"utf8")!==pastedBefore,{timeout:15_000});
    const jwsAdded=readFileSync(importedEntitiesPath,"utf8").trim().split("\n").map(JSON.parse).filter(entity=>!originalPastedIds.has(entity.id));
    assert.equal(jwsAdded.length,1);assert.deepEqual(jwsAdded[0].p1,[5000,6000]);assert.deepEqual(jwsAdded[0].p2,[1000,8000]);
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    assert.equal(JSON.parse(execFileSync(resolve(import.meta.dirname,"../../../../target/debug/cadc"),["check",importedPath,"--target","cad","--format","json","--out","-"],{encoding:"utf8"})).status,"ok");
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>readFileSync(importedEntitiesPath,"utf8")===pastedBefore,{timeout:15_000});
    assert.equal(readFileSync(resolve(importedPath,"rules/styles.toml"),"utf8"),stylesBeforePaste);
    assert.equal(readFileSync(resolve(importedPath,"rules/layers.toml"),"utf8"),layersBeforePaste);
    await activate(await $(".clipboard-panel > summary"));
    await activate(await $(".part-library-panel > summary"));
    const invalidLibraryJws=`${pdfPath}.unsupported.jws`;writeFileSync(invalidLibraryJws,"unknown");
    await $("input[aria-label='Part library folder']").setValue(dirname(pdfPath));
    await $("input[aria-label='Part library JWS scale']").setValue("100");
    await $("button=Load part folder").waitForEnabled({timeout:15_000});await activate(await $("button=Load part folder"));
    const libraryPartButton=await $(`button=Use part ${basename(partPath)}`);
    await libraryPartButton.waitForEnabled({timeout:15_000});
    assert.equal(await $(`button=Use part ${basename(invalidLibraryJws)}`).isEnabled(),false);
    assert.equal(readFileSync(importedEntitiesPath,"utf8"),pastedBefore,"library previews changed target project");
    await activate(libraryPartButton);
    await browser.waitUntil(async()=> (await $("section[aria-label='CAD clipboard contents']").getText()).includes(portablePart.source_project.name),{timeout:15_000});
    assert.equal(readFileSync(importedEntitiesPath,"utf8"),pastedBefore,"library selection changed target project");
    assert.deepEqual(readFileSync(jwsPath),jwsBytes);
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
    await browser.waitUntil(async () => (await $("section[aria-label='Drafting parameters']").getText()).includes("Preview ready"), {timeout:15_000});
    await browser.execute(() => document.querySelector(".svg-surface").dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true})));
    await $("section[aria-label='Drafting parameters']").waitForExist({reverse:true,timeout:15_000});
    await browser.waitUntil(() => entities().length > initialIds.size, {timeout:10_000,timeoutMsg:"Enter on canvas did not apply the active rectangle draft"});
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

    await command("select");
    await activate(await $(`.drawing-stage [data-entity-id='${rectangle.id}']`));
    await command("scale");
    await $("input[aria-label='Scale factor']").setValue("2");
    await command("20000,20000");
    await applyPreview();
    assert(entities().find(e=>e.id===rectangle.id).points.some(point=>point[0]===22000),"scale did not preserve the ID and double geometry around its base point");
    await waitDimension(dimension.id,"2000");
    await $("button[aria-label='Undo']").waitForEnabled({timeout:10_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>source()===beforeStretch,{timeout:10_000});
    await waitDimension(dimension.id,"1000");

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
    // Live review can replace the SVG after block creation; select the current node.
    await browser.waitUntil(async () => {
      const selected = await browser.execute(id => document.querySelector(`.drawing-stage [data-entity-id='${id}'].is-selected`) !== null, reference.id);
      if (!selected) await activate(await $(`.drawing-stage [data-entity-id='${reference.id}']`));
      return selected;
    }, { timeout: 10_000 });
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
    for (const type of ["circle", "text", "hatch"]) {
      const select = await dialog.$("select[aria-label='New block entity type']");
      await browser.execute((element, value) => { element.value = value; element.dispatchEvent(new Event("change", { bubbles: true })); }, select, type);
      const add = await dialog.$(`button=Add ${type}`);
      await add.waitForDisplayed();
      await activate(add);
      await dialog.$(`h3=${type} properties`).waitForDisplayed();
    }
    await activate(await dialog.$("button=Save contents"));
    await dialog.waitForExist({ reverse: true, timeout: 15_000 });
    assert.notEqual(readFileSync(blockPath, "utf8"), blockBefore);
    const savedContents = readFileSync(blockPath, "utf8").trim().split("\n").map(JSON.parse);
    for (const type of ["circle", "text", "hatch"]) assert(savedContents.some(entity => entity.type === type), `${type} was not saved in block`);

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
    const beforeTangent = source();
    const idsBeforeTangent = new Set(entities().map(entity => entity.id));
    const horizontal = entities().find(entity => entity.type === "line");
    await command("line");
    await $("input[aria-label='Length']").setValue("1000");
    await $("input[aria-label='Angle']").setValue("90");
    await command("1000,0"); await command("1000,1000");
    await browser.waitUntil(() => source() !== beforeTangent, {timeout:15_000});
    await $("section[aria-label='Drafting parameters']").waitForExist({reverse:true, timeout:15_000});
    const vertical = entities().find(entity => !idsBeforeTangent.has(entity.id));
    assert(vertical?.type === "line");
    await waitEntity(vertical.id);
    await activate(await $(`.drawing-stage [data-entity-id='${horizontal.id}']`));
    await browser.execute(id => document.querySelector(`.drawing-stage [data-entity-id='${id}']`)
      .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })), vertical.id);
    await browser.waitUntil(() => browser.execute(ids => ids.every(id => document.querySelector(`.drawing-stage [data-entity-id='${id}'].is-selected`)), [horizontal.id, vertical.id]), {timeout:10_000});
    await activate(await $(".toolkit-panel summary"));
    await browser.execute(element => {element.value = "tangent_circle"; element.dispatchEvent(new Event("change", {bubbles:true}));}, await $("select[aria-label='Building tool']"));
    await $("input[aria-label='Near center x,y']").setValue("2000,100");
    await $("input[aria-label='Radius']").setValue("100");
    const beforeTangentPreview = source();
    await activate(await $("button=Preview building tool"));
    await $("button=Apply building preview").waitForEnabled({timeout:15_000});
    assert.equal(source(), beforeTangentPreview, "tangent circle preview modified source");
    assert.match(await $(".toolkit-panel > p[role='status']").getText(), /infinite supporting lines/);
    await activate(await $("button=Apply building preview"));
    await browser.waitUntil(() => source() !== beforeTangentPreview, {timeout:15_000});
    const tangent = entities().find(entity => entity.type === "circle" && Math.abs(entity.radius - 100) < 1e-9);
    assert(tangent, "tangent circle was not saved");
    assert(Math.abs(tangent.center[0] - 1100) < 1e-7 && Math.abs(tangent.center[1] - 100) < 1e-7);
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(() => source() === beforeTangentPreview, {timeout:15_000});
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(() => source() === beforeTangent, {timeout:15_000});
    await command("text");
    await $("textarea[aria-label='Text value']").setValue("既存壁と既存壁");
    await command("25000,20000");
    await applyPreview();
    const annotation = entities().find(entity => entity.type === "text");
    assert(annotation, "annotation text was not saved");
    await activate(await $(".text-tools-panel summary"));
    await $("input[aria-label='Find annotation text']").setValue("既存");
    await $("textarea[aria-label='Replacement text']").setValue("改修");
    const beforeTextPreview = source();
    await activate(await $("button=Preview text replacement"));
    await $("button=Apply text preview").waitForEnabled({timeout:15_000});
    assert.equal(source(), beforeTextPreview, "text preview modified source");
    assert.match(await $(".text-tools-panel > p[role='status']").getText(), /1 text entities in the preview/);
    assert.equal(await $("button=Apply building preview").isEnabled(), false, "building panel can apply text preview");
    await activate(await $("button=Apply text preview"));
    await browser.waitUntil(() => entities().some(e => e.id === annotation.id && e.value === "改修壁と改修壁"), {timeout:15_000});
    await activate(await $(".text-tools-panel summary"));
    await browser.execute(element => {element.value="title"; element.dispatchEvent(new Event("change", {bubbles:true}));}, await $("select[aria-label='Batch text style']"));
    const beforeStylePreview = source();
    await activate(await $("button=Preview style change"));
    await $("button=Apply text preview").waitForEnabled({timeout:15_000});
    assert.equal(source(), beforeStylePreview, "style preview modified source");
    await activate(await $("button=Apply text preview"));
    await browser.waitUntil(() => entities().some(e => e.id === annotation.id && e.style === "title"), {timeout:15_000});
    const styled = entities().find(e => e.id === annotation.id);
    assert.equal(styled.value, "改修壁と改修壁");
    assert.deepEqual(styled.at, annotation.at);
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(() => source() === beforeStylePreview, {timeout:15_000});
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $(".text-tools-panel summary"));
    const beforeWritingDirection = source();
    await browser.execute(element => {element.value="vertical_upright";element.dispatchEvent(new Event("change",{bubbles:true}));}, await $("select[aria-label='Batch text writing direction']"));
    await activate(await $("button=Preview writing direction"));
    await $("button=Apply text preview").waitForEnabled({timeout:15_000});
    assert.equal(source(),beforeWritingDirection,"writing direction preview modified source");
    await activate(await $("button=Apply text preview"));
    await browser.waitUntil(()=>entities().some(e=>e.id===annotation.id&&e.writing_mode==="vertical_upright"),{timeout:15_000});
    const verticalAnnotation=entities().find(e=>e.id===annotation.id);
    assert.equal(verticalAnnotation.value,"改修壁と改修壁");
    assert.deepEqual(verticalAnnotation.at,annotation.at);
    assert.equal(verticalAnnotation.style,annotation.style);
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>source()===beforeWritingDirection,{timeout:15_000});
    await activate(await $(".toolkit-panel summary"));
    await browser.execute(element => {element.value="calculated_text"; element.dispatchEvent(new Event("change", {bubbles:true}));}, await $("select[aria-label='Building tool']"));
    await $("input[aria-label='Origin x,y']").setValue("25000,21000");
    await $("input[aria-label='Arithmetic expression']").setValue("1/(2-2)");
    await $("input[aria-label='Text prefix']").setValue("面積: ");
    await $("input[aria-label='Text suffix']").setValue(" m²");
    const beforeCalculation = source();
    await activate(await $("button=Preview building tool"));
    await browser.waitUntil(async () => (await $(".toolkit-panel > p[role='status']").getText()).includes("division by zero"), {timeout:15_000});
    assert.equal(await $("button=Apply building preview").isEnabled(), false);
    assert.equal(source(), beforeCalculation);
    await $("input[aria-label='Arithmetic expression']").setValue("1000*2000/1e6");
    await activate(await $("button=Preview building tool"));
    await $("button=Apply building preview").waitForEnabled({timeout:15_000});
    assert.match(await $(".toolkit-panel > p[role='status']").getText(), /= 2\.00/);
    assert.equal(source(), beforeCalculation);
    await activate(await $("button=Apply building preview"));
    await browser.waitUntil(() => entities().some(e => e.type === "text" && e.value === "面積: 2.00 m²"), {timeout:15_000});
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(() => source() === beforeCalculation, {timeout:15_000});
    const massingFootprint = entities().find(e=>e.type==="polyline" && e.closed);
    assert(massingFootprint,"standalone massing footprint is missing");
    const indexBeforeMassing = readFileSync(`${projectPath}/.git/index`);
    for (const tool of ["massing_projection", "sun_shadow", "sky_view"]) {
      await command("select");
      await waitEntity(massingFootprint.id);
      await activate(await $(`.drawing-stage [data-entity-id='${massingFootprint.id}']`));
      await browser.execute(() => {document.querySelector(".toolkit-panel").open=true;});
      await browser.execute((element,value)=>{element.value=value;element.dispatchEvent(new Event("change",{bubbles:true}));},await $("select[aria-label='Building tool']"),tool);
      await $("input[aria-label='Building height mm']").setValue("5000");
      if(tool==="massing_projection") {
        await $("input[aria-label='Projection origin east,north']").setValue("20000,20000");
        await $("input[aria-label='Diagram origin x,y']").setValue("25000,20000");
        await $("input[aria-label='Projection yaw degrees']").setValue("30");
        await $("input[aria-label='View elevation degrees']").setValue("30");
      } else if(tool==="sun_shadow") {
        await $("input[aria-label='Solar azimuth degrees']").setValue("90");
        await $("input[aria-label='Solar altitude degrees']").setValue("45");
      } else {
        await $("input[aria-label='Observer east,north']").setValue("15000,15000");
        await $("input[aria-label='Observer height mm']").setValue("0");
        await $("input[aria-label='Sky azimuth samples']").setValue("180");
        await $("input[aria-label='Diagram origin x,y']").setValue("25000,25000");
        await $("input[aria-label='Sky diagram radius mm']").setValue("1000");
      }
      const beforeMassing=source(), count=entities().length;
      await activate(await $("button=Preview building tool"));
      await $("button=Apply building preview").waitForEnabled({timeout:15_000});
      assert.equal(source(),beforeMassing,"massing preview modified source");
      const report=JSON.parse(await $("textarea[aria-label='Massing calculation report JSON']").getValue());
      assert.equal(report.schema_version,"cad-massing-review/1");
      assert.equal(report.analysis.conditions.buildings[0].id,massingFootprint.id);
      if(tool==="sky_view") {
        const reportPath=`${pdfPath}.massing-report.json`;
        await $("input[aria-label='Massing report file']").setValue(reportPath);
        await activate(await $("button=Save new massing report"));
        await browser.waitUntil(()=>existsSync(reportPath),{timeout:15_000});
        assert.deepEqual(JSON.parse(readFileSync(reportPath,"utf8")),report);
      }
      await $("button=Apply building preview").waitForEnabled({timeout:15_000});
      await activate(await $("button=Apply building preview"));
      await browser.waitUntil(()=>entities().length>count,{timeout:15_000});
      await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
      assert.equal(JSON.parse(execFileSync(resolve(import.meta.dirname,"../../../../target/debug/cadc"),["check",projectPath,"--target","cad","--format","json","--out","-"],{encoding:"utf8"})).status,"ok");
      assert.deepEqual(readFileSync(`${projectPath}/.git/index`),indexBeforeMassing);
      await activate(await $("button[aria-label='Undo']"));
      await browser.waitUntil(()=>source()===beforeMassing,{timeout:15_000});
      await waitEntity(massingFootprint.id);
    }
    const beforeAttributes = source();
    const idsBeforeAttributes = new Set(entities().map(e=>e.id));
    await command("line");
    await browser.execute(element=>{element.value="picked_outline";element.dispatchEvent(new Event("change",{bubbles:true}));}, await $("select[aria-label='Drafting pen']"));
    await command("1000,500"); await command("2000,500");
    await browser.waitUntil(()=>source()!==beforeAttributes,{timeout:15_000});
    await $("section[aria-label='Drafting parameters']").waitForExist({reverse:true,timeout:15_000});
    const prototype = entities().find(e=>!idsBeforeAttributes.has(e.id));
    assert.equal(prototype.pen,"picked_outline");
    await waitEntity(prototype.id);
    await activate(await $(`.drawing-stage [data-entity-id='${prototype.id}']`));
    await $("button=Use selected attributes").waitForEnabled({timeout:15_000});
    const beforePickup = source();
    await activate(await $("button=Use selected attributes"));
    await browser.waitUntil(async()=>(await $("section[aria-label='Next drawing attributes'] p[role='status']").getText()).includes("pen picked_outline"),{timeout:15_000});
    assert.equal(source(),beforePickup,"attribute pickup modified source");
    await command("rectangle");
    assert.equal(await $("select[aria-label='Drafting pen']").getValue(),"picked_outline");
    await command("30000,20000"); await command("31000,21000");
    await applyPreview();
    const pickedRectangle = entities().find(e=>!idsBeforeAttributes.has(e.id) && e.id!==prototype.id);
    assert.equal(pickedRectangle.type,"polyline");
    assert.equal(pickedRectangle.layer,prototype.layer);assert.equal(pickedRectangle.pen,prototype.pen);
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>source()===beforePickup,{timeout:15_000});
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>source()===beforeAttributes,{timeout:15_000});
    await $("button=Clear acquired attributes").waitForEnabled({timeout:15_000});
    await activate(await $("button=Clear acquired attributes"));
    await browser.waitUntil(async()=>(await $("section[aria-label='Next drawing attributes'] p[role='status']").getText()).includes("Active layer"),{timeout:15_000});
    assert.equal(source(),beforeAttributes,"clearing attributes modified source");
    const sheetLayoutPath=`${projectPath}/drawings/plan_1f/layouts.toml`;
    const sheetLayoutBefore=readFileSync(sheetLayoutPath,"utf8"),sheetGeometryBefore=source(),sheetIndexBefore=readFileSync(`${projectPath}/.git/index`);
    await activate(await $(".sheet-viewports-panel > summary"));
    for(const [index,scale,at] of [[1,"1/100","10,10"],[2,"1/20","150,10"]]) {
      await activate(await $("button=Add sheet view"));
      await $(`input[aria-label='View ${index} model origin']`).setValue("23000,20000");
      await $(`input[aria-label='View ${index} paper position']`).setValue(at);
      await $(`input[aria-label='View ${index} scale']`).setValue(scale);
    }
    await activate(await $("button=Preview scaled sheet"));
    await $("button=Apply reviewed sheet views").waitForEnabled({timeout:15_000});
    assert.equal(readFileSync(sheetLayoutPath,"utf8"),sheetLayoutBefore,"sheet preview changed layout");
    const sheetReport=JSON.parse(await $("textarea[aria-label='Scaled sheet report JSON']").getValue());
    assert.equal(sheetReport.after.viewports[1].scale,"1/20");
    await activate(await $("button=Apply reviewed sheet views"));
    await browser.waitUntil(()=>readFileSync(sheetLayoutPath,"utf8")!==sheetLayoutBefore,{timeout:15_000});
    await $("button[aria-label='Undo']").waitForEnabled({timeout:15_000});
    await browser.waitUntil(()=>browser.execute(()=>document.querySelectorAll(".drawing-stage [data-sheet-viewport]").length===2),{timeout:15_000});
    assert.equal(source(),sheetGeometryBefore);
    assert.deepEqual(readFileSync(`${projectPath}/.git/index`),sheetIndexBefore);
    await command("line");
    assert.equal(source(),sheetGeometryBefore);
    await activate(await $("button[aria-label='Undo']"));
    await browser.waitUntil(()=>readFileSync(sheetLayoutPath,"utf8")===sheetLayoutBefore,{timeout:15_000});
    assert.equal(source(),sheetGeometryBefore);
    await command("print_preview");
    const preview = await $("section[aria-label='PDF print preview']");
    await preview.waitForDisplayed({ timeout: 15_000 });
    await browser.waitUntil(async () => (await preview.getAttribute("data-rendered")) === "true", { timeout: 15_000 });
    assert(await browser.execute(() => {
      const canvas = document.querySelector(".print-preview canvas");
      if (!canvas || canvas.width === 0 || canvas.height === 0) return false;
      const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
      return pixels.some((value, index) => index % 4 !== 3 && value < 200);
    }), "print preview must contain visible drawing ink");
    const screenshotDir = resolve(import.meta.dirname, "../../../../build");
    mkdirSync(screenshotDir, { recursive: true });
    await browser.saveScreenshot(resolve(screenshotDir, "ui-print-preview.png"));
    await command("select");
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
