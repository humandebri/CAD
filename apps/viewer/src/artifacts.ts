/**
 * apps/viewer: shared review artifact contracts used by web and desktop loaders.
 * The shapes mirror Rust JSON output so UI code can stay transport-agnostic.
 */

export type CheckDiagnostic = {
  code: string;
  message: string;
  severity: string;
  file?: string;
  line?: number;
  entity_id?: string;
  field?: string;
};

export type CheckReport = {
  schema_version: string;
  status: string;
  diagnostics: CheckDiagnostic[];
};

export type DiffChange = {
  entity_id: string;
  drawing: string;
  kind: string;
  reasons: string[];
};

export type DiffWarning = {
  kind: string;
  entity_ids: string[];
  drawing: string;
  message: string;
};

export type DiffReport = {
  schema_version: string;
  status: string;
  changes: DiffChange[];
  warnings: DiffWarning[];
};

export type CommentRecord = {
  id: string;
  drawing: string;
  entity_ids: string[];
  text: string;
  status: string;
};

export type Artifacts = {
  sheetSvg: string;
  diffSvg: string;
  check: CheckReport;
  diff: DiffReport;
  comments: CommentRecord[];
  diffUnavailable?: string;
};

export type ProjectState = {
  project_path: string;
  project_name: string;
  is_git_project: boolean;
  import_warning_count?: number | null;
};

export type AiContextState = {
  status: "ready" | "no_entity_selected" | "error";
  json_path?: string | null;
  markdown_path?: string | null;
  message?: string | null;
};

export type DesktopReviewArtifacts = {
  sheet_svg: string;
  diff_svg: string | null;
  check: CheckReport;
  diff: DiffReport | null;
  comments: CommentRecord[];
  diff_unavailable: string | null;
};

export function emptyDiffReport(message: string): DiffReport {
  return {
    schema_version: "0.1",
    status: message.length > 0 ? "warning" : "ok",
    changes: [],
    warnings: [],
  };
}
