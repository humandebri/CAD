import {test,expect} from '@playwright/test';
test('history shows its bounds and decode warnings before comparing pinned commits',async({page})=>{
  await page.goto('/tests/fixtures/entity-history.html');
  await page.getByRole('button',{name:'Load entity history',exact:true}).click();
  await expect(page.getByLabel('Entity Git history')).toContainText('Scan limit reached');
  await expect(page.getByLabel('Entity Git history')).toContainText('could not be decoded');
  await page.getByRole('button',{name:'Compare this change',exact:true}).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#result')).toHaveText('11223344 → abcdef0123456789');
  await page.getByLabel('History commit limit').fill('501');
  await expect(page.getByRole('button',{name:'Load entity history',exact:true})).toBeDisabled();
});
test('field origins retain incomplete-history warnings, values and pinned comparison',async({page})=>{
  await page.goto('/tests/fixtures/entity-history.html');
  await page.getByRole('button',{name:'Load field origins',exact:true}).click();
  await expect(page.getByLabel('Entity field origins')).toContainText('History is incomplete');
  await expect(page.getByLabel('Entity field origins')).toContainText('Origin unresolved');
  await expect(page.getByLabel('Entity field origins')).toContainText('Working edits are not included');
  await page.locator('.field-value summary').first().click();
  await expect(page.locator('.field-value pre').first()).toContainText('5000');
  await page.getByRole('button',{name:'Compare field change /p2',exact:true}).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#result')).toHaveText('11223344 → abcdef0123456789');
  await page.getByRole('button',{name:'Load field origins',exact:true}).click();
  await expect(page.locator('.entity-history-panel > p[role="status"]')).toContainText('Field origins are current');
  await page.getByLabel('History commit limit').fill('501');
  await expect(page.getByRole('button',{name:'Load field origins',exact:true})).toBeDisabled();
});
