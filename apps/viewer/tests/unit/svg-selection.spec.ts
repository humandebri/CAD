import { expect, test } from "@playwright/test";
import { closestEntityId } from "../../src/svg-selection";

test("selects the block reference when clicking expanded child geometry", async ({ page }) => {
  await page.setContent(`
    <svg>
      <g data-entity-id="block-ref">
        <g data-block-child-id="definition-child"><path id="child" /></g>
      </g>
    </svg>
  `);
  const child = await page.locator("#child").elementHandle();
  expect(child).not.toBeNull();
  const entityId = await page.evaluate(closestEntityId, child);
  expect(entityId).toBe("block-ref");
});
