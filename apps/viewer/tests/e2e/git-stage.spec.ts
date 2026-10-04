import {test,expect} from '@playwright/test';
test('staging requires a reviewed candidate and keyboard activation',async({page})=>{
  await page.goto('/tests/fixtures/git-stage.html');
  await page.getByRole('button',{name:'Preview staging',exact:true}).click();
  await expect(page.getByLabel('Staging candidate')).toContainText('Entities including dependencies: 2');
  await expect(page.locator('#result')).toHaveText('Index preserved.');
  expect(await page.evaluate(()=>Reflect.get(window,'compromised'))).toBeUndefined();
  await page.getByRole('button',{name:'Stage reviewed candidate',exact:true}).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#result')).toHaveText('Staged reviewed-123');
});
test('checker errors block staging and keep the diagnostics visible',async({page})=>{
  await page.goto('/tests/fixtures/git-stage.html?blocked');
  await page.getByRole('button',{name:'Preview staging',exact:true}).click();
  await expect(page.getByLabel('Staging candidate')).toContainText('Dimension target is missing.');
  await expect(page.getByRole('button',{name:'Stage reviewed candidate',exact:true})).toBeDisabled();
  await page.getByRole('button',{name:'Discard staging preview',exact:true}).click();
  await expect(page.getByLabel('Staging candidate')).toHaveCount(0);
});
