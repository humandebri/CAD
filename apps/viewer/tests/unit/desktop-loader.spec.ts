/**
 * apps/viewer: verifies the Tauri desktop loader with a mocked invoke transport.
 * The test keeps desktop IPC behavior covered without launching a Tauri window.
 */
import { expect, test } from "@playwright/test";
import { type DesktopReviewArtifacts, type DiffReport, type CheckReport } from "../../src/artifacts";
import { type InvokeReview, loadReviewSnapshotFromDesktop } from "../../src/desktop-loader";

const checkReport: CheckReport = {
  schema_version: "0.3",
  status: "ok",
  diagnostics: [],
};

const diffReport: DiffReport = {
  schema_version: "0.3",
  status: "ok",
  changes: [
    {
      entity_id: "ent_01JZ0000000000000000000000",
      drawing: "plan_1f",
      kind: "modified",
      reasons: ["geometry_changed"],
    },
  ],
  warnings: [],
  configuration_changes: [],
};

test("loads desktop artifacts through run_review invoke", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const review: DesktopReviewArtifacts = {
    project_name: "reviewed-project",
    drawing_names: ["plan_1f"],
    current_drawing: "plan_1f",
    sheet_svg: "<svg><g /></svg>",
    diff_svg: null,
    check: checkReport,
    diff: diffReport,
    comments: [
      {
        id: "cmt_01JZ0000000000000000000000",
        drawing: "plan_1f",
        entity_ids: ["ent_01JZ0000000000000000000000"],
        text: "comment",
        status: "open",
      },
    ],
    comments_revision: "comments-revision",
    diff_unavailable: null,
    layers: { revision: "revision", active_layer: null, groups: [], layers: [] },
    editor: { drawing: "plan_1f", revision: "rev", entities: [], text_styles: [], dimension_styles: [], pens: [], fills: [] },
  };
  const invoke: InvokeReview = async (command, args) => {
    calls.push({ command, args });
    return review;
  };

  const snapshot = await loadReviewSnapshotFromDesktop(
    "/tmp/project",
    (svg) => `safe:${svg}`,
    invoke,
  );

  expect(calls).toEqual([{ command: "run_review", args: { projectPath: "/tmp/project" } }]);
  expect(snapshot.projectName).toBe("reviewed-project");
  expect(snapshot.artifacts.sheetSvg).toBe("safe:<svg><g /></svg>");
  expect(snapshot.artifacts.diffSvg).toBe("");
  expect(snapshot.artifacts.diff.changes).toHaveLength(1);
  expect(snapshot.artifacts.comments).toHaveLength(1);
});

test("marks diff unavailable when desktop command has no diff report", async () => {
  const review: DesktopReviewArtifacts = {
    project_name: "reviewed-project",
    drawing_names: ["plan_1f"],
    current_drawing: "plan_1f",
    sheet_svg: "<svg />",
    diff_svg: null,
    check: checkReport,
    diff: null,
    comments: [],
    comments_revision: "comments-revision",
    diff_unavailable: "project has no tracked files in Git",
    layers: { revision: "revision", active_layer: null, groups: [], layers: [] },
    editor: { drawing: "plan_1f", revision: "rev", entities: [], text_styles: [], dimension_styles: [], pens: [], fills: [] },
  };
  const invoke: InvokeReview = async () => review;

  const snapshot = await loadReviewSnapshotFromDesktop("/tmp/project", (svg) => svg, invoke);

  expect(snapshot.artifacts.diff.status).toBe("warning");
  expect(snapshot.artifacts.diff.changes).toEqual([]);
  expect(snapshot.artifacts.diffUnavailable).toBe("project has no tracked files in Git");
});
