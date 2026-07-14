import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  BlockWorkspaceState,
  DrawingHistoryRequest,
  DrawingHistoryState,
  DrawingEditRequest,
  DrawingEditResult,
  EditOperation,
  LayoutWorkspaceState,
  SnapCandidate,
  SnapKind,
} from "./artifacts";

export type InvokeEditor = <T>(
  command: string,
  args: Record<string, unknown>,
) => Promise<T>;

const invokeEditor: InvokeEditor = (command, args) => tauriInvoke(command, args);

export function shouldRefreshDesktopReview(message: string): boolean {
  return message.includes("revision_conflict") || message.includes("history");
}

export function expectedRevisionForOperation(
  operation: EditOperation,
  editorRevision: string,
  blocks: BlockWorkspaceState[] = [],
  layouts: LayoutWorkspaceState[] = [],
): string | null {
  if (operation.kind === "update_block_definition") {
    return blocks.find((block) => block.id === operation.block)?.revision ?? null;
  }
  if (operation.kind === "update_layout") {
    return layouts.find((layout) => layout.id === operation.layout)?.revision
      ?? layouts[0]?.revision
      ?? null;
  }
  return editorRevision;
}

export function applyDrawingEdit(
  projectPath: string,
  request: DrawingEditRequest,
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<DrawingEditResult> {
  return invokeCommand<DrawingEditResult>("apply_drawing_edit", { projectPath, request });
}

export function undoDrawingEdit(
  projectPath: string,
  request: DrawingHistoryRequest,
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<DrawingEditResult> {
  return invokeCommand<DrawingEditResult>("undo_drawing_edit", { projectPath, request });
}

export function redoDrawingEdit(
  projectPath: string,
  request: DrawingHistoryRequest,
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<DrawingEditResult> {
  return invokeCommand<DrawingEditResult>("redo_drawing_edit", { projectPath, request });
}

export function listDrawingHistory(
  projectPath: string,
  drawing: string,
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<DrawingHistoryState> {
  return invokeCommand<DrawingHistoryState>("list_drawing_history", { projectPath, drawing });
}

export function clearDrawingHistory(
  projectPath: string,
  drawing: string,
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<void> {
  return invokeCommand<void>("clear_drawing_history", { projectPath, drawing });
}

export function queryDrawingSnap(
  projectPath: string,
  drawing: string,
  revision: string,
  point: [number, number],
  toleranceMm: number,
  modes: SnapKind[] = ["endpoint", "midpoint", "intersection"],
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<SnapCandidate | null> {
  return invokeCommand<SnapCandidate | null>("query_snap", {
    projectPath,
    drawing,
    revision,
    point,
    toleranceMm,
    modes,
  });
}
