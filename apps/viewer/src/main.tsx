/**
 * apps/viewer: generated CAD review artifacts are loaded from fixed build paths.
 * The viewer keeps SVG interactive by inlining it and reading data-entity-id.
 */
import { render } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import {
  AlertTriangle,
  CheckCircle2,
  FileJson2,
  Layers3,
  Maximize2,
  MousePointer2,
  ZoomIn,
  ZoomOut,
} from "lucide-preact";
import "./styles.css";

type ViewMode = "sheet" | "diff";
type LoadState = "loading" | "ready" | "error";

type CheckDiagnostic = {
  code: string;
  message: string;
  severity: string;
  file?: string;
  line?: number;
  entity_id?: string;
  field?: string;
};

type CheckReport = {
  schema_version: string;
  status: string;
  diagnostics: CheckDiagnostic[];
};

type DiffChange = {
  entity_id: string;
  drawing: string;
  kind: string;
  reasons: string[];
};

type DiffWarning = {
  kind: string;
  entity_ids: string[];
  drawing: string;
  message: string;
};

type DiffReport = {
  schema_version: string;
  status: string;
  changes: DiffChange[];
  warnings: DiffWarning[];
};

type CommentRecord = {
  id: string;
  drawing: string;
  entity_ids: string[];
  text: string;
  status: string;
};

type Artifacts = {
  sheetSvg: string;
  diffSvg: string;
  check: CheckReport;
  diff: DiffReport;
  comments: CommentRecord[];
};

const paths = {
  sheetSvg: "/build/plan_1f.svg",
  diffSvg: "/build/plan_1f.diff.svg",
  check: "/build/check.json",
  diff: "/build/diff.json",
  comments: "/comments/plan_1f.ndjson",
};

function App() {
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [errorMessage, setErrorMessage] = useState("");
  const [artifacts, setArtifacts] = useState<Artifacts | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("diff");
  const [selectedEntityId, setSelectedEntityId] = useState("");
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [isPanning, setIsPanning] = useState(false);
  const panStart = useRef({ pointerX: 0, pointerY: 0, x: 0, y: 0 });

  useEffect(() => {
    loadArtifacts()
      .then((loaded) => {
        setArtifacts(loaded);
        setLoadState("ready");
      })
      .catch((error: unknown) => {
        setErrorMessage(error instanceof Error ? error.message : "unknown load error");
        setLoadState("error");
      });
  }, []);

  const activeSvg = viewMode === "sheet" ? artifacts?.sheetSvg : artifacts?.diffSvg;
  const selectedSummary = useMemo(() => {
    if (artifacts === null || selectedEntityId === "") {
      return null;
    }
    return {
      diagnostics: artifacts.check.diagnostics.filter(
        (diagnostic) => diagnostic.entity_id === selectedEntityId,
      ),
      changes: artifacts.diff.changes.filter((change) => change.entity_id === selectedEntityId),
      comments: artifacts.comments.filter((comment) =>
        comment.entity_ids.includes(selectedEntityId),
      ),
    };
  }, [artifacts, selectedEntityId]);

  useEffect(() => {
    document
      .querySelectorAll(".drawing-stage [data-entity-id]")
      .forEach((element) => element.classList.remove("is-selected"));
    if (selectedEntityId === "") {
      return;
    }
    document
      .querySelectorAll(`.drawing-stage [data-entity-id="${selectedEntityId}"]`)
      .forEach((element) => element.classList.add("is-selected"));
  }, [activeSvg, selectedEntityId]);

  function resetView() {
    setScale(1);
    setPan({ x: 0, y: 0 });
  }

  function selectEntity(entityId: string) {
    setSelectedEntityId(entityId);
  }

  function handleSvgClick(event: MouseEvent) {
    if (!(event.target instanceof Element)) {
      return;
    }
    const entityElement = event.target.closest("[data-entity-id]");
    const entityId = entityElement?.getAttribute("data-entity-id");
    if (entityId !== null && entityId !== undefined && entityId !== "") {
      selectEntity(entityId);
    }
  }

  return (
    <main class="app-shell">
      <header class="topbar">
        <div class="brand">
          <Layers3 size={20} aria-hidden="true" />
          <span>CAD Review</span>
        </div>
        <div class="mode-tabs" aria-label="SVG mode">
          <button
            type="button"
            class={viewMode === "sheet" ? "tab is-active" : "tab"}
            onClick={() => setViewMode("sheet")}
          >
            Sheet
          </button>
          <button
            type="button"
            class={viewMode === "diff" ? "tab is-active" : "tab"}
            onClick={() => setViewMode("diff")}
          >
            Diff
          </button>
        </div>
        <div class="view-tools" aria-label="View controls">
          <button
            type="button"
            class="icon-button"
            aria-label="Zoom out"
            title="Zoom out"
            onClick={() => setScale((current) => Math.max(0.5, current - 0.1))}
          >
            <ZoomOut size={18} aria-hidden="true" />
          </button>
          <output class="zoom-readout" aria-label="Zoom">
            {Math.round(scale * 100)}%
          </output>
          <button
            type="button"
            class="icon-button"
            aria-label="Zoom in"
            title="Zoom in"
            onClick={() => setScale((current) => Math.min(2.5, current + 0.1))}
          >
            <ZoomIn size={18} aria-hidden="true" />
          </button>
          <button
            type="button"
            class="icon-button"
            aria-label="Reset view"
            title="Reset view"
            onClick={resetView}
          >
            <Maximize2 size={18} aria-hidden="true" />
          </button>
        </div>
      </header>

      <section class="review-grid">
        <aside class="side-panel" aria-label="Review results">
          <PanelHeader artifacts={artifacts} loadState={loadState} />
          {artifacts !== null && (
            <ResultPanel
              artifacts={artifacts}
              selectedEntityId={selectedEntityId}
              onSelectEntity={selectEntity}
            />
          )}
        </aside>

        <section class="canvas-panel" aria-label="CAD paper">
          {loadState === "loading" && <div class="empty-state">Loading generated artifacts</div>}
          {loadState === "error" && <div class="empty-state is-error">{errorMessage}</div>}
          {loadState === "ready" && activeSvg !== undefined && (
            <div
              class={isPanning ? "drawing-stage is-panning" : "drawing-stage"}
              onMouseDown={(event) => {
                setIsPanning(true);
                panStart.current = {
                  pointerX: event.clientX,
                  pointerY: event.clientY,
                  x: pan.x,
                  y: pan.y,
                };
              }}
              onMouseMove={(event) => {
                if (!isPanning) {
                  return;
                }
                setPan({
                  x: panStart.current.x + event.clientX - panStart.current.pointerX,
                  y: panStart.current.y + event.clientY - panStart.current.pointerY,
                });
              }}
              onMouseUp={() => setIsPanning(false)}
              onMouseLeave={() => setIsPanning(false)}
            >
              <div
                class="svg-surface"
                style={{
                  transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
                }}
                onClick={handleSvgClick}
                dangerouslySetInnerHTML={{ __html: activeSvg }}
              />
            </div>
          )}
        </section>

        <aside class="detail-panel" aria-label="Selected entity">
          <h2>
            <MousePointer2 size={17} aria-hidden="true" />
            Selection
          </h2>
          {selectedEntityId === "" || selectedSummary === null ? (
            <p class="muted">Select an entity on the paper or from a result list.</p>
          ) : (
            <EntityDetails entityId={selectedEntityId} summary={selectedSummary} />
          )}
        </aside>
      </section>
    </main>
  );
}

function PanelHeader(props: { artifacts: Artifacts | null; loadState: LoadState }) {
  const { artifacts, loadState } = props;
  const status = artifacts === null ? loadState : `${artifacts.check.status} / ${artifacts.diff.status}`;
  return (
    <div class="panel-header">
      <div>
        <p class="eyebrow">Generated review</p>
        <h1>plan_1f</h1>
      </div>
      <span class="status-chip">{status}</span>
    </div>
  );
}

function ResultPanel(props: {
  artifacts: Artifacts;
  selectedEntityId: string;
  onSelectEntity: (entityId: string) => void;
}) {
  const { artifacts, selectedEntityId, onSelectEntity } = props;
  return (
    <div class="result-stack">
      <section class="result-section">
        <h2>
          <FileJson2 size={17} aria-hidden="true" />
          Diff
        </h2>
        <div class="result-list">
          {artifacts.diff.changes.map((change) => (
            <button
              type="button"
              class={
                selectedEntityId === change.entity_id
                  ? `result-row ${change.kind} is-selected`
                  : `result-row ${change.kind}`
              }
              key={change.entity_id}
              onClick={() => onSelectEntity(change.entity_id)}
            >
              <span class="row-kind">{change.kind}</span>
              <span class="entity-id">{change.entity_id}</span>
              <span class="row-note">{formatReasons(change.reasons)}</span>
            </button>
          ))}
        </div>
      </section>

      <section class="result-section">
        <h2>
          <AlertTriangle size={17} aria-hidden="true" />
          Check
        </h2>
        {artifacts.check.diagnostics.length === 0 ? (
          <p class="success-line">
            <CheckCircle2 size={16} aria-hidden="true" />
            No diagnostics
          </p>
        ) : (
          <div class="result-list">
            {artifacts.check.diagnostics.map((diagnostic) => (
              <button
                type="button"
                class="result-row diagnostic"
                key={`${diagnostic.code}-${diagnostic.file ?? "project"}-${diagnostic.line ?? 0}`}
                onClick={() => {
                  if (diagnostic.entity_id !== undefined) {
                    onSelectEntity(diagnostic.entity_id);
                  }
                }}
              >
                <span class="row-kind">{diagnostic.severity}</span>
                <span class="entity-id">{diagnostic.entity_id ?? diagnostic.code}</span>
                <span class="row-note">{diagnostic.message}</span>
              </button>
            ))}
          </div>
        )}
      </section>

      <section class="result-section">
        <h2>Comments</h2>
        <div class="comment-list">
          {artifacts.comments.map((comment) => (
            <button
              type="button"
              class="comment-row"
              key={comment.id}
              onClick={() => onSelectEntity(comment.entity_ids[0] ?? "")}
            >
              <span>{comment.text}</span>
              <small>{comment.status}</small>
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}

function EntityDetails(props: {
  entityId: string;
  summary: {
    diagnostics: CheckDiagnostic[];
    changes: DiffChange[];
    comments: CommentRecord[];
  };
}) {
  const { entityId, summary } = props;
  return (
    <div class="detail-stack">
      <code>{entityId}</code>
      <section>
        <h3>Diff</h3>
        {summary.changes.length === 0 ? (
          <p class="muted">No diff entry.</p>
        ) : (
          summary.changes.map((change) => (
            <p class="detail-line" key={`${change.entity_id}-${change.kind}`}>
              <strong>{change.kind}</strong>
              <span>{formatReasons(change.reasons)}</span>
            </p>
          ))
        )}
      </section>
      <section>
        <h3>Check</h3>
        {summary.diagnostics.length === 0 ? (
          <p class="muted">No diagnostics.</p>
        ) : (
          summary.diagnostics.map((diagnostic) => (
            <p class="detail-line" key={`${diagnostic.code}-${diagnostic.line ?? 0}`}>
              <strong>{diagnostic.code}</strong>
              <span>{diagnostic.message}</span>
            </p>
          ))
        )}
      </section>
      <section>
        <h3>Comments</h3>
        {summary.comments.length === 0 ? (
          <p class="muted">No comments.</p>
        ) : (
          summary.comments.map((comment) => (
            <p class="detail-line" key={comment.id}>
              <strong>{comment.status}</strong>
              <span>{comment.text}</span>
            </p>
          ))
        )}
      </section>
    </div>
  );
}

async function loadArtifacts(): Promise<Artifacts> {
  const [sheetSvg, diffSvg, checkValue, diffValue, commentsText] = await Promise.all([
    fetchText(paths.sheetSvg),
    fetchText(paths.diffSvg),
    fetchJson(paths.check),
    fetchJson(paths.diff),
    fetchText(paths.comments),
  ]);
  return {
    sheetSvg: sanitizeSvg(sheetSvg),
    diffSvg: sanitizeSvg(diffSvg),
    check: parseCheckReport(checkValue),
    diff: parseDiffReport(diffValue),
    comments: parseComments(commentsText),
  };
}

function sanitizeSvg(svgText: string): string {
  const document = new DOMParser().parseFromString(svgText, "image/svg+xml");
  if (document.querySelector("parsererror") !== null) {
    throw new Error("SVG parse failed");
  }
  document.querySelectorAll("script, foreignObject").forEach((element) => element.remove());
  document.querySelectorAll("*").forEach((element) => {
    Array.from(element.attributes).forEach((attribute) => {
      const attributeName = attribute.name.toLowerCase();
      const attributeValue = attribute.value.trim().toLowerCase();
      if (attributeName.startsWith("on") || attributeValue.startsWith("javascript:")) {
        element.removeAttribute(attribute.name);
      }
    });
  });
  return new XMLSerializer().serializeToString(document.documentElement);
}

async function fetchText(path: string): Promise<string> {
  const response = await fetch(path);
  if (!response.ok) {
    throw new Error(`${path} returned ${response.status}`);
  }
  return response.text();
}

async function fetchJson(path: string): Promise<unknown> {
  const response = await fetch(path);
  if (!response.ok) {
    throw new Error(`${path} returned ${response.status}`);
  }
  return response.json();
}

function parseCheckReport(value: unknown): CheckReport {
  if (!isRecord(value)) {
    throw new Error("check.json is not an object");
  }
  return {
    schema_version: readString(value.schema_version),
    status: readString(value.status),
    diagnostics: readArray(value.diagnostics).map(parseCheckDiagnostic),
  };
}

function parseCheckDiagnostic(value: unknown): CheckDiagnostic {
  if (!isRecord(value)) {
    throw new Error("diagnostic is not an object");
  }
  return {
    code: readString(value.code),
    message: readString(value.message),
    severity: readString(value.severity),
    file: readOptionalString(value.file),
    line: readOptionalNumber(value.line),
    entity_id: readOptionalString(value.entity_id),
    field: readOptionalString(value.field),
  };
}

function parseDiffReport(value: unknown): DiffReport {
  if (!isRecord(value)) {
    throw new Error("diff.json is not an object");
  }
  return {
    schema_version: readString(value.schema_version),
    status: readString(value.status),
    changes: readArray(value.changes).map(parseDiffChange),
    warnings: readArray(value.warnings).map(parseDiffWarning),
  };
}

function parseDiffChange(value: unknown): DiffChange {
  if (!isRecord(value)) {
    throw new Error("diff change is not an object");
  }
  return {
    entity_id: readString(value.entity_id),
    drawing: readString(value.drawing),
    kind: readString(value.kind),
    reasons: readArray(value.reasons).map(readString),
  };
}

function parseDiffWarning(value: unknown): DiffWarning {
  if (!isRecord(value)) {
    throw new Error("diff warning is not an object");
  }
  return {
    kind: readString(value.kind),
    entity_ids: readArray(value.entity_ids).map(readString),
    drawing: readString(value.drawing),
    message: readString(value.message),
  };
}

function parseComments(text: string): CommentRecord[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => parseComment(JSON.parse(line)));
}

function parseComment(value: unknown): CommentRecord {
  if (!isRecord(value)) {
    throw new Error("comment is not an object");
  }
  return {
    id: readString(value.id),
    drawing: readString(value.drawing),
    entity_ids: readArray(value.entity_ids).map(readString),
    text: readString(value.text),
    status: readString(value.status),
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readArray(value: unknown): unknown[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value;
}

function readString(value: unknown): string {
  if (typeof value !== "string") {
    return "";
  }
  return value;
}

function readOptionalString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function readOptionalNumber(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined;
}

function formatReasons(reasons: string[]): string {
  if (reasons.length === 0) {
    return "no semantic reason";
  }
  return reasons.join(", ");
}

const root = document.getElementById("app");

if (root === null) {
  throw new Error("Missing #app root element");
}

render(<App />, root);
