import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { CommentRecord } from "./artifacts";

export type CommentMutationResult = {
  drawing: string;
  revision: string;
  comments: CommentRecord[];
  history_id: string | null;
  changed_files: string[];
};

type InvokeComments = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

const invokeComments: InvokeComments = (command, args) => tauriInvoke(command, args);

export function createDesktopComment(
  projectPath: string,
  request: {
    drawing: string;
    expected_revision: string;
    entity_id: string;
    anchor: { x: number; y: number };
    text: string;
  },
  invokeCommand: InvokeComments = invokeComments,
): Promise<CommentMutationResult> {
  return invokeCommand<CommentMutationResult>("create_comment", { projectPath, request });
}

export function updateDesktopCommentStatus(
  projectPath: string,
  request: {
    drawing: string;
    expected_revision: string;
    comment_id: string;
    status: "open" | "resolved";
  },
  invokeCommand: InvokeComments = invokeComments,
): Promise<CommentMutationResult> {
  return invokeCommand<CommentMutationResult>("update_comment_status", { projectPath, request });
}
