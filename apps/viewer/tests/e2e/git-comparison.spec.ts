import { expect, test } from "@playwright/test";

test("Git comparison accepts refs and keyboard submission, and rejects blank refs", async ({ page }) => {
  await page.goto("/tests/fixtures/git-comparison.html");
  const base = page.getByRole("combobox", { name: "Comparison base revision" });
  const head = page.getByRole("combobox", { name: "Comparison head revision" });
  await expect(base).toHaveValue("HEAD");
  await expect(head).toHaveValue("worktree");
  await base.fill(" HEAD~1 ");
  await head.fill("index");
  await head.press("Enter");
  await expect(page.locator("output")).toHaveText("HEAD~1 → index");
  await base.fill("  ");
  await expect(page.getByRole("button", { name: "Compare revisions" })).toBeDisabled();
  await head.press("Enter");
  await expect(page.locator("output")).toHaveText("HEAD~1 → index");
});
