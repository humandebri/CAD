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
  configuration_changes: Array<{
    path: string;
    kind: "added" | "removed" | "modified";
    before?: unknown;
    after?: unknown;
  }>;
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
  layers: LayerWorkspaceState;
  drawingNames: string[];
  currentDrawing: string;
  editor: EditorDrawingState;
};

export type EditorEntity = Record<string, unknown> & {
  schema_version: string;
  id: string;
  type: string;
  layer: string;
  pen?: string | null;
};

export type EditorDrawingState = {
  drawing: string;
  revision: string;
  entities: EditorEntity[];
  text_styles: string[];
  dimension_styles: string[];
  pens: string[];
};

export type EditOperation =
  | { kind: "create"; entity: Record<string, unknown> }
  | { kind: "replace"; entity_id: string; entity: EditorEntity }
  | { kind: "translate"; entity_id: string; delta: [number, number]; duplicate: boolean }
  | { kind: "delete"; entity_id: string };

export type DrawingEditRequest = {
  drawing: string;
  expected_revision: string;
  operation: EditOperation;
};

export type DrawingEditResult = {
  drawing: string;
  revision: string;
  entity_id?: string | null;
  operation: string;
};

export type SnapKind = "endpoint" | "midpoint" | "intersection";

export type SnapCandidate = {
  kind: SnapKind;
  point: [number, number];
  distance: number;
};

export type LayerWorkspaceGroup = {
  id: string;
  name: string;
  order: number;
  scale_denominator: number;
  visible: boolean;
  locked: boolean;
};

export type LayerWorkspaceLayer = {
  id: string;
  name: string;
  group?: string | null;
  order: number;
  visible: boolean;
  locked: boolean;
  printable: boolean;
  used_entity_count: number;
};

export type LayerWorkspaceState = {
  revision: string;
  active_layer?: string | null;
  groups: LayerWorkspaceGroup[];
  layers: LayerWorkspaceLayer[];
};

export type LayerPatch = {
  id: string;
  visible?: boolean;
  locked?: boolean;
  printable?: boolean;
};

export type LayerRulesPatch = {
  expectedRevision: string;
  layers?: LayerPatch[];
  groups?: Array<{ id: string; visible?: boolean; locked?: boolean }>;
  activeLayer?: string;
};

export type ExportIssue = {
  code: string;
  message: string;
  entity_id?: string | null;
};

export type ExportReport = {
  schema_version: string;
  status: "exported" | "blocked";
  output_path: string;
  written_entities: number;
  expanded_entities: number;
  warnings: ExportIssue[];
  blockers: ExportIssue[];
};

export type ProjectState = {
  project_path: string;
  project_name: string;
  is_git_project: boolean;
  import_warning_count?: number | null;
};

export type DesktopReviewSnapshot = {
  artifacts: Artifacts;
  projectName: string;
};

export type ProjectWatchState = {
  project_path: string;
  status: "watching";
  debounce_ms: number;
};

export type ProjectWatchEvent = {
  kind: "changed" | "error";
  project_path: string;
  paths: string[];
  message?: string | null;
};

export type LiveReviewState = {
  status: "starting" | "watching" | "refreshing" | "error";
  message?: string;
};

export type AiContextState = {
  status: "ready" | "no_entity_selected" | "error";
  json_path?: string | null;
  markdown_path?: string | null;
  message?: string | null;
};

export type DesktopReviewArtifacts = {
  project_name: string;
  drawing_names: string[];
  current_drawing: string;
  sheet_svg: string;
  diff_svg: string | null;
  check: CheckReport;
  diff: DiffReport | null;
  comments: CommentRecord[];
  diff_unavailable: string | null;
  layers: LayerWorkspaceState;
  editor: EditorDrawingState;
};

export const emptyLayerWorkspace: LayerWorkspaceState = {
  revision: "",
  active_layer: null,
  groups: [],
  layers: [],
};

export function emptyDiffReport(message: string): DiffReport {
  return {
    schema_version: "0.2",
    status: message.length > 0 ? "warning" : "ok",
    changes: [],
    warnings: [],
    configuration_changes: [],
  };
}
