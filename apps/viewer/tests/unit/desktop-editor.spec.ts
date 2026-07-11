import { expect, test } from "@playwright/test";
import { applyDrawingEdit, queryDrawingSnap, type InvokeEditor } from "../../src/desktop-editor";

test("uses drawing edit command contract", async () => {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const invoke: InvokeEditor = async <T,>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    return { drawing: "plan", revision: "next", entity_id: "ent_new", operation: "create" } as T;
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
