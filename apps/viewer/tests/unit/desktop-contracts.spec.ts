import { expect, test } from "@playwright/test";
import { writeAiContextFromDesktop } from "../../src/desktop-ai-context";
import { createDesktopComment, updateDesktopCommentStatus } from "../../src/desktop-comments";
import { applyDrawingEdit, previewDrawingEdit, queryDrawingSnap, undoDrawingEdit, redoDrawingEdit, listDrawingHistory } from "../../src/desktop-editor";
import { updateLayerRulesFromDesktop, exportJwwFromDesktop, exportJwwPreservingFromDesktop, extractOriginalJwwFromDesktop, exportDrawingPdfFromDesktop } from "../../src/desktop-layers";
import { importJwwFromDesktop } from "../../src/desktop-import";
import { startProjectWatch, stopProjectWatch } from "../../src/desktop-live-review";

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
const projectPath = "/project";
const edit = { drawing: "plan", expected_revision: "before", operation: { kind: "rectangle" as const, layer: "0-1", p1: [0, 0] as [number, number], p2: [300, 200] as [number, number] } };
const history = { drawing: "plan", expected_revision: "before" };
const comment = { drawing: "plan", expected_revision: "before", entity_id: "ent_1", anchor: { x: 1, y: 2 }, text: "note" };
const status = { drawing: "plan", expected_revision: "after", comment_id: "cmt_1", status: "resolved" as const };
const expectedFiles = [{ relative_path: "drawings/plan/entities.ndjson", revision: "before", exists: true }];
const cases: Array<[string, (invoke: Invoke) => Promise<unknown>, Record<string, unknown> | undefined]> = [
  ["write_ai_context", invoke => writeAiContextFromDesktop(projectPath, "diff", "ent_1", invoke), { projectPath, viewMode: "diff", selectedEntityId: "ent_1" }],
  ["create_comment", invoke => createDesktopComment(projectPath, comment, invoke), { projectPath, request: comment }],
  ["update_comment_status", invoke => updateDesktopCommentStatus(projectPath, status, invoke), { projectPath, request: status }],
  ["apply_drawing_edit", invoke => applyDrawingEdit(projectPath, edit, invoke), { projectPath, request: edit }],
  ["preview_drawing_edit", invoke => previewDrawingEdit(projectPath, edit, invoke), { projectPath, request: edit }],
  ["query_snap", invoke => queryDrawingSnap(projectPath, "plan", "before", [1, 2], 0.5, ["endpoint"], invoke), { projectPath, drawing: "plan", revision: "before", point: [1, 2], toleranceMm: 0.5, modes: ["endpoint"] }],
  ["undo_drawing_edit", invoke => undoDrawingEdit(projectPath, history, invoke), { projectPath, request: history }],
  ["redo_drawing_edit", invoke => redoDrawingEdit(projectPath, history, invoke), { projectPath, request: history }],
  ["list_drawing_history", invoke => listDrawingHistory(projectPath, "plan", invoke), { projectPath, drawing: "plan" }],
  ["update_layer_rules", invoke => updateLayerRulesFromDesktop(projectPath, { drawing: "plan", expectedRevision: "before", activeLayer: "a", layers: [{ id: "a", visible: false }] }, invoke), { projectPath, patch: { drawing: "plan", layers: [{ id: "a", visible: false }], groups: [], active_layer: "a", expected_revision: "before" } }],
  ["export_jww", invoke => exportJwwFromDesktop(projectPath, "plan", "/out.jww", true, false, invoke), { projectPath, drawing: "plan", outputPath: "/out.jww", allowLossy: true, overwrite: false }],
  ["export_jww_preserving", invoke => exportJwwPreservingFromDesktop(projectPath, "plan", "/out.jww", false, invoke), { projectPath, drawing: "plan", outputPath: "/out.jww", overwrite: false }],
  ["extract_original_jww", invoke => extractOriginalJwwFromDesktop(projectPath, "/original.jww", true, invoke), { projectPath, outputPath: "/original.jww", overwrite: true }],
  ["export_drawing_pdf", invoke => exportDrawingPdfFromDesktop({ projectPath, drawing: "plan", outputPath: "/out.pdf", overwrite: false }, invoke), { projectPath, drawing: "plan", layout: null, outputPath: "/out.pdf", overwrite: false, expectedFiles: [] }],
  ["export_drawing_pdf", invoke => exportDrawingPdfFromDesktop({ projectPath, drawing: "plan", layout: "print", outputPath: "/out.pdf", overwrite: true, expectedFiles }, invoke), { projectPath, drawing: "plan", layout: "print", outputPath: "/out.pdf", overwrite: true, expectedFiles }],
  ["import_jww", invoke => importJwwFromDesktop("/Test1.jww", projectPath, invoke), { jwwPath: "/Test1.jww", outDir: "/project/test1_imported" }],
  ["start_project_watch", invoke => startProjectWatch(projectPath, invoke), { projectPath }],
  ["stop_project_watch", invoke => stopProjectWatch(invoke), undefined],
];

test("desktop IPC keeps command names, argument names, and defaults", async () => {
  for (const [command, call, args] of cases) {
    const calls: unknown[] = [];
    const invoke: Invoke = async <T>(name: string, values?: Record<string, unknown>) => {
      calls.push({ command: name, args: values });
      return undefined as T;
    };
    await call(invoke);
    expect(calls, command).toEqual([{ command, args }]);
  }
});
