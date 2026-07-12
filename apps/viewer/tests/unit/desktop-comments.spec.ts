import { expect, test } from "@playwright/test";
import { createDesktopComment, updateDesktopCommentStatus } from "../../src/desktop-comments";

test("uses comment create and status command contracts", async () => {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const invoke = async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    return { drawing: "plan_1f", revision: "next", comments: [] } as T;
  };

  await createDesktopComment("/tmp/project", {
    drawing: "plan_1f",
    expected_revision: "old",
    entity_id: "ent_1",
    anchor: { x: 1, y: 2 },
    text: "note",
  }, invoke);
  await updateDesktopCommentStatus("/tmp/project", {
    drawing: "plan_1f",
    expected_revision: "next",
    comment_id: "cmt_1",
    status: "resolved",
  }, invoke);

  expect(calls).toEqual([
    {
      command: "create_comment",
      args: {
        projectPath: "/tmp/project",
        request: {
          drawing: "plan_1f",
          expected_revision: "old",
          entity_id: "ent_1",
          anchor: { x: 1, y: 2 },
          text: "note",
        },
      },
    },
    {
      command: "update_comment_status",
      args: {
        projectPath: "/tmp/project",
        request: {
          drawing: "plan_1f",
          expected_revision: "next",
          comment_id: "cmt_1",
          status: "resolved",
        },
      },
    },
  ]);
});
