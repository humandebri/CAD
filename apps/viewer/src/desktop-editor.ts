import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  DrawingEditRequest,
  DrawingEditResult,
  SnapCandidate,
  SnapKind,
} from "./artifacts";

export type InvokeEditor = <T>(
  command: string,
  args: Record<string, unknown>,
) => Promise<T>;

const invokeEditor: InvokeEditor = (command, args) => tauriInvoke(command, args);

export function applyDrawingEdit(
  projectPath: string,
  request: DrawingEditRequest,
  invokeCommand: InvokeEditor = invokeEditor,
): Promise<DrawingEditResult> {
  return invokeCommand<DrawingEditResult>("apply_drawing_edit", { projectPath, request });
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
