import { expect, test } from "@playwright/test";
import {
  applyDrawingEdit,
  clearDrawingHistory,
  expectedRevisionForOperation,
  listDrawingHistory,
  queryDrawingSnap,
  redoDrawingEdit,
  shouldRefreshDesktopReview,
  undoDrawingEdit,
  type InvokeEditor,
} from "../../src/desktop-editor";

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

test("uses drawing edit command contract", async () => {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const invoke: InvokeEditor = async <T,>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    return {
      drawing: "plan",
      revision: "next",
      entity_id: "ent_new",
      entity_ids: ["ent_new"],
      operation: "create",
      history_id: null,
    } as T;
  };
  const request = {
    drawing: "plan",
    expected_revision: "base",
    operation: { kind: "create" as const, entity: { type: "point", layer: "0-1", at: [1, 2] } },
  };

  const result = await applyDrawingEdit("/tmp/project", request, invoke);

  expect(result.revision).toBe("next");
  expect(calls).toEqual([{ command: "apply_drawing_edit", args: { projectPath: "/tmp/project", request } }]);
});

test("uses snap query command contract", async () => {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const invoke: InvokeEditor = async <T,>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    return { kind: "endpoint", point: [1, 2], distance: 0.1 } as T;
  };

  const result = await queryDrawingSnap("/tmp/project", "plan", "rev", [1, 2], 0.5, ["endpoint"], invoke);

  expect(result?.kind).toBe("endpoint");
  expect(calls[0]).toEqual({
    command: "query_snap",
    args: {
      projectPath: "/tmp/project",
      drawing: "plan",
      revision: "rev",
      point: [1, 2],
      toleranceMm: 0.5,
      modes: ["endpoint"],
    },
  });
});

test("uses undo, redo, and history list command contracts", async () => {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const invoke: InvokeEditor = async <T,>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === "list_drawing_history") {
      return { drawing: "plan", undo: [], redo: [], limit: 100 } as T;
    }
    return {
      drawing: "plan",
      revision: "next",
      entity_id: null,
      entity_ids: [],
      operation: command === "undo_drawing_edit" ? "undo" : "redo",
      history_id: "hist_1",
    } as T;
  };
  const request = { drawing: "plan", expected_revision: "base" };

  await undoDrawingEdit("/tmp/project", request, invoke);
  await redoDrawingEdit("/tmp/project", request, invoke);
  await listDrawingHistory("/tmp/project", "plan", invoke);
  await clearDrawingHistory("/tmp/project", "plan", invoke);

  expect(calls).toEqual([
    { command: "undo_drawing_edit", args: { projectPath: "/tmp/project", request } },
    { command: "redo_drawing_edit", args: { projectPath: "/tmp/project", request } },
    { command: "list_drawing_history", args: { projectPath: "/tmp/project", drawing: "plan" } },
    { command: "clear_drawing_history", args: { projectPath: "/tmp/project", drawing: "plan" } },
  ]);
});
