/**
 * apps/viewer: desktop artifact loader for Tauri invoke transport.
 * Sanitization is injected so the transport can be unit-tested without a DOM.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
  type DesktopReviewArtifacts,
  type DesktopReviewSnapshot,
  emptyDiffReport,
} from "./artifacts";

export type InvokeReview = (
  command: string,
  args: Record<string, unknown>,
) => Promise<DesktopReviewArtifacts>;

export type SvgSanitizer = (svgText: string) => string;

const invokeReview: InvokeReview = (command, args) => tauriInvoke<DesktopReviewArtifacts>(command, args);

export type GitComparison = { base: string; head: string };

export async function loadReviewSnapshotFromDesktop(
  projectPath: string,
  sanitizeSvg: SvgSanitizer,
  invokeCommand: InvokeReview = invokeReview,
  drawingName?: string,
  comparison?: GitComparison,
): Promise<DesktopReviewSnapshot> {
  const args: Record<string, unknown> = drawingName === undefined ? { projectPath } : { projectPath, drawingName };
  if (comparison !== undefined) {
    args.baseRevision = comparison.base;
    args.headRevision = comparison.head;
  }
  const review = await invokeCommand(
    "run_review",
    args,
  );
  return {
    projectName: review.project_name,
    artifacts: {
      sheetSvg: sanitizeSvg(review.sheet_svg),
      diffSvg: review.diff_svg === null
        ? ""
        : sanitizeSvg(review.diff_svg),
      check: review.check,
      diff: review.diff ?? emptyDiffReport(review.diff_unavailable ?? "diff unavailable"),
      comments: review.comments,
      commentsRevision: review.comments_revision,
      diffUnavailable: review.diff_unavailable ?? undefined,
      layers: review.layers,
      drawingNames: review.drawing_names,
      currentDrawing: review.current_drawing,
      editor: review.editor,
      blocks: review.blocks ?? [],
      layouts: review.layouts ?? [],
      comparison: review.comparison ?? undefined,
    },
  };
}
