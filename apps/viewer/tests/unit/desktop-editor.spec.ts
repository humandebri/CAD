import { expect, test } from "@playwright/test";
import { expectedRevisionForOperation, shouldRefreshDesktopReview } from "../../src/desktop-editor";

test("refreshes desktop review after stale PDF or history errors", () => {
  expect(shouldRefreshDesktopReview("revision_conflict: source changed")).toBe(true);
  expect(shouldRefreshDesktopReview("history unavailable")).toBe(true);
  expect(shouldRefreshDesktopReview("output already exists")).toBe(false);
});

test("selects the revision owned by each edit target", () => {
  const blocks = [{ id: "door", name: "Door", entity_count: 1, revision: "block-rev" }];
  const layouts = [{
    id: "default",
    paper: "A3",
    orientation: "landscape" as const,
    scale: "1/100",
    origin: [0, 0] as [number, number],
    margins: [0, 0, 0, 0] as [number, number, number, number],
    plot_area: null,
    active: true,
    revision: "layout-rev",
  }];
  expect(expectedRevisionForOperation(
    { kind: "delete", entity_id: "ent_1" },
    "entity-rev",
    blocks,
    layouts,
  )).toBe("entity-rev");
  expect(expectedRevisionForOperation(
    { kind: "update_block_definition", block: "door", properties: { name: "Updated" } },
    "entity-rev",
    blocks,
    layouts,
  )).toBe("block-rev");
  expect(expectedRevisionForOperation(
    { kind: "update_layout", layout: "default", properties: { scale: "1/50" } },
    "entity-rev",
    blocks,
    layouts,
  )).toBe("layout-rev");
  expect(expectedRevisionForOperation(
    { kind: "update_block_definition", block: "missing", properties: {} },
    "entity-rev",
    blocks,
    layouts,
  )).toBeNull();
});
