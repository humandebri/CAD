import { test, expect } from "@playwright/test";

test("attribute pickup stages layer and pen for new geometry while retaining the source", async ({ page }) => {
  await page.setViewportSize({width:375,height:812});
  await page.goto("/tests/fixtures/creation-attributes.html");
  await page.getByRole("button",{name:"Use selected attributes"}).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByLabel("Next drawing attributes")).toContainText("layer 0-2, pen outline, text style note");
  await expect(page.locator("output")).toHaveText("Source unchanged.");
  await expect(page.getByLabel("Drafting pen")).toHaveValue("outline");
  await page.getByRole("button",{name:"Apply",exact:true}).click();
  expect(JSON.parse(await page.locator("output").innerText())).toMatchObject({kind:"rectangle",layer:"0-2",pen:"outline",p1:[100,200],p2:[1100,1200]});
  await page.getByRole("button",{name:"Clear acquired attributes"}).click();
  await expect(page.getByLabel("Drafting pen")).toHaveValue("");
  await expect(page.getByLabel("Next drawing attributes")).toContainText("Active layer and drafting parameters");
  await page.getByRole("button",{name:"Apply",exact:true}).click();
  expect(JSON.parse(await page.locator("output").innerText())).toMatchObject({layer:"0-1",pen:null,p1:[100,200]});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  await page.goto("/tests/fixtures/creation-attributes.html?empty");
  await expect(page.getByRole("button",{name:"Use selected attributes"})).toBeDisabled();
});
