import {test,expect} from '@playwright/test';
test('reviewed clean merge is applied by keyboard and changing a revision discards the candidate',async({page})=>{
  await page.goto('/tests/fixtures/git-merge.html');
  await page.getByRole('button',{name:'Preview CAD merge',exact:true}).click();
  await expect(page.getByLabel('CAD merge candidate')).toContainText('Changed source files: 2');
  await expect(page.getByLabel('CAD merge candidate')).toContainText('rules/styles.toml');
  await expect(page.locator('#result')).toHaveText('Source and index preserved.');
  expect(await page.evaluate(()=>Reflect.get(window,'compromised'))).toBeUndefined();
  await page.getByLabel('Merge incoming commit').fill('feature');
  await expect(page.getByLabel('CAD merge candidate')).toHaveCount(0);
  await page.getByRole('button',{name:'Preview CAD merge',exact:true}).click();
  await page.getByRole('button',{name:'Apply reviewed CAD merge',exact:true}).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#result')).toHaveText('Applied reviewed-merge (HEAD~1, feature)');
  await expect(page.getByLabel('CAD merge candidate')).toHaveCount(0);
});
test('conflict values are reviewable and blocked candidates cannot apply',async({page})=>{
  await page.goto('/tests/fixtures/git-merge.html?blocked');
  await page.getByRole('button',{name:'Preview CAD merge',exact:true}).click();
  await expect(page.getByRole('button',{name:'Apply reviewed CAD merge',exact:true})).toBeDisabled();
  await page.locator('.merge-conflict summary').click();
  await expect(page.locator('.merge-conflict')).toContainText('ours');
  await expect(page.locator('.merge-conflict pre').nth(1)).toHaveText('[\n  5,\n  0\n]');
  await page.setViewportSize({width:375,height:812});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.getByRole('button',{name:'Discard merge preview',exact:true}).click();
  await expect(page.getByLabel('CAD merge candidate')).toHaveCount(0);
});
test('stale application retains working files and requires a fresh preview',async({page})=>{
  await page.goto('/tests/fixtures/git-merge.html?stale');
  await page.getByRole('button',{name:'Preview CAD merge',exact:true}).click();
  await page.getByRole('button',{name:'Apply reviewed CAD merge',exact:true}).click();
  await expect(page.locator('.git-merge-panel > p[role="status"]')).toContainText('Source changed');
  await expect(page.getByLabel('CAD merge candidate')).toHaveCount(0);
  await expect(page.locator('#result')).toHaveText('Source and index preserved.');
});
