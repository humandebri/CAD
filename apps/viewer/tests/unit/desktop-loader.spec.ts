/**
 * apps/viewer: verifies the Tauri desktop loader with a mocked invoke transport.
 * The test keeps desktop IPC behavior covered without launching a Tauri window.
 */
import { expect, test } from "@playwright/test";
import {
  type DesktopReviewArtifacts,
  type DiffReport,
  type CheckReport,
} from "../../src/artifacts";
import { type InvokeReview, loadArtifactsFromDesktop } from "../../src/desktop-loader";

const checkReport: CheckReport = {
  schema_version: "0.1",
  status: "ok",
  diagnostics: [],
};

const diffReport: DiffReport = {
  schema_version: "0.1",
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
};

test("loads desktop artifacts through run_review invoke", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const review: DesktopReviewArtifacts = {
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
    diff_unavailable: null,
  };
  const invoke: InvokeReview = async (command, args) => {
    calls.push({ command, args });
    return review;
  };

  const artifacts = await loadArtifactsFromDesktop("/tmp/project", (svg) => `safe:${svg}`, invoke);

  expect(calls).toEqual([{ command: "run_review", args: { projectPath: "/tmp/project" } }]);
  expect(artifacts.sheetSvg).toBe("safe:<svg><g /></svg>");
  expect(artifacts.diffSvg).toBe("safe:<svg><g /></svg>");
  expect(artifacts.diff.changes).toHaveLength(1);
  expect(artifacts.comments).toHaveLength(1);
});

test("marks diff unavailable when desktop command has no diff report", async () => {
  const review: DesktopReviewArtifacts = {
    sheet_svg: "<svg />",
    diff_svg: null,
    check: checkReport,
    diff: null,
    comments: [],
    diff_unavailable: "project has no tracked files in Git",
  };
  const invoke: InvokeReview = async () => review;

  const artifacts = await loadArtifactsFromDesktop("/tmp/project", (svg) => svg, invoke);

  expect(artifacts.diff.status).toBe("warning");
  expect(artifacts.diff.changes).toEqual([]);
  expect(artifacts.diffUnavailable).toBe("project has no tracked files in Git");
});
