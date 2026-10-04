import { test, expect } from "@playwright/test";

test("commit reviews unrelated staged files and requires accepting the disclosed commit policy", async ({ page }) => {
  await page.goto("/tests/fixtures/git-commit.html?ready");
  await expect(page.getByLabel("Complete commit candidate")).toContainText("unrelated.txt");
  await expect(page.getByLabel("Complete commit candidate")).toContainText("CAD Test");
  const commit=page.getByRole("button",{name:"Create reviewed commit",exact:true});
  await expect(commit).toBeDisabled();
  await page.getByText("Review staged patch",{exact:true}).click();
  await expect(page.locator(".commit-patch pre")).toContainText("Previously staged outside the CAD selection");
  await page.getByRole("checkbox").check();
  await commit.focus();await page.keyboard.press("Enter");
  await expect(page.locator(".git-commit-panel > p[role='status']")).toContainText("Commit created: new-commit");
  expect(JSON.parse(await page.locator("#result").textContent()||"{}")).toMatchObject({message:"Reviewed drawing change",hash:"reviewed-index"});
});

test("changing the message invalidates the reviewed plan and stale index errors require a new preview", async ({ page }) => {
  await page.goto("/tests/fixtures/git-commit.html?ready&stale");
  await expect(page.getByLabel("Complete commit candidate")).toBeVisible();
  await page.getByLabel("Commit message").fill("New message");
  await expect(page.getByLabel("Complete commit candidate")).toHaveCount(0);
  await page.getByRole("button",{name:"Preview complete index",exact:true}).click();
  await page.getByRole("checkbox").check();
  await page.getByRole("button",{name:"Create reviewed commit",exact:true}).click();
  await expect(page.locator(".git-commit-panel > p[role='status']")).toContainText("Index or branch changed");
  await expect(page.getByLabel("Complete commit candidate")).toHaveCount(0);
  await expect(page.locator("#result")).toHaveText("Branch preserved.");
});

test("an incomplete patch cannot be committed even after acknowledgement", async ({ page }) => {
  await page.setViewportSize({width:375,height:812});
  await page.goto("/tests/fixtures/git-commit.html?ready&large");
  await expect(page.getByRole("alert")).toContainText("Commit is blocked");
  await page.getByRole("checkbox").check();
  await expect(page.getByRole("button",{name:"Create reviewed commit",exact:true})).toBeDisabled();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});
