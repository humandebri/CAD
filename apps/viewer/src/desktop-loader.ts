/**
 * apps/viewer: desktop artifact loader for Tauri invoke transport.
 * Sanitization is injected so the transport can be unit-tested without a DOM.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { type Artifacts, type DesktopReviewArtifacts, emptyDiffReport } from "./artifacts";

export type InvokeReview = (
  command: string,
  args: Record<string, unknown>,
) => Promise<DesktopReviewArtifacts>;

export type SvgSanitizer = (svgText: string) => string;

const invokeReview: InvokeReview = (command, args) => tauriInvoke<DesktopReviewArtifacts>(command, args);

export async function loadArtifactsFromDesktop(
  projectPath: string,
  sanitizeSvg: SvgSanitizer,
  invokeCommand: InvokeReview = invokeReview,
): Promise<Artifacts> {
  const review = await invokeCommand("run_review", { projectPath });
  return {
    sheetSvg: sanitizeSvg(review.sheet_svg),
    diffSvg: sanitizeSvg(review.diff_svg ?? review.sheet_svg),
    check: review.check,
    diff: review.diff ?? emptyDiffReport(review.diff_unavailable ?? "diff unavailable"),
    comments: review.comments,
    diffUnavailable: review.diff_unavailable ?? undefined,
  };
}
