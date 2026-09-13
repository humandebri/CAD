/**
 * apps/viewer: generated CAD review artifacts are loaded from fixed build paths.
 * The viewer keeps SVG interactive by inlining it and reading data-entity-id.
 */
import { render, type ComponentChildren } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { invoke } from "@tauri-apps/api/core";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  Circle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  FileInput,
  FilePlus2,
  FileJson2,
  FileOutput,
  FolderOpen,
  Copy,
  Minus,
  Layers3,
  Lock,
  Maximize2,
  MousePointer2,
  PenLine,
  Plus,
  Redo2,
  RefreshCw,
  RotateCcw,
  Ruler,
  Scan,
  ScanSearch,
  Undo2,
  Unlock,
  Waypoints,
  Type as TypeIcon,
  ZoomIn,
  ZoomOut,
} from "lucide-preact";
import {
  type Artifacts,
  type AiContextState,
  type CheckDiagnostic,
  type CheckReport,
  type CommentRecord,
  type DiffChange,
  type DiffReport,
  type DiffWarning,
  type LiveReviewState,
  type ExportReport,
  type EditOperation,
  type DrawingEditPreview,
  type DrawingHistoryState,
  type EditorEntity,
  type LayerRulesPatch,
  type LayerWorkspaceState,
  type ProjectState,
  type ProjectWatchEvent,
  type SnapCandidate,
  emptyLayerWorkspace,
} from "./artifacts";
import { formatError } from "./app-errors";
import { commandForKeyboardEvent, commandFromText, commandLabel, parseCoordinateInput, type CadCommand } from "./command-registry";
import { closestEntityId } from "./svg-selection";
import { DraftingPanel, initialDraftingOptions, finiteDraftNumber, repeatableCommand, retuneDraftOperation } from "./drafting-panel";
import {
  applyDrawingEdit,
  previewDrawingEdit,
  expectedRevisionForOperation,
  listDrawingHistory,
  queryDrawingSnap,
  redoDrawingEdit,
  shouldRefreshDesktopReview,
  undoDrawingEdit,
} from "./desktop-editor";
import { createDesktopComment, updateDesktopCommentStatus } from "./desktop-comments";
import { LatestAiContextWriteQueue } from "./desktop-ai-context";
import { importJwwFromDesktop } from "./desktop-import";
import { loadReviewSnapshotFromDesktop } from "./desktop-loader";
import {
  exportDrawingPdfFromDesktop,
  exportJwwFromDesktop,
  exportJwwPreservingFromDesktop,
  extractOriginalJwwFromDesktop,
  LatestLayerRulesQueue,
  PdfExportGuard,
} from "./desktop-layers";
import {
  DrawingLoadGuard,
  LatestProjectPersistenceQueue,
  LatestReviewQueue,
  mergeProjectStateFromReview,
  ProjectOpenGuard,
  shouldPreserveView,
  startProjectWatchWithCatchUp,
  stopProjectWatch,
  subscribeProjectWatch,
} from "./desktop-live-review";
import {
  beginWheelHistory,
  committedWheelPrevious,
  swapPreviousView,
} from "./view-history";
import "./styles.css";

if (import.meta.env.VITE_CAD_DESKTOP_E2E === "1") {
  void import("@wdio/tauri-plugin");
}
import {
  type BBox,
  type Point,
  type ViewBox,
  bboxFromElements,
  centerViewBoxAtPoint,
  classifyJwCadGesture,
  computeFitSvgViewBox,
  computeFitViewBox,
  formatViewBox,
  formatZoomScale,
  GESTURE_THRESHOLD_PX,
  MAX_ZOOM_SCALE,
  MIN_ZOOM_SCALE,
  panViewBox,
  parseViewBox,
  viewBoxZoomScale,
  wheelDeltaToScaleFactor,
  ZOOM_STEP_FACTOR,
  zoomViewBoxAtPoint,
} from "./view-fit";

type ViewMode = "sheet" | "diff";
type LoadState = "idle" | "loading" | "ready" | "error";
type DragMode = "pan" | "jw-gesture" | "zoom-area" | "selection-area" | "right-wait" | "edit-move" | "edit-endpoint";
type EditorMode =
  | "endpoint" | "stretch" | "rectangle" | "fillet" | "chamfer" | "rectangular_array" | "create_block"
  | "select"
  | "move"
  | "copy"
  | "line"
  | "polyline"
  | "circle"
  | "arc"
  | "text"
  | "dimension"
  | "rotate"
  | "mirror"
  | "offset"
  | "trim"
  | "extend"
  | "point"
  | "insert_block"
  | "edit_block"
  | "hatch"
  | "layout"
  | "print_preview";

type DragInteraction = {
  pointerId: number;
  mode: DragMode;
  startClient: Point;
  endClient: Point;
  startViewBox: ViewBox;
  moved: boolean;
};

type SelectionRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type ProjectSetupMode = "new" | "add" | "duplicate";

const PAN_DRAG_THRESHOLD_PX = 3;
const WHEEL_HISTORY_DELAY_MS = 150;

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
  const [projectState, setProjectState] = useState<ProjectState | null>(null);
  const [importMessage, setImportMessage] = useState("");
  const [exportOpen, setExportOpen] = useState(false);
  const [projectSetupMode, setProjectSetupMode] = useState<ProjectSetupMode | null>(null);
  const [projectSetupBusy, setProjectSetupBusy] = useState(false);
  const [exportReport, setExportReport] = useState<ExportReport | null>(null);
  const [exportBusy, setExportBusy] = useState(false);
  const [pdfBusy, setPdfBusy] = useState(false);
  const [liveReviewState, setLiveReviewState] = useState<LiveReviewState>({
    status: "starting",
  });
  const [aiContextState, setAiContextState] = useState<AiContextState>({
    status: "no_entity_selected",
    message: "no entity selected",
  });
  const [viewMode, setViewMode] = useState<ViewMode>("diff");
  const [selectedEntityId, setSelectedEntityId] = useState("");
  const [selectedEntityIds, setSelectedEntityIds] = useState<Set<string>>(new Set());
  const [editorMode, setEditorMode] = useState<EditorMode>("select");
  const [cadCommand, setCadCommand] = useState<CadCommand>("select");
  const [commandInput, setCommandInput] = useState("");
  const [snapEnabled, setSnapEnabled] = useState(true);
  const [orthoEnabled, setOrthoEnabled] = useState(false);
  const [draftPoints, setDraftPoints] = useState<Array<[number, number]>>([]);
  const [pendingOperation, setPendingOperation] = useState<EditOperation | null>(null);
  const [draftingOptions, setDraftingOptions] = useState(initialDraftingOptions);
  const [editPreview, setEditPreview] = useState<DrawingEditPreview | null>(null);
  const [dimensionResolutions, setDimensionResolutions] = useState<Record<string, "delete" | "detach">>({});
  const [blockEditing, setBlockEditing] = useState<{ block: string; name: string; revision: string; drawingRevision: string; entities: EditorEntity[]; svg: string } | null>(null);
  const [blockEntityIndex, setBlockEntityIndex] = useState(0);
  const [blockEditMessage, setBlockEditMessage] = useState("");
  const [blockDeletion, setBlockDeletion] = useState<{ id: string; dimensions: string[]; resolutions: Record<string, "delete" | "detach"> } | null>(null);
  const [blockThumbnail, setBlockThumbnail] = useState("");
  const previewSequence = useRef(0);
  const pendingRevision = useRef<string | null>(null);
  const lastSuccessfulCommand = useRef<EditorMode | null>(null);
  const editorModeRef = useRef(editorMode);
  editorModeRef.current = editorMode;
  const draftActivityRef = useRef({ points: draftPoints, operation: pendingOperation });
  draftActivityRef.current = { points: draftPoints, operation: pendingOperation };
  const currentEditorRevisionRef = useRef(artifacts?.editor.revision);
  currentEditorRevisionRef.current = artifacts?.editor.revision;
  const insertionRevision = useRef<string | null>(null);
  const asyncDraftSequence = useRef(0);
  const blockLoadSequence = useRef(0);
  const [draftParameterError, setDraftParameterError] = useState(false);
  const endpointDrag = useRef<{ entityId: string; vertexIndex: number; origin: [number, number]; to: [number, number] } | null>(null);
  const [historyState, setHistoryState] = useState<DrawingHistoryState | null>(null);
  const [historyMessage, setHistoryMessage] = useState("");
  const [isHistoryBusy, setIsHistoryBusy] = useState(false);
  const [snapCandidate, setSnapCandidate] = useState<SnapCandidate | null>(null);
  const [editMessage, setEditMessage] = useState("");
  const [isEditSaving, setIsEditSaving] = useState(false);
  const [baseViewBox, setBaseViewBox] = useState<ViewBox | null>(null);
  const [currentViewBox, setCurrentViewBox] = useState<ViewBox | null>(null);
  const [previousViewBox, setPreviousViewBox] = useState<ViewBox | null>(null);
  const [isPanning, setIsPanning] = useState(false);
  const [isZoomAreaActive, setIsZoomAreaActive] = useState(false);
  const [selectionRect, setSelectionRect] = useState<SelectionRect | null>(null);
  const drawingStageRef = useRef<HTMLDivElement>(null);
  const svgSurfaceRef = useRef<HTMLDivElement>(null);
  const currentViewBoxRef = useRef<ViewBox | null>(null);
  const lastCommentAnchorRef = useRef<{ x: number; y: number } | null>(null);
  const projectStateRef = useRef<ProjectState | null>(null);
  const currentDrawingRef = useRef<string | null>(null);
  const snapSequenceRef = useRef(0);
  const historySequenceRef = useRef(0);
  const snapTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const watchActiveRef = useRef(false);
  const watchErrorRef = useRef<string | null>(null);
  const viewContextRef = useRef<string | null>(null);
  const watchListenerReadyRef = useRef<Promise<() => void> | null>(null);
  const dragInteraction = useRef<DragInteraction | null>(null);
  const suppressClickUntil = useRef(0);
  const wheelHistoryStart = useRef<ViewBox | null>(null);
  const wheelHistoryTimer = useRef<number | null>(null);
  const aiContextWriteQueue = useMemo(() => new LatestAiContextWriteQueue(), []);
  const previousLayerVisibilityRef = useRef<LayerWorkspaceState | null>(null);
  const layerRulesQueue = useMemo(() => new LatestLayerRulesQueue(), []);
  const drawingLoadGuard = useMemo(() => new DrawingLoadGuard(), []);
  const projectOpenGuard = useMemo(() => new ProjectOpenGuard(), []);
  const lastProjectPersistence = useMemo(() => new LatestProjectPersistenceQueue(), []);
  const pdfExportGuard = useMemo(() => new PdfExportGuard(), []);
  const reviewQueue = useMemo(
    () =>
      new LatestReviewQueue((projectPath) =>
        loadReviewSnapshotFromDesktop(
          projectPath,
          sanitizeSvg,
          undefined,
          currentDrawingRef.current ?? undefined,
        ),
      ),
    [],
  );
  const isDesktop = isTauriRuntime();

  useEffect(() => {
    pdfExportGuard.invalidate();
  }, [
    projectState?.project_path,
    artifacts?.currentDrawing,
    artifacts?.editor.revision,
    artifacts?.commentsRevision,
    artifacts?.layers.revision,
    pdfExportGuard,
  ]);

  useEffect(() => {
    const sequence = ++historySequenceRef.current;
    if (!isDesktop || projectState === null || artifacts === null) {
      setHistoryState(null);
      setHistoryMessage("");
      return;
    }
    setHistoryState(null);
    setHistoryMessage("");
    void listDrawingHistory(
      projectState.project_path,
      artifacts.currentDrawing,
    )
      .then((state) => {
        if (sequence === historySequenceRef.current) {
          setHistoryState(state);
        }
      })
      .catch((error: unknown) => {
        if (sequence === historySequenceRef.current) {
          setHistoryState(null);
          setHistoryMessage(formatError(error, "failed to load edit history"));
        }
      });
  }, [
    isDesktop,
    projectState?.project_path,
    artifacts?.currentDrawing,
    artifacts?.editor.revision,
    artifacts?.commentsRevision,
    artifacts?.layers.revision,
  ]);

  useEffect(() => {
    setSelectedEntityIds((current) => {
      if (selectedEntityId === "") return new Set();
      return current.has(selectedEntityId) ? current : new Set([selectedEntityId]);
    });
  }, [selectedEntityId]);

  useEffect(() => {
    if (selectedEntityId !== "" && !selectedEntityIds.has(selectedEntityId)) {
      setSelectedEntityId(selectedEntityIds.values().next().value ?? "");
    }
  }, [selectedEntityId, selectedEntityIds]);

  useEffect(() => {
    if (!isDesktop) {
      return;
    }
    let disposed = false;
    const subscription = subscribeProjectWatch((event) => handleProjectWatchEvent(event));
    watchListenerReadyRef.current = subscription;
    subscription.catch((error: unknown) => {
      if (!disposed) {
        const message = formatError(error, "failed to listen for project changes");
        watchActiveRef.current = false;
        watchErrorRef.current = message;
        setLiveReviewState({
          status: "error",
          message,
        });
      }
    });
    return () => {
      disposed = true;
      watchActiveRef.current = false;
      reviewQueue.reset();
      watchListenerReadyRef.current = null;
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
      void stopProjectWatch().catch(() => undefined);
    };
  }, [isDesktop, reviewQueue]);

  useEffect(() => {
    if (isDesktop) {
      const startupSequence = projectOpenGuard.begin();
      loadLastDesktopProject()
        .then((projectPath) => {
          if (!projectOpenGuard.accepts(startupSequence)) return;
          if (projectPath === null) {
            setLoadState("idle");
            return;
          }
          return openDesktopProject(projectPath, true, startupSequence);
        })
        .catch((error: unknown) => {
          if (!projectOpenGuard.accepts(startupSequence)) return;
          setErrorMessage(formatError(error, "unknown desktop load error"));
          setLoadState("error");
        });
      return;
    }
    loadWebArtifacts();
  }, [isDesktop]);

  const sourceSvg = viewMode === "sheet" ? artifacts?.sheetSvg : artifacts?.diffSvg;
  const activeSvg = viewMode === "sheet" && editPreview?.svg ? editPreview.svg : sourceSvg;
  const activeBaseViewBox = useMemo(() => parseSvgViewBox(sourceSvg), [sourceSvg]);
  const viewContext = `${projectState?.project_path ?? (isDesktop ? "desktop" : "web")}:${artifacts?.currentDrawing ?? "drawing"}:${viewMode}`;
  const zoomScale = useMemo(() => {
    if (baseViewBox === null || currentViewBox === null) {
      return 1;
    }
    return viewBoxZoomScale(baseViewBox, currentViewBox);
  }, [baseViewBox, currentViewBox]);
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
  const selectedEditorEntity = useMemo(
    () => artifacts?.editor.entities.find((entity) => entity.id === selectedEntityId) ?? null,
    [artifacts, selectedEntityId],
  );

  useEffect(() => {
    const preserveView = shouldPreserveView(viewContextRef.current, viewContext);
    ++asyncDraftSequence.current;
    viewContextRef.current = viewContext;
    setBaseViewBox(activeBaseViewBox);
    if (!preserveView || currentViewBoxRef.current === null) {
      setCurrentViewBox(activeBaseViewBox);
      currentViewBoxRef.current = activeBaseViewBox;
      setPreviousViewBox(null);
      setIsPanning(false);
      setIsZoomAreaActive(false);
      setSelectionRect(null);
      dragInteraction.current = null;
      clearPendingWheelHistory();
    }
  }, [activeBaseViewBox, viewContext]);

  useEffect(() => {
    function cancelActiveOperation(event: KeyboardEvent) {
      const inputFocused = event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLSelectElement;
      if (inputFocused && event.key !== "Escape") {
        if (event.key === "Enter" && event.target instanceof Element && event.target.closest(".drafting-panel") !== null && !(event.target instanceof HTMLTextAreaElement)) {
          event.preventDefault(); applyDraftingParameters();
        }
        return;
      }
      if (event.key === "F3") {
        event.preventDefault();
        setSnapEnabled((enabled) => !enabled);
        return;
      }
      if (event.key === "F8") {
        event.preventDefault();
        setOrthoEnabled((enabled) => !enabled);
        return;
      }
      const command = commandForKeyboardEvent(event);
      if (command !== null) {
        event.preventDefault();
        setCadCommand(command);
        if (command === "delete") {
          void deleteSelectedEntities();
          return;
        }
        if (command === "undo" || command === "redo") {
          void runHistoryAction(command === "undo");
          return;
        }
        if (command === "edit_block") {
          editSelectedBlock();
          return;
        }
        if (repeatableCommand(command)) {
          selectEditorMode(command as EditorMode);
          return;
        }
        if (command === "layout" || command === "print_preview") {
          setEditorMode(command);
          setViewMode("sheet");
          setEditMessage(command === "layout" ? "Active layout is shown in the paper frame" : "Print preview uses the active layout");
          return;
        }
      }
      if (event.key === "Escape") {
        ++asyncDraftSequence.current;
        insertionRevision.current = null;
        setBlockEditing(null);
        event.preventDefault();
        setIsZoomAreaActive(false);
        setSelectionRect(null);
        dragInteraction.current = null;
        setEditorMode("select");
        setCadCommand("select");
        setDraftPoints([]);
        setPendingOperation(null);
        setSnapCandidate(null);
        setEditPreview(null);
        endpointDrag.current = null;
      }
      if (event.key === "Backspace" && draftPoints.length > 0) {
        event.preventDefault();
        setDraftPoints(points => points.slice(0, -1));
        setPendingOperation(null);
        return;
      }
      if ((event.key === "Enter" || event.code === "Space") && editorMode === "select" && lastSuccessfulCommand.current !== null) {
        event.preventDefault();
        selectEditorMode(lastSuccessfulCommand.current);
        return;
      }
      if (event.key === "Enter" && pendingOperation !== null) {
        event.preventDefault();
        applyDraftingParameters();
      }
      if (event.key === "Enter" && editorMode === "polyline") {
        event.preventDefault();
        completePolyline();
      }
      if (event.key === "Enter" && pendingOperation === null && editorMode !== "polyline" && editorMode !== "select") {
        event.preventDefault();
        applyDraftingParameters();
      }
    }
    window.addEventListener("keydown", cancelActiveOperation);
    return () => window.removeEventListener("keydown", cancelActiveOperation);
  }, [editorMode, draftPoints, pendingOperation, artifacts, selectedEntityId, selectedEntityIds, projectState, historyState, draftingOptions, draftParameterError]);

  useEffect(() => {
    const sequence = ++previewSequence.current;
    setEditPreview(null);
    setDimensionResolutions({});
    if (pendingOperation === null || projectState === null || artifacts === null || !isDesktop) {
      pendingRevision.current = null;
      return;
    }
    const expectedRevision = expectedRevisionForOperation(pendingOperation, artifacts.editor.revision, artifacts.blocks, artifacts.layouts);
    if (expectedRevision === null) return;
    if (pendingRevision.current !== null && pendingRevision.current !== expectedRevision) {
      setPendingOperation(null);
      setEditMessage("Source changed during preview. Restart the operation on the refreshed drawing.");
      return;
    }
    pendingRevision.current = expectedRevision;
    const timer = window.setTimeout(() => void previewDrawingEdit(projectState.project_path, { drawing: artifacts.currentDrawing, expected_revision: expectedRevision, operation: pendingOperation }).then(preview => {
      if (sequence !== previewSequence.current) return;
      setEditPreview({ ...preview, svg: sanitizeSvg(preview.svg) });
      setEditMessage([...preview.warnings, "Preview ready; Enter or Apply to save"].join(" · "));
    }).catch(error => {
      if (sequence === previewSequence.current) setEditMessage(formatError(error, "preview failed"));
    }), 60);
    return () => { window.clearTimeout(timer); ++previewSequence.current; };
  }, [pendingOperation, artifacts?.editor.revision, projectState?.project_path]);

  useEffect(() => () => clearPendingWheelHistory(), []);

  useEffect(() => {
    const svg = currentSvgElement();
    if (
      svg === null ||
      baseViewBox === null ||
      currentViewBox === null ||
      activeBaseViewBox === null ||
      !sameViewBox(baseViewBox, activeBaseViewBox)
    ) {
      return;
    }
    svg.setAttribute("viewBox", formatViewBox(currentViewBox));
  }, [activeBaseViewBox, baseViewBox, currentViewBox]);

  useEffect(() => {
    document
      .querySelectorAll(".drawing-stage [data-entity-id]")
      .forEach((element) => element.classList.remove("is-selected"));
    if (selectedEntityId === "") {
      return;
    }
    selectedEntityIds.forEach((entityId) => {
      document
        .querySelectorAll(`.drawing-stage [data-entity-id="${entityId}"]`)
        .forEach((element) => element.classList.add("is-selected"));
    });
  }, [activeSvg, selectedEntityIds]);

  useEffect(() => {
    lastCommentAnchorRef.current = null;
  }, [artifacts?.currentDrawing, selectedEntityId]);

  useEffect(() => {
    if (artifacts !== null) {
      applyLayerWorkspaceToSvg(artifacts.layers);
      document.querySelectorAll<SVGElement>(".drawing-stage [data-screen-stroke-width]").forEach(element => {
        const width = Number(element.getAttribute("data-screen-stroke-width"));
        if (Number.isFinite(width) && width > 0) element.style.setProperty("--screen-stroke-width", `${width}px`);
      });
    }
  }, [activeSvg, artifacts?.layers]);

  useEffect(() => {
    if (!isDesktop || projectState === null || loadState !== "ready") {
      return;
    }
    if (selectedEntityId === "") {
      return aiContextWriteQueue.enqueue(
        {
          projectPath: projectState.project_path,
          viewMode,
          selectedEntityId: "",
        },
        setAiContextState,
        (error: unknown) => {
          setAiContextState({
            status: "error",
            message: formatError(error, "failed to clear AI context"),
          });
        },
      );
    }
    return aiContextWriteQueue.enqueue(
      {
        projectPath: projectState.project_path,
        viewMode,
        selectedEntityId,
      },
      setAiContextState,
      (error: unknown) => {
        setAiContextState({
          status: "error",
          message: formatError(error, "failed to write AI context"),
        });
      },
    );
  }, [
    aiContextWriteQueue,
    artifacts,
    isDesktop,
    loadState,
    projectState,
    selectedEntityId,
    viewMode,
  ]);

  function replaceCurrentViewBox(nextViewBox: ViewBox) {
    currentViewBoxRef.current = nextViewBox;
    setCurrentViewBox(nextViewBox);
  }

  function clearPendingWheelHistory() {
    if (wheelHistoryTimer.current !== null) {
      window.clearTimeout(wheelHistoryTimer.current);
      wheelHistoryTimer.current = null;
    }
    wheelHistoryStart.current = null;
  }

  function commitPendingWheelHistory() {
    if (wheelHistoryTimer.current !== null) {
      window.clearTimeout(wheelHistoryTimer.current);
      wheelHistoryTimer.current = null;
    }
    const startViewBox = wheelHistoryStart.current;
    const current = currentViewBoxRef.current;
    wheelHistoryStart.current = null;
    const previous = committedWheelPrevious(startViewBox, current);
    if (previous !== null) {
      setPreviousViewBox(previous);
    }
  }

  /** Discrete navigation replaces the one-step history with the current view. */
  function applyViewBox(nextViewBox: ViewBox) {
    const current = currentViewBoxRef.current;
    if (current === null || sameViewBox(current, nextViewBox)) {
      return;
    }
    clearPendingWheelHistory();
    setPreviousViewBox(current);
    replaceCurrentViewBox(nextViewBox);
  }

  function resetView() {
    if (baseViewBox !== null) {
      applyViewBox(baseViewBox);
    }
  }

  function zoomBy(factor: number) {
    const current = currentViewBoxRef.current;
    if (baseViewBox === null || current === null) {
      return;
    }
    const currentScale = viewBoxZoomScale(baseViewBox, current);
    const zoom = zoomViewBoxAtPoint({
      baseViewBox,
      currentViewBox: current,
      targetScale: currentScale * factor,
      focusPoint: viewBoxCenter(current),
      minScale: MIN_ZOOM_SCALE,
      maxScale: MAX_ZOOM_SCALE,
    });
    if (zoom !== null) {
      applyViewBox(zoom.viewBox);
    }
  }

  function zoomToContent() {
    const fit = contentFitViewBox();
    if (fit !== null) {
      applyViewBox(fit);
    }
  }

  function contentFitViewBox(): ViewBox | null {
    const svg = currentSvgElement();
    if (svg === null) {
      return null;
    }
    const content = bboxFromElements(
      Array.from(svg.querySelectorAll("[data-entity-id][data-bbox]")).filter(
        (element) => element.getAttribute("data-layer-visible") !== "false",
      ),
    );
    return content === null ? null : fittedCadBBoxViewBox(content);
  }

  function focusSelectedEntity(entityId: string) {
    const svg = currentSvgElement();
    if (svg === null) {
      return;
    }
    const matchedElements: Element[] = [];
    svg.querySelectorAll("[data-entity-id][data-bbox]").forEach((element) => {
      if (element.getAttribute("data-entity-id") === entityId) {
        if (element.getAttribute("data-layer-visible") !== "false") {
          matchedElements.push(element);
        }
      }
    });
    const content = bboxFromElements(matchedElements);
    if (content === null) {
      return;
    }
    const fit = fittedCadBBoxViewBox(content);
    if (fit !== null) {
      applyViewBox(fit);
    }
  }

  function currentSvgElement(): SVGSVGElement | null {
    const surface = svgSurfaceRef.current;
    if (surface === null) {
      return null;
    }
    return surface.querySelector("svg");
  }

  function fittedCadBBoxViewBox(content: BBox): ViewBox | null {
    const stage = drawingStageRef.current;
    if (stage === null || baseViewBox === null) {
      return null;
    }
    const stageRect = stage.getBoundingClientRect();
    const fit = computeFitViewBox({
      stage: { width: stageRect.width, height: stageRect.height },
      baseViewBox,
      content,
      maxScale: MAX_ZOOM_SCALE,
    });
    return fit?.viewBox ?? null;
  }

  function clientPointToSvg(point: Point): Point | null {
    const svg = currentSvgElement();
    const screenMatrix = svg?.getScreenCTM();
    if (screenMatrix === null || screenMatrix === undefined) {
      return null;
    }
    try {
      const transformed = new DOMPoint(point.x, point.y).matrixTransform(screenMatrix.inverse());
      if (!Number.isFinite(transformed.x) || !Number.isFinite(transformed.y)) {
        return null;
      }
      return { x: transformed.x, y: transformed.y };
    } catch {
      return null;
    }
  }

  function applyZoomArea(startClient: Point, endClient: Point): boolean {
    const stage = drawingStageRef.current;
    if (
      stage === null ||
      baseViewBox === null ||
      Math.abs(endClient.x - startClient.x) < GESTURE_THRESHOLD_PX ||
      Math.abs(endClient.y - startClient.y) < GESTURE_THRESHOLD_PX
    ) {
      return false;
    }
    const startSvg = clientPointToSvg(startClient);
    const endSvg = clientPointToSvg(endClient);
    if (startSvg === null || endSvg === null) {
      return false;
    }
    const stageRect = stage.getBoundingClientRect();
    const fit = computeFitSvgViewBox({
      stage: { width: stageRect.width, height: stageRect.height },
      baseViewBox,
      content: pointsToBBox(startSvg, endSvg),
      paddingPx: 0,
      maxScale: MAX_ZOOM_SCALE,
    });
    if (fit === null) {
      return false;
    }
    applyViewBox(fit.viewBox);
    return true;
  }

  function goToPreviousView() {
    const swapped = swapPreviousView(currentViewBoxRef.current, previousViewBox);
    if (swapped === null) {
      return;
    }
    clearPendingWheelHistory();
    setPreviousViewBox(swapped.previous);
    replaceCurrentViewBox(swapped.current);
  }

  function handleWheel(event: WheelEvent) {
    event.preventDefault();
    if (dragInteraction.current !== null) {
      return;
    }
    const stage = drawingStageRef.current;
    const current = currentViewBoxRef.current;
    if (stage === null || baseViewBox === null || current === null) {
      return;
    }
    const stageRect = stage.getBoundingClientRect();
    const factor = wheelDeltaToScaleFactor(event.deltaY, event.deltaMode, stageRect.height);
    const focusPoint = clientPointToSvg({ x: event.clientX, y: event.clientY });
    if (factor === null || focusPoint === null) {
      return;
    }
    const currentScale = viewBoxZoomScale(baseViewBox, current);
    const zoom = zoomViewBoxAtPoint({
      baseViewBox,
      currentViewBox: current,
      targetScale: currentScale * factor,
      focusPoint,
      minScale: MIN_ZOOM_SCALE,
      maxScale: MAX_ZOOM_SCALE,
    });
    if (zoom === null || sameViewBox(current, zoom.viewBox)) {
      return;
    }

    // A stream of trackpad or wheel events is one reversible navigation step.
    wheelHistoryStart.current = beginWheelHistory(wheelHistoryStart.current, current);
    if (wheelHistoryTimer.current !== null) {
      window.clearTimeout(wheelHistoryTimer.current);
    }
    replaceCurrentViewBox(zoom.viewBox);
    wheelHistoryTimer.current = window.setTimeout(
      commitPendingWheelHistory,
      WHEEL_HISTORY_DELAY_MS,
    );
  }

  function handlePointerDown(event: PointerEvent) {
    const anchorPoint = clientPointToSvg({ x: event.clientX, y: event.clientY });
    if (anchorPoint !== null && viewMode === "sheet") {
      lastCommentAnchorRef.current = { x: anchorPoint.x, y: -anchorPoint.y };
    }
    if ((editorMode === "move" || editorMode === "copy") && viewMode === "sheet") {
      const targetId = closestEntityId(event.target);
      const point = clientPointToSvg({ x: event.clientX, y: event.clientY });
      if (targetId === selectedEntityId && point !== null && drawingStageRef.current !== null) {
        const cadPoint: [number, number] = [point.x, -point.y];
        dragInteraction.current = {
          pointerId: event.pointerId,
          mode: "edit-move",
          startClient: { x: event.clientX, y: event.clientY },
          endClient: { x: event.clientX, y: event.clientY },
          startViewBox: currentViewBoxRef.current ?? { minX: 0, minY: 0, width: 1, height: 1 },
          moved: false,
        };
        setDraftPoints([cadPoint, cadPoint]);
        drawingStageRef.current.setPointerCapture(event.pointerId);
      }
      return;
    }
    if (editorMode !== "select" && viewMode === "sheet") {
      return;
    }
    const current = currentViewBoxRef.current;
    const stage = drawingStageRef.current;
    if (current === null || stage === null || (event.button !== 0 && event.button !== 2)) {
      return;
    }
    const point = { x: event.clientX, y: event.clientY };
    const mode: DragMode =
      event.buttons === 3
        ? "jw-gesture"
        : editorMode === "select" && event.shiftKey && event.button === 0
          ? "selection-area"
        : isZoomAreaActive && event.button === 0
          ? "zoom-area"
          : event.button === 0
            ? "pan"
            : "right-wait";
    dragInteraction.current = {
      pointerId: event.pointerId,
      mode,
      startClient: point,
      endClient: point,
      startViewBox: current,
      moved: false,
    };
    if (mode === "zoom-area" || mode === "jw-gesture") {
      stage.setPointerCapture(event.pointerId);
    }
  }

  function beginEndpoint(event: PointerEvent, entity: EditorEntity, vertexIndex: number, origin: [number, number]) {
    if (event.button !== 0 || currentViewBoxRef.current === null || drawingStageRef.current === null) return;
    event.preventDefault(); event.stopPropagation();
    endpointDrag.current = { entityId: entity.id, vertexIndex, origin, to: origin };
    setEditorMode("endpoint"); setCadCommand("endpoint");
    setDraftPoints([origin]); setPendingOperation(null);
    dragInteraction.current = { pointerId: event.pointerId, mode: "edit-endpoint", startClient: { x: event.clientX, y: event.clientY }, endClient: { x: event.clientX, y: event.clientY }, startViewBox: currentViewBoxRef.current, moved: false };
    drawingStageRef.current.setPointerCapture(event.pointerId);
  }

  /** Mouse down observes the second physical button, which pointerdown omits. */
  function handleChordMouseDown(event: MouseEvent) {
    if (event.buttons !== 3) {
      return;
    }
    const current = currentViewBoxRef.current;
    if (current === null) {
      return;
    }
    event.preventDefault();
    const point = { x: event.clientX, y: event.clientY };
    dragInteraction.current = {
      pointerId: dragInteraction.current?.pointerId ?? 1,
      mode: "jw-gesture",
      startClient: point,
      endClient: point,
      startViewBox: current,
      moved: false,
    };
    const stage = drawingStageRef.current;
    if (stage !== null && !stage.hasPointerCapture(dragInteraction.current.pointerId)) {
      stage.setPointerCapture(dragInteraction.current.pointerId);
    }
    setIsPanning(false);
    setSelectionRect(null);
  }

  function selectEntitiesInDrag(startClient: Point, endClient: Point) {
    const start = clientPointToSvg(startClient);
    const end = clientPointToSvg(endClient);
    const svg = currentSvgElement();
    if (start === null || end === null || svg === null) return;
    const area = {
      minX: Math.min(start.x, end.x),
      minY: Math.min(-start.y, -end.y),
      maxX: Math.max(start.x, end.x),
      maxY: Math.max(-start.y, -end.y),
    };
    const crossing = endClient.x < startClient.x;
    const ids = new Set<string>();
    svg.querySelectorAll("[data-entity-id][data-bbox]").forEach((element) => {
      if (element.closest('[data-layer-visible="false"]') !== null) return;
      for (let ancestor: Element | null = element; ancestor !== null; ancestor = ancestor.parentElement) {
        const style = getComputedStyle(ancestor);
        if (style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") return;
        if (ancestor === svg) break;
      }
      const values = element.getAttribute("data-bbox")?.split(",").map(Number);
      const id = element.getAttribute("data-entity-id");
      if (id === null || values === undefined || values.length !== 4 || values.some((value) => !Number.isFinite(value))) return;
      const [minX, minY, maxX, maxY] = values;
      const matches = crossing
        ? maxX >= area.minX && minX <= area.maxX && maxY >= area.minY && minY <= area.maxY
        : minX >= area.minX && maxX <= area.maxX && minY >= area.minY && maxY <= area.maxY;
      if (matches) ids.add(id);
    });
    setSelectedEntityIds(ids);
    setSelectedEntityId(ids.values().next().value ?? "");
    setEditMessage(`${crossing ? "Crossing" : "Window"} selection: ${ids.size}`);
  }

  function handlePointerMove(event: PointerEvent) {
    if (editorMode !== "select" && viewMode === "sheet") {
      void updateSnapForPointer(event);
    }
    let interaction = dragInteraction.current;
    if (interaction === null) {
      return;
    }
    if (interaction.mode === "edit-endpoint" && endpointDrag.current !== null) {
      const target = endpointDrag.current;
      const raw = clientPointToSvg({ x: event.clientX, y: event.clientY });
      if (raw === null) return;
      const rawPoint: [number, number] = [raw.x, -raw.y];
      const tolerance = (currentViewBoxRef.current?.width ?? 1) / Math.max(drawingStageRef.current?.clientWidth ?? 1, 1) * 10;
      const snapped = snapEnabled && !event.shiftKey && snapCandidate !== null && pointArrayDistance(snapCandidate.point, rawPoint) <= tolerance ? snapCandidate.point : rawPoint;
      target.to = applyOrtho(snapped, target.origin, orthoEnabled);
      interaction.moved = pointDistance(interaction.startClient, { x: event.clientX, y: event.clientY }) >= GESTURE_THRESHOLD_PX;
      setDraftPoints([target.origin, target.to]);
      if (interaction.moved) setPendingOperation({ kind: "endpoint", entity_id: target.entityId, vertex_index: target.vertexIndex, to: target.to });
      return;
    }
    if (event.buttons === 3 && interaction.mode !== "jw-gesture") {
      const point = { x: event.clientX, y: event.clientY };
      interaction = {
        pointerId: interaction.pointerId,
        mode: "jw-gesture",
        startClient: point,
        endClient: point,
        startViewBox: currentViewBoxRef.current ?? interaction.startViewBox,
        moved: false,
      };
      dragInteraction.current = interaction;
      const stage = drawingStageRef.current;
      if (stage !== null && !stage.hasPointerCapture(event.pointerId)) {
        stage.setPointerCapture(event.pointerId);
      }
      setIsPanning(false);
    }
    if (event.pointerId !== interaction.pointerId) {
      return;
    }

    const endClient = { x: event.clientX, y: event.clientY };
    interaction.endClient = endClient;
    if (interaction.mode === "edit-endpoint") {
      suppressCanvasClick();
      setEditMessage("Endpoint preview; Enter saves, Esc cancels");
      return;
    }
    if (interaction.mode === "edit-move") {
      const point = clientPointToSvg(endClient);
      if (point !== null) {
        interaction.moved = pointDistance(interaction.startClient, endClient) >= PAN_DRAG_THRESHOLD_PX;
        setDraftPoints((current) => [current[0] ?? [point.x, -point.y], [point.x, -point.y]]);
      }
      return;
    }
    if (interaction.mode === "pan") {
      const distance = pointDistance(interaction.startClient, endClient);
      if (!interaction.moved && distance < PAN_DRAG_THRESHOLD_PX) {
        return;
      }
      interaction.moved = true;
      const stage = drawingStageRef.current;
      if (stage === null) {
        return;
      }
      if (!stage.hasPointerCapture(event.pointerId)) {
        stage.setPointerCapture(event.pointerId);
      }
      const stageRect = stage.getBoundingClientRect();
      const nextViewBox = panViewBox({
        currentViewBox: interaction.startViewBox,
        viewport: { width: stageRect.width, height: stageRect.height },
        deltaPx: {
          x: endClient.x - interaction.startClient.x,
          y: endClient.y - interaction.startClient.y,
        },
      });
      if (nextViewBox !== null) {
        setIsPanning(true);
        replaceCurrentViewBox(nextViewBox);
      }
      return;
    }

    interaction.moved = pointDistance(interaction.startClient, endClient) >= GESTURE_THRESHOLD_PX;
    if (interaction.mode === "zoom-area" || interaction.mode === "selection-area") {
      if (interaction.mode === "selection-area" && interaction.moved) {
        const stage = drawingStageRef.current;
        if (stage !== null && !stage.hasPointerCapture(event.pointerId)) {
          stage.setPointerCapture(event.pointerId);
        }
      }
      setSelectionRect(interaction.moved
        ? clientSelectionRect(interaction.startClient, endClient)
        : null);
      return;
    }
    const gesture = classifyJwCadGesture(interaction.startClient, endClient);
    setSelectionRect(
      gesture === "zoom-area" ? clientSelectionRect(interaction.startClient, endClient) : null,
    );
  }

  function handlePointerUp(event: PointerEvent) {
    const interaction = dragInteraction.current;
    const stage = drawingStageRef.current;
    if (interaction === null || event.pointerId !== interaction.pointerId) {
      return;
    }
    interaction.endClient = { x: event.clientX, y: event.clientY };
    dragInteraction.current = null;
    setIsPanning(false);
    setSelectionRect(null);
    if (stage?.hasPointerCapture(event.pointerId)) {
      stage.releasePointerCapture(event.pointerId);
    }

    if (interaction.mode === "edit-move") {
      suppressCanvasClick();
      const start = draftPoints[0];
      const rawEnd = clientPointToSvg(interaction.endClient);
      const end = snapCandidate?.point ?? (rawEnd === null ? undefined : [rawEnd.x, -rawEnd.y] as [number, number]);
      if (interaction.moved && start !== undefined && end !== undefined && selectedEntityId !== "") {
        const entityIds = selectedEntityIds.size > 0 ? Array.from(selectedEntityIds) : [selectedEntityId];
        const delta: [number, number] = [end[0] - start[0], end[1] - start[1]];
        void commitDrawingEdit(entityIds.length > 1
          ? { kind: "translate_many", entity_ids: entityIds, delta, duplicate: editorMode === "copy" }
          : { kind: "translate", entity_id: selectedEntityId, delta, duplicate: editorMode === "copy" });
      } else {
        setDraftPoints([]);
      }
      return;
    }
    if (interaction.mode === "selection-area") {
      if (interaction.moved) {
        suppressCanvasClick();
        selectEntitiesInDrag(interaction.startClient, interaction.endClient);
      }
      return;
    }

    if (interaction.mode === "pan") {
      if (interaction.moved) {
        suppressCanvasClick();
        const current = currentViewBoxRef.current;
        if (current !== null && !sameViewBox(interaction.startViewBox, current)) {
          setPreviousViewBox(interaction.startViewBox);
        }
      }
      return;
    }
    if (interaction.mode === "zoom-area") {
      suppressCanvasClick();
      if (applyZoomArea(interaction.startClient, interaction.endClient)) {
        setIsZoomAreaActive(false);
      }
      return;
    }
    if (interaction.mode === "jw-gesture") {
      suppressCanvasClick();
      executeJwCadGesture(interaction);
    }
  }

  function handlePointerCancel(event: PointerEvent) {
    const interaction = dragInteraction.current;
    if (interaction === null || event.pointerId !== interaction.pointerId) {
      return;
    }
    if (interaction.mode === "pan" && interaction.moved) {
      replaceCurrentViewBox(interaction.startViewBox);
    }
    if (interaction.mode === "edit-move") {
      setDraftPoints([]);
      setPendingOperation(null);
      setSnapCandidate(null);
    }
    dragInteraction.current = null;
    setIsPanning(false);
    setSelectionRect(null);
  }

  function executeJwCadGesture(interaction: DragInteraction) {
    const gesture = classifyJwCadGesture(interaction.startClient, interaction.endClient);
    if (gesture === "zoom-area") {
      applyZoomArea(interaction.startClient, interaction.endClient);
      return;
    }
    if (gesture === "zoom-all") {
      zoomToContent();
      return;
    }
    if (gesture === "previous") {
      goToPreviousView();
      return;
    }
    if (gesture === "none") {
      return;
    }
    const focusPoint = clientPointToSvg(interaction.startClient);
    if (focusPoint === null || baseViewBox === null) {
      return;
    }
    if (gesture === "recenter") {
      const centered = centerViewBoxAtPoint(interaction.startViewBox, focusPoint);
      if (centered !== null) {
        applyViewBox(centered);
      }
      return;
    }
    const currentScale = viewBoxZoomScale(baseViewBox, interaction.startViewBox);
    const zoom = zoomViewBoxAtPoint({
      baseViewBox,
      currentViewBox: interaction.startViewBox,
      targetScale: currentScale / 2,
      focusPoint,
      minScale: MIN_ZOOM_SCALE,
      maxScale: MAX_ZOOM_SCALE,
    });
    if (zoom !== null) {
      applyViewBox(zoom.viewBox);
    }
  }

  function clientSelectionRect(startClient: Point, endClient: Point): SelectionRect | null {
    const stage = drawingStageRef.current;
    if (stage === null) {
      return null;
    }
    const stageRect = stage.getBoundingClientRect();
    return {
      x: Math.min(startClient.x, endClient.x) - stageRect.left,
      y: Math.min(startClient.y, endClient.y) - stageRect.top,
      width: Math.abs(endClient.x - startClient.x),
      height: Math.abs(endClient.y - startClient.y),
    };
  }

  function suppressCanvasClick() {
    suppressClickUntil.current = performance.now() + 250;
  }

  function selectEntity(entityId: string, focus: boolean, append = false) {
    if (entityId === "") {
      return;
    }
    setSelectedEntityId(entityId);
    setSelectedEntityIds((current) => {
      if (!append) return new Set([entityId]);
      const next = new Set(current);
      if (next.has(entityId)) {
        next.delete(entityId);
      } else {
        next.add(entityId);
      }
      return next;
    });
    if (focus) {
      focusSelectedEntity(entityId);
    }
  }

  async function createCommentForSelection() {
    if (isEditSaving || isHistoryBusy || !isDesktop || projectState === null || artifacts === null || selectedEntityId === "") {
      return;
    }
    const text = window.prompt("Comment");
    if (text === null || text.trim() === "") {
      return;
    }
    setIsEditSaving(true);
    try {
      const result = await createDesktopComment(projectState.project_path, {
        drawing: artifacts.currentDrawing,
        expected_revision: artifacts.commentsRevision,
        entity_id: selectedEntityId,
        anchor: lastCommentAnchorRef.current ?? entityAnchor(selectedEditorEntity),
        text,
      });
      setArtifacts((current) => current === null ? current : {
        ...current,
        comments: result.comments,
        commentsRevision: result.revision,
      });
      enqueueDesktopReview(projectState.project_path);
    } catch (error: unknown) {
      setEditMessage(formatError(error, "failed to create comment"));
    } finally {
      setIsEditSaving(false);
    }
  }

  async function changeCommentStatus(comment: CommentRecord) {
    if (isEditSaving || isHistoryBusy || !isDesktop || projectState === null || artifacts === null) {
      return;
    }
    setIsEditSaving(true);
    try {
      const result = await updateDesktopCommentStatus(projectState.project_path, {
        drawing: artifacts.currentDrawing,
        expected_revision: artifacts.commentsRevision,
        comment_id: comment.id,
        status: comment.status === "resolved" ? "open" : "resolved",
      });
      setArtifacts((current) => current === null ? current : {
        ...current,
        comments: result.comments,
        commentsRevision: result.revision,
      });
      enqueueDesktopReview(projectState.project_path);
    } catch (error: unknown) {
      setEditMessage(formatError(error, "failed to update comment status"));
    } finally {
      setIsEditSaving(false);
    }
  }

  function loadWebArtifacts() {
    setLoadState("loading");
    loadArtifacts()
      .then((loaded) => {
        currentDrawingRef.current = loaded.currentDrawing;
        setArtifacts(loaded);
        setProjectState(null);
        setLoadState("ready");
      })
      .catch((error: unknown) => {
        setErrorMessage(formatError(error, "unknown load error"));
        setLoadState("error");
      });
  }

  async function loadLastDesktopProject(): Promise<string | null> {
    if (import.meta.env.VITE_CAD_DESKTOP_E2E === "1") {
      return import.meta.env.VITE_CAD_E2E_PROJECT_PATH ?? null;
    }
    return invoke<string | null>("load_last_project");
  }

  async function openDesktopProject(
    projectPath: string,
    clearSelection = true,
    requestedSequence?: number,
  ) {
    const openSequence = requestedSequence ?? projectOpenGuard.begin();
    setLoadState("loading");
    setLiveReviewState({ status: "starting" });
    watchActiveRef.current = false;
    watchErrorRef.current = null;
    reviewQueue.reset();
    layerRulesQueue.reset();
    previousLayerVisibilityRef.current = null;
    projectStateRef.current = null;
    currentDrawingRef.current = null;
    drawingLoadGuard.reset(null);
    if (clearSelection) {
      setSelectedEntityId("");
      setSelectedEntityIds(new Set());
    }
    setEditorMode("select");
    setCadCommand("select");
    setDraftPoints([]);
    setPendingOperation(null);
    setHistoryState(null);
    setHistoryMessage("");
    const state = await invoke<ProjectState>("open_project", { projectPath });
    if (!projectOpenGuard.accepts(openSequence)) return;
    await loadDesktopProjectState(state, openSequence);
  }

  async function loadDesktopProjectState(state: ProjectState, openSequence: number) {
    await lastProjectPersistence.enqueue(
      openSequence,
      (sequence) => projectOpenGuard.accepts(sequence),
      () => invoke("save_last_project", { projectPath: state.project_path }),
    );
    if (!projectOpenGuard.accepts(openSequence)) return;
    const loaded = await loadReviewSnapshotFromDesktop(state.project_path, sanitizeSvg);
    if (!projectOpenGuard.accepts(openSequence)) return;
    currentDrawingRef.current = loaded.artifacts.currentDrawing;
    drawingLoadGuard.reset(loaded.artifacts.currentDrawing);
    const reviewedState = mergeProjectStateFromReview(state, loaded);
    projectStateRef.current = reviewedState;
    setProjectState(reviewedState);
    setImportMessage(importWarningMessage(reviewedState));
    setArtifacts(loaded.artifacts);
    setLoadState("ready");
    try {
      const listenerReady = watchListenerReadyRef.current;
      if (listenerReady === null) {
        throw new Error("project watch listener is not ready");
      }
      await listenerReady;
      if (!projectOpenGuard.accepts(openSequence)) return;
      await startProjectWatchWithCatchUp(state.project_path, (projectPath) => {
        watchActiveRef.current = true;
        watchErrorRef.current = null;
        enqueueDesktopReview(projectPath);
      });
    } catch (error: unknown) {
      if (!projectOpenGuard.accepts(openSequence)) return;
      const message = formatError(error, "failed to watch project");
      watchActiveRef.current = false;
      watchErrorRef.current = message;
      setLiveReviewState({
        status: "error",
        message,
      });
    }
  }

  async function chooseProject() {
    if (!isDesktop) {
      return;
    }
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Open CAD project",
    });
    if (typeof selected === "string") {
      try {
        await openDesktopProject(selected);
      } catch (error: unknown) {
        setSelectedEntityId("");
        setErrorMessage(formatError(error, "unknown project open error"));
        setLoadState("error");
      }
    }
  }

  async function submitProjectSetup(values: {
    parentDir: string;
    folderName: string;
    projectName: string;
    drawing: string;
    paper: string;
    orientation: "landscape" | "portrait";
    scaleDenominator: number;
    sourceDrawing: string;
  }) {
    if (!isDesktop || projectSetupMode === null) return;
    setProjectSetupBusy(true);
    try {
      if (projectSetupMode === "new") {
        const state = await invoke<ProjectState>("create_project", {
          request: {
            parent_dir: values.parentDir,
            folder_name: values.folderName,
            project_name: values.projectName,
            drawing: values.drawing,
            paper: values.paper,
            orientation: values.orientation,
            scale_denominator: values.scaleDenominator,
          },
        });
        setProjectSetupMode(null);
        await openDesktopProject(state.project_path);
      } else if (projectState !== null) {
        if (projectSetupMode === "add") {
          await invoke("add_drawing", {
            request: {
              project_path: projectState.project_path,
              drawing: values.drawing,
              paper: values.paper,
              orientation: values.orientation,
              scale_denominator: values.scaleDenominator,
            },
          });
        } else {
          await invoke("duplicate_drawing", {
            request: {
              project_path: projectState.project_path,
              source_drawing: values.sourceDrawing,
              drawing: values.drawing,
            },
          });
        }
        setProjectSetupMode(null);
        await openDesktopProject(projectState.project_path, false);
      }
    } catch (error: unknown) {
      setErrorMessage(formatError(error, "project operation failed"));
    } finally {
      setProjectSetupBusy(false);
    }
  }

  async function chooseJwwImport() {
    if (!isDesktop) {
      return;
    }
    const selectedFile = await open({
      multiple: false,
      title: "Import JWW",
      filters: [{ name: "JWW", extensions: ["jww"] }],
    });
    if (typeof selectedFile !== "string") {
      return;
    }
    const selectedParent = await open({
      directory: true,
      multiple: false,
      title: "Choose import destination folder",
    });
    if (typeof selectedParent !== "string") {
      return;
    }
    setLoadState("loading");
    setLiveReviewState({ status: "starting" });
    watchActiveRef.current = false;
    watchErrorRef.current = null;
    reviewQueue.reset();
    projectStateRef.current = null;
    layerRulesQueue.reset();
    previousLayerVisibilityRef.current = null;
    setSelectedEntityId("");
    const openSequence = projectOpenGuard.begin();
    try {
      const state = await importJwwFromDesktop(selectedFile, selectedParent);
      if (!projectOpenGuard.accepts(openSequence)) return;
      await loadDesktopProjectState(state, openSequence);
    } catch (error: unknown) {
      setErrorMessage(formatError(error, "unknown JWW import error"));
      setLoadState("error");
    }
  }

  function applyLayerWorkspaceToSvg(state: LayerWorkspaceState) {
    const groups = new Map(state.groups.map((group) => [group.id, group]));
    const visibility = new Map(
      state.layers.map((layer) => [
        layer.id,
        layer.visible && (groups.get(layer.group ?? "default")?.visible ?? true),
      ]),
    );
    const locked = new Map(
      state.layers.map((layer) => [
        layer.id,
        layer.locked || (groups.get(layer.group ?? "default")?.locked ?? false),
      ]),
    );
    document.querySelectorAll(".drawing-stage [data-layer]").forEach((element) => {
      const layerId = element.getAttribute("data-layer") ?? "";
      const visible = visibility.get(layerId) ?? true;
      if (element instanceof SVGElement) {
        element.style.display = visible ? "" : "none";
      }
      element.setAttribute("data-layer-visible", String(visible));
      element.setAttribute("data-layer-locked", String(locked.get(layerId) ?? false));
    });
  }

  async function updateLayerWorkspace(
    next: LayerWorkspaceState,
    patch: LayerRulesPatch,
    rememberVisibility = false,
  ) {
    if (artifacts === null || isEditSaving || isHistoryBusy) {
      return;
    }
    if (rememberVisibility) {
      previousLayerVisibilityRef.current = artifacts.layers;
    }
    setArtifacts((current) => (current === null ? current : { ...current, layers: next }));
    applyLayerWorkspaceToSvg(next);
    if (selectedEntityId !== "") {
      const selected = currentSvgElement()?.querySelector(
        `[data-entity-id="${selectedEntityId}"]`,
      );
      const selectedLayer = selected?.getAttribute("data-layer");
      if (selectedLayer !== null && selectedLayer !== undefined) {
        const layer = next.layers.find((candidate) => candidate.id === selectedLayer);
        const group = next.groups.find(
          (candidate) => candidate.id === (layer?.group ?? "default"),
        );
        if (layer?.visible === false || group?.visible === false) {
          setSelectedEntityId("");
        }
      }
    }
    if (!isDesktop || projectState === null) {
      return;
    }
    setIsEditSaving(true);
    try {
      const result = await layerRulesQueue.enqueue(projectState.project_path, {
        ...patch,
        drawing: artifacts.currentDrawing,
        expectedRevision: artifacts.layers.revision,
      });
      if (!result.isLatest) {
        return;
      }
      if ("error" in result) {
        setImportMessage(formatError(result.error, "failed to update layer rules"));
        enqueueDesktopReview(projectState.project_path);
        return;
      }
      setArtifacts((current) =>
        current === null ? current : { ...current, layers: result.state },
      );
      applyLayerWorkspaceToSvg(result.state);
    } catch (error: unknown) {
      setImportMessage(formatError(error, "failed to update layer rules"));
      enqueueDesktopReview(projectState.project_path);
    } finally {
      setIsEditSaving(false);
    }
  }

  async function restoreLayerVisibility() {
    if (artifacts === null || previousLayerVisibilityRef.current === null) {
      return;
    }
    const previous = previousLayerVisibilityRef.current;
    previousLayerVisibilityRef.current = artifacts.layers;
    await updateLayerWorkspace(
      previous,
      {
        expectedRevision: artifacts.layers.revision,
        layers: previous.layers.map((layer) => ({ id: layer.id, visible: layer.visible })),
      },
    );
  }

  async function performJwwExport(drawing: string, allowLossy: boolean) {
    if (!isDesktop || projectState === null) {
      return;
    }
    const outputPath = await save({
      title: "Export JWW (Experimental)",
      defaultPath: `${projectState.project_name}.jww`,
      filters: [{ name: "JWW", extensions: ["jww"] }],
    });
    if (typeof outputPath !== "string") {
      return;
    }
    setExportBusy(true);
    try {
      let report: ExportReport;
      try {
        report = await exportJwwFromDesktop(
          projectState.project_path,
          drawing,
          outputPath,
          allowLossy,
          false,
        );
      } catch (error: unknown) {
        const message = formatError(error, "JWW export failed");
        if (!message.includes("output already exists")) {
          throw error;
        }
        const overwrite = await confirm("Replace the existing JWW file?", {
          title: "Export JWW",
          kind: "warning",
        });
        if (!overwrite) {
          return;
        }
        report = await exportJwwFromDesktop(
          projectState.project_path,
          drawing,
          outputPath,
          allowLossy,
          true,
        );
      }
      setExportReport(report);
      if (report.status === "exported") {
        setImportMessage(`JWW exported with ${report.warnings.length} warning(s).`);
        setExportOpen(false);
      }
    } catch (error: unknown) {
      setImportMessage(formatError(error, "JWW export failed"));
    } finally {
      setExportBusy(false);
    }
  }

  async function performCompatibleJwwSave() {
    if (!isDesktop || projectState === null || artifacts === null) {
      return;
    }
    const outputPath = await save({
      title: "Save JWW Compatible",
      defaultPath: `${projectState.project_name}.jww`,
      filters: [{ name: "JWW", extensions: ["jww"] }],
    });
    if (typeof outputPath !== "string") {
      return;
    }
    setExportBusy(true);
    try {
      let report: ExportReport;
      try {
        report = await exportJwwPreservingFromDesktop(
          projectState.project_path,
          artifacts.currentDrawing,
          outputPath,
          false,
        );
      } catch (error: unknown) {
        const message = formatError(error, "compatible JWW save failed");
        if (!message.includes("output already exists")) {
          throw error;
        }
        const overwrite = await confirm("Replace the existing JWW file?", {
          title: "Save JWW Compatible",
          kind: "warning",
        });
        if (!overwrite) {
          return;
        }
        report = await exportJwwPreservingFromDesktop(
          projectState.project_path,
          artifacts.currentDrawing,
          outputPath,
          true,
        );
      }
      setExportReport(report);
      if (report.status === "exported") {
        setImportMessage(report.mode === "preserved_exact"
          ? "JWW saved byte-for-byte from the preserved original."
          : "JWW saved in compatibility mode; review the file-level compatibility report.");
      } else {
        setImportMessage(`Compatible JWW save blocked: ${report.blockers.map((issue) => issue.message).join("; ")}`);
      }
    } catch (error: unknown) {
      setExportReport({
        schema_version: "0.3",
        status: "blocked",
        mode: "preserved_edited",
        output_path: outputPath,
        written_entities: 0,
        expanded_entities: 0,
        warnings: [],
        blockers: [{ code: "preserve_failed", message: formatError(error, "compatible JWW save failed") }],
      });
      setImportMessage(formatError(error, "compatible JWW save failed"));
    } finally {
      setExportBusy(false);
    }
  }

  async function performOriginalJwwExtraction() {
    if (!isDesktop || projectState === null) {
      return;
    }
    const outputPath = await save({
      title: "Extract Original JWW",
      defaultPath: `${projectState.project_name}-original.jww`,
      filters: [{ name: "JWW", extensions: ["jww"] }],
    });
    if (typeof outputPath !== "string") {
      return;
    }
    try {
      await extractOriginalJwwFromDesktop(projectState.project_path, outputPath, false);
      setImportMessage("Original JWW extracted byte-for-byte.");
    } catch (error: unknown) {
      const message = formatError(error, "original JWW extraction failed");
      if (!message.includes("output already exists")) {
        setImportMessage(message);
        return;
      }
      const overwrite = await confirm("Replace the existing JWW file?", {
        title: "Extract Original JWW",
        kind: "warning",
      });
      if (overwrite) {
        await extractOriginalJwwFromDesktop(projectState.project_path, outputPath, true);
        setImportMessage("Original JWW extracted byte-for-byte.");
      }
    }
  }

  async function performPdfExport() {
    if (!isDesktop || projectState === null || artifacts === null || historyState === null || pdfBusy) return;
    const sequence = pdfExportGuard.begin();
    const activeLayout = artifacts.layouts?.find((layout) => layout.active);
    const outputPath = import.meta.env.VITE_CAD_DESKTOP_E2E === "1"
      ? import.meta.env.VITE_CAD_E2E_PDF_PATH
      : await save({
          title: "Export PDF",
          defaultPath: `${projectState.project_name}-${artifacts.currentDrawing}.pdf`,
          filters: [{ name: "PDF", extensions: ["pdf"] }],
        });
    if (typeof outputPath !== "string") return;
    if (!pdfExportGuard.accepts(sequence)) return;
    setPdfBusy(true);
    try {
      await exportDrawingPdfFromDesktop({
        projectPath: projectState.project_path,
        drawing: artifacts.currentDrawing,
        layout: activeLayout?.id ?? null,
        outputPath,
        overwrite: false,
        expectedFiles: historyState?.current_files ?? [],
      });
      if (pdfExportGuard.accepts(sequence)) {
        setEditMessage("PDF exported from the active layout");
      }
    } catch (error: unknown) {
      const message = formatError(error, "PDF export failed");
      if (!pdfExportGuard.accepts(sequence)) return;
      if (message.includes("output already exists")) {
        const overwrite = await confirm("Replace the existing PDF file?", {
          title: "Export PDF",
          kind: "warning",
        });
        if (overwrite && pdfExportGuard.accepts(sequence)) {
          try {
            await exportDrawingPdfFromDesktop({
              projectPath: projectState.project_path,
              drawing: artifacts.currentDrawing,
              layout: activeLayout?.id ?? null,
              outputPath,
              overwrite: true,
              expectedFiles: historyState?.current_files ?? [],
            });
            if (pdfExportGuard.accepts(sequence)) {
              setEditMessage("PDF exported from the active layout");
            }
          } catch (retryError: unknown) {
            if (pdfExportGuard.accepts(sequence)) {
              const retryMessage = formatError(retryError, "PDF export failed");
              setEditMessage(retryMessage);
              if (shouldRefreshDesktopReview(retryMessage)) {
                enqueueDesktopReview(projectState.project_path);
              }
            }
          }
        }
      } else {
        setEditMessage(message);
        if (shouldRefreshDesktopReview(message)) {
          enqueueDesktopReview(projectState.project_path);
        }
      }
    } finally {
      setPdfBusy(false);
    }
  }

  function rerunReview() {
    if (isDesktop && projectState !== null) {
      enqueueDesktopReview(projectState.project_path);
      return;
    }
    loadWebArtifacts();
  }

  function handleProjectWatchEvent(event: ProjectWatchEvent) {
    const currentProject = projectStateRef.current;
    if (currentProject === null || event.project_path !== currentProject.project_path) {
      return;
    }
    if (event.kind === "error") {
      const message = event.message ?? "project watcher failed";
      watchActiveRef.current = false;
      watchErrorRef.current = message;
      reviewQueue.reset();
      setLiveReviewState({
        status: "error",
        message,
      });
      return;
    }
    watchActiveRef.current = true;
    watchErrorRef.current = null;
    enqueueDesktopReview(currentProject.project_path);
  }

  function enqueueDesktopReview(projectPath: string) {
    historySequenceRef.current += 1;
    setLiveReviewState({ status: "refreshing" });
    reviewQueue.enqueue(
      projectPath,
      (loaded) => {
        if (projectStateRef.current?.project_path !== projectPath) {
          return;
        }
        if (!drawingLoadGuard.acceptsCurrent(loaded.artifacts.currentDrawing)) {
          return;
        }
        drawingLoadGuard.reset(loaded.artifacts.currentDrawing);
        const reviewedState = mergeProjectStateFromReview(projectStateRef.current, loaded);
        projectStateRef.current = reviewedState;
        setProjectState(reviewedState);
        currentDrawingRef.current = loaded.artifacts.currentDrawing;
        setArtifacts(loaded.artifacts);
        const continueInsertion = editorModeRef.current === "insert_block" && insertionRevision.current === loaded.artifacts.editor.revision;
        const keepIdleTool = draftActivityRef.current.points.length === 0 && draftActivityRef.current.operation === null;
        // Watcher catch-up after our own save must not cancel the next command.
        // SourceChecked still rejects changes to rules or blocks after a preview.
        const sameDrawingRevision = currentEditorRevisionRef.current === loaded.artifacts.editor.revision;
        if (!sameDrawingRevision) ++asyncDraftSequence.current;
        if (!keepIdleTool && !sameDrawingRevision) {
          setEditorMode(continueInsertion ? "insert_block" : "select");
          setCadCommand(continueInsertion ? "insert_block" : "select");
          setDraftPoints([]);
          setPendingOperation(null);
          setSnapCandidate(null);
        }
        dragInteraction.current = null;
        const availableEntityIds = new Set(loaded.artifacts.editor.entities.map((entity) => entity.id));
        setSelectedEntityIds((entityIds) => new Set(
          Array.from(entityIds).filter((entityId) => availableEntityIds.has(entityId)),
        ));
        setSelectedEntityId((entityId) =>
          entityId === "" || availableEntityIds.has(entityId) ? entityId : "",
        );
        setLoadState("ready");
        if (watchActiveRef.current) {
          setLiveReviewState({ status: "watching" });
        } else {
          setLiveReviewState({
            status: "error",
            message: watchErrorRef.current ?? "project watcher is unavailable",
          });
        }
      },
      (error: unknown) => {
        if (projectStateRef.current?.project_path !== projectPath) {
          return;
        }
        setLiveReviewState({
          status: "error",
          message: formatError(error, "live review failed"),
        });
      },
    );
  }

  async function changeDrawing(drawing: string) {
    if (!isDesktop || projectState === null || drawing === artifacts?.currentDrawing) {
      return;
    }
    setLoadState("loading");
    setSelectedEntityId("");
    setSelectedEntityIds(new Set());
    setEditorMode("select");
    setCadCommand("select");
    setDraftPoints([]);
    setPendingOperation(null);
    setSnapCandidate(null);
    setHistoryState(null);
    setHistoryMessage("");
    reviewQueue.reset();
    const token = drawingLoadGuard.begin(drawing);
    currentDrawingRef.current = drawing;
    try {
      const loaded = await loadReviewSnapshotFromDesktop(
        projectState.project_path,
        sanitizeSvg,
        undefined,
        drawing,
      );
      if (!drawingLoadGuard.accepts(token, loaded.artifacts.currentDrawing)) {
        return;
      }
      drawingLoadGuard.reset(loaded.artifacts.currentDrawing);
      currentDrawingRef.current = loaded.artifacts.currentDrawing;
      setArtifacts(loaded.artifacts);
      setLoadState("ready");
      setPreviousViewBox(null);
    } catch (error: unknown) {
      const restoredDrawing = drawingLoadGuard.rollback(token);
      if (restoredDrawing === undefined) {
        return;
      }
      currentDrawingRef.current = restoredDrawing;
      setErrorMessage(formatError(error, "failed to switch drawing"));
      setLoadState("error");
    }
  }

  function selectEditorMode(mode: EditorMode) {
    ++asyncDraftSequence.current;
    insertionRevision.current = null;
    setDraftParameterError(false);
    pendingRevision.current = null;
    editorModeRef.current = mode;
    setEditorMode(mode);
    setCadCommand(mode);
    setDraftPoints([]);
    setPendingOperation(null);
    setSnapCandidate(null);
    setEditPreview(null);
    endpointDrag.current = null;
    setEditMessage(mode === "stretch" ? "Pick two rectangle corners, a base point, then a destination" : mode === "endpoint" ? "Drag a vertex handle, or choose a handle and enter its coordinate" : "Pick geometry or enter coordinates; Enter applies, Esc cancels");
    if (mode !== "select") {
      setViewMode("sheet");
    }
  }

  function editSelectedBlock() {
    setBlockDeletion(null);
    const block = selectedEditorEntity?.block;
    if (selectedEditorEntity?.type !== "block_ref" || typeof block !== "string") {
      setEditMessage("Select a block reference to edit");
      return;
    }
    setDraftingOptions(options => ({ ...options, block, blockName: artifacts?.blocks?.find(candidate => candidate.id === block)?.name ?? "" }));
    selectEditorMode("edit_block");
    const sequence = ++blockLoadSequence.current;
    const projectPath = projectState?.project_path, drawing = artifacts?.currentDrawing;
    const accepts = () => sequence === blockLoadSequence.current && editorModeRef.current === "edit_block" && projectStateRef.current?.project_path === projectPath && currentDrawingRef.current === drawing;
    if (projectState !== null && artifacts !== null) void invoke<{ block: string; name: string; revision: string; entities: EditorEntity[]; svg: string }>("load_block_contents", { projectPath, drawing, block }).then(result => {
      if (!accepts()) return;
      setBlockEditing({ ...result, drawingRevision: artifacts.editor.revision, svg: sanitizeSvg(result.svg) }); setBlockEntityIndex(0);
    }).catch(error => { if (accepts()) setEditMessage(formatError(error, "Unable to open block")); });
  }

  async function saveBlockOperation(operation: Record<string, unknown>, expectedRevision: string) {
    if (projectState === null || artifacts === null || isEditSaving) return;
    setIsEditSaving(true);
    try {
      await invoke("apply_block_contents", { projectPath: projectState.project_path, request: { drawing: artifacts.currentDrawing, expected_revision: expectedRevision, operation } });
      setBlockEditing(null); selectEditorMode("select");
      setEditMessage("Block saved"); enqueueDesktopReview(projectState.project_path);
    } catch (error) { const message = formatError(error, "Block edit failed"); setEditMessage(message); setBlockEditMessage(message); }
    finally { setIsEditSaving(false); }
  }

  function replaceBlockEntity(entity: EditorEntity) {
    if (blockEditing === null || projectState === null || artifacts === null) return;
    const entities = blockEditing.entities.map(current => current.id === entity.id ? entity : current);
    stageBlockEntities(entities);
  }

  function stageBlockEntities(entities: EditorEntity[]) {
    if (blockEditing === null || projectState === null || artifacts === null) return;
    setBlockEditing({ ...blockEditing, entities });
    void invoke<{ svg: string }>("load_block_contents", { projectPath: projectState.project_path, drawing: artifacts.currentDrawing, block: blockEditing.block, entities }).then(result => setBlockEditing(current => current?.entities === entities ? { ...current, svg: sanitizeSvg(result.svg) } : current)).catch(error => setBlockEditMessage(formatError(error, "Invalid block preview")));
  }

  async function resolveBlockDeletion() {
    if (blockEditing === null || blockDeletion === null || projectState === null || artifacts === null) return;
    const original = blockEditing.entities;
    try {
      const result = await invoke<{ entities: EditorEntity[]; svg: string }>("load_block_contents", {
        projectPath: projectState.project_path, drawing: artifacts.currentDrawing, block: blockEditing.block,
        entities: original, removeEntityId: blockDeletion.id, dimensionResolutions: blockDeletion.resolutions,
      });
      setBlockEditing(current => current?.entities === original ? { ...current, entities: result.entities, svg: sanitizeSvg(result.svg) } : current);
      setBlockDeletion(null); setBlockEntityIndex(0);
    } catch (error) { setBlockEditMessage(formatError(error, "Unable to resolve block dimensions")); }
  }

  useEffect(() => {
    let active = true;
    setBlockThumbnail("");
    const block = draftingOptions.block || artifacts?.blocks?.[0]?.id;
    if (editorMode === "insert_block" && block && projectState !== null && artifacts !== null) void invoke<{ svg: string }>("load_block_contents", { projectPath: projectState.project_path, drawing: artifacts.currentDrawing, block }).then(result => { if (active) setBlockThumbnail(sanitizeSvg(result.svg)); }).catch(() => { if (active) setBlockThumbnail(""); });
    return () => { active = false; };
  }, [editorMode, draftingOptions.block, projectState?.project_path, artifacts?.editor.revision]);

  function applyDraftingParameters() {
    if (draftParameterError) { setEditMessage("Correct invalid parameters before applying"); return; }
    if (pendingOperation !== null) { void commitDrawingEdit(pendingOperation); return; }
    if (artifacts === null) return;
    const layer = activeLayerEditable();
    const points = draftPoints;
    const number = (value: string) => finiteDraftNumber(value);
    if (["line", "rectangle", "hatch", "text", "polyline"].includes(editorMode) && layer === null) {
      setEditMessage("Choose a visible, unlocked active layer in the layer list");
      return;
    }
    if (editorMode === "line" && points.length === 1 && layer !== null) {
      const length = number(draftingOptions.distance), angle = number(draftingOptions.angle);
      if (length === null || angle === null || length <= 0) { setEditMessage("Length must be positive and angle finite"); return; }
      const radians = angle * Math.PI / 180;
      setPendingOperation({ kind: "create", entity: { type: "line", layer, pen: null, p1: points[0], p2: [points[0][0] + length * Math.cos(radians), points[0][1] + length * Math.sin(radians)] } });
    } else if (editorMode === "rectangle" && points.length === 1 && layer !== null) {
      const width = number(draftingOptions.width), height = number(draftingOptions.height);
      if (width === null || height === null || width <= 0 || height <= 0) { setEditMessage("Width and height must be positive"); return; }
      setPendingOperation({ kind: "rectangle", layer, p1: points[0], p2: [points[0][0] + width, points[0][1] + height] });
    } else if (editorMode === "hatch" && !draftingOptions.hatchRegion && layer !== null) {
      if (points.length < 3) { setEditMessage("Pick at least three boundary vertices"); return; }
      stageHatch(points.length > 3 && pointArrayDistance(points[0], points.at(-1)!) < 1e-9 ? [points.slice(0, -1)] : [points], layer);
    } else if (editorMode === "rectangular_array") {
      const rows = number(draftingOptions.rows), columns = number(draftingOptions.columns);
      const row_spacing = number(draftingOptions.rowSpacing), column_spacing = number(draftingOptions.columnSpacing);
      if (rows === null || columns === null || row_spacing === null || column_spacing === null || !Number.isInteger(rows) || !Number.isInteger(columns) || rows < 1 || columns < 1) { setEditMessage("Rows and columns must be positive integers; spacing must be finite"); return; }
      setPendingOperation({ kind: "rectangular_array", entity_ids: [...selectedEntityIds], rows, columns, row_spacing, column_spacing });
    } else if (editorMode === "edit_block" && draftingOptions.blockName.trim()) {
      void commitDrawingEdit({ kind: "update_block_definition", block: draftingOptions.block, properties: { name: draftingOptions.blockName.trim() } });
    } else if (editorMode === "create_block" && points.length === 1) {
      const name = draftingOptions.blockName.trim();
      if (!name || selectedEntityIds.size === 0) { setEditMessage("Select entities and enter a block name"); return; }
      void saveBlockOperation({ type: "create", block: `block_${crypto.randomUUID().replaceAll("-", "")}`, name, base_point: points[0], entity_ids: [...selectedEntityIds], replace_originals: draftingOptions.replaceOriginals, detach_external_dimensions: draftingOptions.detachExternalDimensions }, artifacts.editor.revision);
    } else if (editorMode === "text" && points.length > 0) {
      stageText(points[0]);
    } else if (editorMode === "rotate" && points.length > 0) {
      const angle = number(draftingOptions.angle);
      if (angle === null) { setEditMessage("Enter a finite angle"); return; }
      setPendingOperation({ kind: "rotate", entity_ids: [...selectedEntityIds], center: points[0], angle_deg: angle });
    } else if (editorMode === "polyline") {
      completePolyline();
    } else {
      setEditMessage("Complete the point selection before applying");
    }
  }

  function stageText(at: [number, number]) {
    const layer = activeLayerEditable();
    const style = draftingOptions.style || artifacts?.editor.text_styles[0];
    if (layer === null || !style || !draftingOptions.text.trim()) { setEditMessage("Enter text and choose a style"); return; }
    setPendingOperation({ kind: "create", entity: { type: "text", layer, pen: null, style, at, rotation_deg: 0, mirror_y: false, value: draftingOptions.text } });
  }

  function stageHatch(loops: [number, number][][], layer: string) {
    const scale = finiteDraftNumber(draftingOptions.distance), angle = finiteDraftNumber(draftingOptions.angle);
    const fill = draftingOptions.fill || artifacts?.editor.fills[0];
    if (!fill || scale === null || scale <= 0 || angle === null) { setEditMessage("Choose a fill and finite angle with a positive pitch"); return; }
    setPendingOperation({ kind: "create", entity: { type: "hatch", layer, pen: null, loops, pattern: draftingOptions.pattern, angle_deg: angle, scale, fill } });
  }

  function activeLayerEditable(): string | null {
    if (!isDesktop || artifacts === null) {
      return null;
    }
    const active = artifacts.layers.active_layer;
    const layer = artifacts.layers.layers.find((candidate) => candidate.id === active);
    const group = artifacts.layers.groups.find(
      (candidate) => candidate.id === (layer?.group ?? "default"),
    );
    if (layer === undefined || !layer.visible || layer.locked || group?.visible === false || group?.locked === true) {
      return null;
    }
    return layer.id;
  }

  async function commitDrawingEdit(operation: EditOperation) {
    if (operation === pendingOperation && draftParameterError) {
      setEditMessage("Correct invalid parameters before applying");
      return;
    }
    if (isEditSaving || !isDesktop || projectState === null || artifacts === null) {
      return;
    }
    setIsEditSaving(true);
    setEditMessage("Saving edit...");
    try {
      const expectedRevision = (operation === pendingOperation ? pendingRevision.current : null) ?? expectedRevisionForOperation(
        operation,
        artifacts.editor.revision,
        artifacts.blocks,
        artifacts.layouts,
      );
      if (expectedRevision === null) {
        throw new Error(`revision is unavailable for ${operation.kind}`);
      }
      let sourceFiles = operation === pendingOperation ? editPreview?.source_files : undefined;
      if (!["update_layout", "update_block_definition", "resolve_dimensions", "source_checked"].includes(operation.kind)) {
        const preview = await previewDrawingEdit(projectState.project_path, { drawing: artifacts.currentDrawing, expected_revision: expectedRevision, operation });
        sourceFiles ??= preview.source_files;
        if (preview.dimension_impacts.length > 0) {
          if (operation === pendingOperation && preview.dimension_impacts.every(id => dimensionResolutions[id] !== undefined)) {
            operation = { kind: "resolve_dimensions", operation, resolutions: preview.dimension_impacts.map(entity_id => ({ entity_id, action: dimensionResolutions[entity_id] })) };
          } else {
            setPendingOperation(operation);
            setEditPreview({ ...preview, svg: sanitizeSvg(preview.svg) });
            setEditMessage("Choose how to resolve each affected dimension before applying");
            return;
          }
        }
      }
      if (sourceFiles !== undefined && operation.kind !== "source_checked") operation = { kind: "source_checked", operation, expected_files: sourceFiles };
      const result = await applyDrawingEdit(projectState.project_path, {
        drawing: artifacts.currentDrawing,
        expected_revision: expectedRevision,
        operation,
      });
      if (operation.kind !== "update_block_definition" && operation.kind !== "update_layout") {
        setArtifacts((current) =>
          current === null
            ? current
            : { ...current, editor: { ...current.editor, revision: result.revision } },
        );
      }
      if (result.operation === "delete") {
        setSelectedEntityIds(new Set());
        setSelectedEntityId("");
      } else {
        const resultIds = result.entity_ids.length > 0
          ? result.entity_ids
          : (result.entity_id === null || result.entity_id === undefined ? [] : [result.entity_id]);
        setSelectedEntityIds(new Set(resultIds));
        setSelectedEntityId(resultIds[0] ?? "");
      }
      setDraftPoints([]);
      setPendingOperation(null);
      setEditPreview(null);
      setSnapCandidate(null);
      if (repeatableCommand(editorMode)) lastSuccessfulCommand.current = editorMode;
      insertionRevision.current = editorMode === "insert_block" ? result.revision : null;
      setEditorMode(editorMode === "insert_block" ? "insert_block" : "select");
      setCadCommand(editorMode === "insert_block" ? "insert_block" : "select");
      setEditMessage(`${result.operation} saved`);
      enqueueDesktopReview(projectState.project_path);
    } catch (error: unknown) {
      const message = formatError(error, "edit failed");
      setEditMessage(message);
      if (message.includes("revision_conflict")) {
        enqueueDesktopReview(projectState.project_path);
      }
    } finally {
      setIsEditSaving(false);
    }
  }

  async function deleteSelectedEntities() {
    if (!isDesktop || projectState === null || artifacts === null) {
      return;
    }
    const entityIds = selectedEntityIds.size > 0
      ? Array.from(selectedEntityIds)
      : (selectedEntityId === "" ? [] : [selectedEntityId]);
    if (entityIds.length === 0) {
      setEditMessage("Select one or more entities first");
      return;
    }
    if (!await confirm(`Delete ${entityIds.length} selected entit${entityIds.length === 1 ? "y" : "ies"}?`, { title: "Delete entities", kind: "warning" })) {
      setCadCommand("select");
      return;
    }
    await commitDrawingEdit(entityIds.length > 1
      ? { kind: "delete_many", entity_ids: entityIds }
      : { kind: "delete", entity_id: entityIds[0] });
  }

  async function runHistoryAction(undo: boolean) {
    if (isHistoryBusy || isEditSaving || !isDesktop || projectState === null || artifacts === null) {
      return;
    }
    const available = undo ? historyState?.undo.length : historyState?.redo.length;
    if (!available) {
      setHistoryMessage(undo ? "Nothing to undo" : "Nothing to redo");
      setCadCommand("select");
      return;
    }
    setIsHistoryBusy(true);
    setHistoryMessage(undo ? "Undoing edit..." : "Redoing edit...");
    const requestSequence = historySequenceRef.current;
    const requestDrawing = artifacts.currentDrawing;
    try {
      const request = {
        drawing: requestDrawing,
        expected_files: historyState?.current_files ?? [],
      };
      const result = undo
        ? await undoDrawingEdit(projectState.project_path, request)
        : await redoDrawingEdit(projectState.project_path, request);
      if (
        requestSequence !== historySequenceRef.current
        || currentDrawingRef.current !== requestDrawing
      ) {
        return;
      }
      setArtifacts((current) =>
        current === null
          ? current
          : { ...current, editor: { ...current.editor, revision: result.revision } },
      );
      setSelectedEntityIds(new Set(result.entity_ids));
      setSelectedEntityId(result.entity_ids[0] ?? "");
      setEditorMode("select");
      setCadCommand("select");
      setDraftPoints([]);
      setPendingOperation(null);
      setSnapCandidate(null);
      setHistoryMessage(`${undo ? "Undo" : "Redo"} applied`);
      enqueueDesktopReview(projectState.project_path);
    } catch (error: unknown) {
      const message = formatError(error, undo ? "undo failed" : "redo failed");
      setHistoryMessage(message);
      if (message.includes("revision_conflict") || message.includes("history")) {
        enqueueDesktopReview(projectState.project_path);
      }
    } finally {
      setIsHistoryBusy(false);
    }
  }

  function updateSnapForPointer(event: PointerEvent) {
    if (!snapEnabled || event.shiftKey || projectState === null || artifacts === null) {
      setSnapCandidate(null);
      return;
    }
    const svgPoint = clientPointToSvg({ x: event.clientX, y: event.clientY });
    const stage = drawingStageRef.current;
    const viewBox = currentViewBoxRef.current;
    if (svgPoint === null || stage === null || viewBox === null) {
      return;
    }
    const sequence = ++snapSequenceRef.current;
    const tolerance = (viewBox.width / Math.max(stage.clientWidth, 1)) * 10;
    if (snapTimerRef.current !== null) {
      clearTimeout(snapTimerRef.current);
    }
    const request = {
      projectPath: projectState.project_path,
      drawing: artifacts.currentDrawing,
      revision: artifacts.editor.revision,
      point: [svgPoint.x, -svgPoint.y] as [number, number],
    };
    snapTimerRef.current = setTimeout(() => {
      snapTimerRef.current = null;
      void queryDrawingSnap(
        request.projectPath,
        request.drawing,
        request.revision,
        request.point,
        tolerance,
        undefined,
        undefined,
        draftPoints.at(-1) ?? null,
      )
        .then((candidate) => {
          if (sequence === snapSequenceRef.current) setSnapCandidate(candidate);
        })
        .catch(() => {
          if (sequence === snapSequenceRef.current) setSnapCandidate(null);
        });
    }, 16);
  }

  function clickedCadPoint(event: MouseEvent): [number, number] | null {
    const svgPoint = clientPointToSvg({ x: event.clientX, y: event.clientY });
    const point = snapCandidate !== null && snapEnabled && !event.shiftKey
      ? snapCandidate.point
      : svgPoint === null ? null : [svgPoint.x, -svgPoint.y] as [number, number];
    return point === null ? null : applyOrtho(point, draftPoints.at(-1) ?? null, orthoEnabled);
  }

  function completePolyline() {
    const layer = activeLayerEditable();
    if (layer === null || draftPoints.length < 2) {
      return;
    }
    void commitDrawingEdit({
      kind: "create",
      entity: { type: "polyline", layer, pen: null, points: draftPoints, closed: false },
    });
  }

  function handleDraftClick(event: MouseEvent): boolean {
    if (editorMode === "select" || viewMode !== "sheet") {
      return false;
    }
    const point = clickedCadPoint(event);
    if (point === null) {
      return true;
    }
    return handleDraftPoint(point);
  }

  function handleDraftPoint(point: [number, number]): boolean {
    if (editorMode === "select" || viewMode !== "sheet" || artifacts === null) {
      return false;
    }
    if (editorMode === "endpoint") {
      const target = endpointDrag.current;
      if (target === null) { setEditMessage("Choose a vertex handle first"); return true; }
      target.to = point;
      setPendingOperation({ kind: "endpoint", entity_id: target.entityId, vertex_index: target.vertexIndex, to: point });
      return true;
    }
    if (editorMode === "stretch") {
      const points = [...draftPoints, point];
      setDraftPoints(points);
      if (points.length === 4) setPendingOperation({ kind: "stretch", entity_ids: artifacts.editor.entities.map(entity => entity.id), min: [Math.min(points[0][0], points[1][0]), Math.min(points[0][1], points[1][1])], max: [Math.max(points[0][0], points[1][0]), Math.max(points[0][1], points[1][1])], delta: [points[3][0] - points[2][0], points[3][1] - points[2][1]] });
      return true;
    }
    if (editorMode === "rectangle") {
      const layer = activeLayerEditable();
      const points = [...draftPoints, point];
      setDraftPoints(points);
      if (layer !== null && points.length === 2) {
        setDraftingOptions(options => ({ ...options, width: String(Math.abs(points[1][0] - points[0][0])), height: String(Math.abs(points[1][1] - points[0][1])) }));
        setPendingOperation({ kind: "rectangle", layer, p1: points[0], p2: points[1] });
      }
      return true;
    }
    if (editorMode === "fillet" || editorMode === "chamfer") {
      const ids = [...selectedEntityIds];
      if (ids.length !== 2) { setEditMessage("Select exactly two lines first; then pick the retained side of each line"); return true; }
      const points = [...draftPoints, point];
      setDraftPoints(points);
      if (points.length < 2) return true;
      const first = finiteDraftNumber(draftingOptions.distance), second = finiteDraftNumber(draftingOptions.secondDistance);
      if (first === null || second === null || first <= 0 || second <= 0) { setEditMessage("Distances must be positive"); return true; }
      const common = { first_entity_id: ids[0], second_entity_id: ids[1], first_pick: points[0], second_pick: points[1] };
      setPendingOperation(editorMode === "fillet" ? { kind: "fillet", ...common, radius: first } : { kind: "chamfer", ...common, first_distance: first, second_distance: second });
      return true;
    }
    if (editorMode === "create_block") {
      setDraftPoints([point]);
      setEditMessage("Base point set; enter a block name then Apply");
      return true;
    }
    if (editorMode === "rectangular_array" || editorMode === "edit_block") return true;
    if (editorMode === "dimension") {
      const points = [...draftPoints, point];
      setDraftPoints(points);
      const layer = activeLayerEditable(), style = artifacts.editor.dimension_styles[0];
      if (layer === null || style === undefined) { setEditMessage("Choose an editable layer and a dimension style"); return true; }
      const type = draftingOptions.dimension;
      const selected = artifacts.editor.entities.filter(entity => selectedEntityIds.has(entity.id));
      const fixed = (point: [number, number]) => ({ kind: "fixed", point });
      const reference = (entity: EditorEntity, feature: string) => ({ kind: "entity", entity_id: entity.id, feature });
      const anchor = (point: [number, number]) => draftingOptions.associate ? dimensionAnchorAt(artifacts, point) : fixed(point);
      const create = (p1: [number, number], p2: [number, number], offset: number, measurement: Record<string, unknown>): EditOperation => ({ kind: "create", entity: { type: "dimension", layer, pen: null, style, p1, p2, offset, text_rotation_deg: 0, text_mirror_y: false, value: null, measurement } });
      if (type === "radius" || type === "diameter") {
        const target = selected.find(entity => entity.type === "circle" || entity.type === "arc");
        const center = asCadPoint(target?.center);
        if (!target || center === null || typeof target.radius !== "number") { setEditMessage("Select a circle or arc, then pick the dimension position"); return true; }
        const rim: [number, number] = [center[0] + target.radius, center[1]];
        setPendingOperation(create(center, rim, pointArrayDistance(center, point), { kind: type, center: draftingOptions.associate ? reference(target, "center") : fixed(center), rim: draftingOptions.associate ? reference(target, "radius") : fixed(rim) }));
      } else if (type === "angle" && selected.length === 2 && selected.every(entity => entity.type === "line")) {
        const first = selected[0], second = selected[1];
        const lineAnchor = (entity: EditorEntity, feature: "start" | "end") => draftingOptions.associate ? reference(entity, feature) : fixed(asCadPoint(feature === "start" ? entity.p1 : entity.p2)!);
        setPendingOperation(create(asCadPoint(first.p1)!, asCadPoint(first.p2)!, Math.max(pointArrayDistance(point, asCadPoint(first.p1)!), 1), { kind: "angle_lines", first_start: lineAnchor(first, "start"), first_end: lineAnchor(first, "end"), second_start: lineAnchor(second, "start"), second_end: lineAnchor(second, "end") }));
      } else if (type === "chain" || type === "baseline") {
        if (points.length < 3) { setEditMessage("Pick the first two measurement points, then the dimension offset; subsequent points add dimensions"); return true; }
        const pairs = points.length === 3 ? [[points[0], points[1]]] : Array.from({ length: points.length - 2 }, (_, i) => [type === "baseline" ? points[0] : (i === 0 ? points[0] : i === 1 ? points[1] : points[i + 1]), i === 0 ? points[1] : points[i + 2]]);
        setPendingOperation({ kind: "batch", operations: pairs.map(([a, b], index) => create(a, b, signedLineOffset(a, b, points[2]) + (type === "baseline" ? index * 10 : 0), { kind: "aligned", first: anchor(a), second: anchor(b) })) });
      } else if (points.length >= 3) {
        const measurement = type === "angle" ? { kind: "angle", vertex: anchor(points[0]), first: anchor(points[1]), second: anchor(points[2]) } : { kind: type, first: anchor(points[0]), second: anchor(points[1]) };
        setPendingOperation(create(points[0], points[1], type === "angle" ? pointArrayDistance(points[0], points[1]) / 2 : signedLineOffset(points[0], points[1], points[2]), measurement));
      }
      return true;
    }
    if (editorMode === "move" || editorMode === "copy") {
      if (selectedEntityId === "") {
        setEditMessage("Select an entity first");
        return true;
      }
      if (draftPoints.length === 0) {
        setDraftPoints([point]);
        return true;
      }
      const entityIds = selectedEntityIds.size > 0 ? Array.from(selectedEntityIds) : [selectedEntityId];
      const delta: [number, number] = [point[0] - draftPoints[0][0], point[1] - draftPoints[0][1]];
      void commitDrawingEdit(entityIds.length > 1
        ? { kind: "translate_many", entity_ids: entityIds, delta, duplicate: editorMode === "copy" }
        : { kind: "translate", entity_id: selectedEntityId, delta, duplicate: editorMode === "copy" });
      return true;
    }
    if (["rotate", "mirror", "offset", "trim", "extend"].includes(editorMode)) {
      const entityIds = Array.from(selectedEntityIds);
      if (entityIds.length === 0) {
        setEditMessage("Select one or more entities first");
        return true;
      }
      if (editorMode === "rotate") {
        setDraftPoints([point]);
        const angle = finiteDraftNumber(draftingOptions.angle);
        if (angle !== null) {
          setPendingOperation({ kind: "rotate", entity_ids: entityIds, center: point, angle_deg: angle });
          setEditMessage("Rotate preview ready; press Enter to apply");
        } else {
          setEditMessage("Enter a finite rotation angle");
        }
        return true;
      }
      if (editorMode === "mirror") {
        const points = [...draftPoints, point];
        setDraftPoints(points);
        if (points.length < 2) return true;
        setPendingOperation({ kind: "mirror", entity_ids: entityIds, axis_start: points[0], axis_end: points[1] });
        setEditMessage("Mirror preview ready; press Enter to apply");
        return true;
      }
      if (editorMode === "offset") {
        const distance = finiteDraftNumber(draftingOptions.distance);
        if (distance !== null && distance > 0) {
          const operations: EditOperation[] = entityIds.map(id => ({ kind: "offset", entity_ids: [id], distance: offsetSide(artifacts.editor.entities.find(entity => entity.id === id), point) * distance }));
          setPendingOperation({ kind: "batch", operations });
          setEditMessage("Offset preview ready; press Enter to apply");
        } else {
          setEditMessage("Enter a finite offset distance");
        }
        return true;
      }
      if (entityIds.length < 2) {
        setEditMessage(`Select a target and ${editorMode === "trim" ? "cutter" : "boundary"} entity`);
        return true;
      }
      const targetEntityId = selectedEntityId || entityIds[0];
      const otherEntityId = entityIds.find((id) => id !== targetEntityId);
      if (otherEntityId === undefined) {
        setEditMessage("Select two different entities");
        return true;
      }
      setPendingOperation(editorMode === "trim"
        ? { kind: "trim", target_entity_id: targetEntityId, cutter_entity_id: otherEntityId, pick_point: point }
        : { kind: "extend", target_entity_id: targetEntityId, boundary_entity_id: otherEntityId, pick_point: point });
      setEditMessage(`${editorMode === "trim" ? "Trim" : "Extend"} preview ready; press Enter to apply`);
      return true;
    }
    const layer = activeLayerEditable();
    if (layer === null) {
      setEditMessage("Choose a visible, unlocked active layer");
      return true;
    }
    if (editorMode === "point") {
      void commitDrawingEdit({
        kind: "create",
        entity: { type: "point", layer, pen: null, at: point, temporary: false, marker_code: null, rotation_deg: 0, scale: 1 },
      });
      return true;
    }
    if (editorMode === "text") {
      setDraftPoints([point]);
      stageText(point);
      return true;
    }
    if (editorMode === "insert_block") {
      const block = artifacts.blocks?.find(block => block.id === draftingOptions.block) ?? artifacts.blocks?.[0];
      if (block === undefined) {
        setEditMessage("No block definition is available");
        return true;
      }
      const angle = finiteDraftNumber(draftingOptions.angle), scale = finiteDraftNumber(draftingOptions.scale);
      if (angle === null || scale === null || scale <= 0) { setEditMessage("Choose a finite angle and positive scale"); return true; }
      setPendingOperation({
        kind: "insert_block",
        block: block.id,
        layer,
        at: point,
        rotation_deg: angle,
        scale,
        mirror_x: draftingOptions.mirrorX,
        mirror_y: draftingOptions.mirrorY,
      });
      return true;
    }
    if (editorMode === "hatch") {
      if (draftingOptions.hatchRegion && projectState !== null) {
        const sequence = ++asyncDraftSequence.current, drawing = artifacts.currentDrawing, projectPath = projectState.project_path;
        void invoke<[number, number][][]>("find_hatch_region", { projectPath, drawing, point }).then(loops => {
          if (sequence === asyncDraftSequence.current && editorModeRef.current === "hatch" && projectStateRef.current?.project_path === projectPath && currentDrawingRef.current === drawing) stageHatch(loops, layer);
        }).catch(error => { if (sequence === asyncDraftSequence.current) setEditMessage(formatError(error, "Cannot find a closed region")); });
        return true;
      }
      const points = [...draftPoints, point];
      setDraftPoints(points);
      setEditMessage(`${points.length} boundary vertices; Enter closes the hatch`);
      return true;
    }
    const points = [...draftPoints, point];
    setDraftPoints(points);
    if (editorMode === "polyline") {
      return true;
    }
    const needed = editorMode === "arc" ? 3 : 2;
    if (points.length < needed) {
      return true;
    }
    if (editorMode === "line") {
      void commitDrawingEdit({ kind: "create", entity: { type: "line", layer, pen: null, p1: points[0], p2: points[1] } });
    } else if (editorMode === "circle") {
      void commitDrawingEdit({ kind: "create", entity: { type: "circle", layer, pen: null, center: points[0], radius: pointArrayDistance(points[0], points[1]) } });
    } else if (editorMode === "arc") {
      void commitDrawingEdit({ kind: "create", entity: { type: "arc", layer, pen: null, center: points[0], radius: pointArrayDistance(points[0], points[1]), start_deg: pointAngle(points[0], points[1]), end_deg: pointAngle(points[0], points[2]) } });
    }
    return true;
  }

  function submitCommandInput() {
    const value = commandInput.trim();
    if (value === "") {
      if (editorMode === "select" && lastSuccessfulCommand.current !== null) selectEditorMode(lastSuccessfulCommand.current);
      else applyDraftingParameters();
      return;
    }
    const coordinate = parseCoordinateInput(value, draftPoints.at(-1) ?? null);
    if (coordinate !== null && editorMode !== "select") {
      handleDraftPoint(applyOrtho(coordinate, draftPoints.at(-1) ?? null, orthoEnabled));
      setCommandInput("");
      return;
    }
    const command = commandFromText(value);
    if (command !== null) {
      if (command === "undo" || command === "redo") {
        void runHistoryAction(command === "undo");
      } else if (command === "delete") {
        void deleteSelectedEntities();
      } else if (command === "edit_block") {
        editSelectedBlock();
      } else if (command === "layout" || command === "print_preview") {
        setCadCommand(command);
        setEditorMode(command);
        setViewMode("sheet");
      } else {
        selectEditorMode(command as EditorMode);
        setViewMode("sheet");
      }
      setCommandInput("");
      return;
    }
    setEditMessage("Enter a command or x,y / @dx,dy / @distance<angle");
  }

  function handleSvgClick(event: MouseEvent) {
    if (performance.now() <= suppressClickUntil.current) {
      suppressClickUntil.current = 0;
      event.preventDefault();
      return;
    }
    if (!(event.target instanceof Element)) {
      return;
    }
    if (handleDraftClick(event)) {
      event.preventDefault();
      return;
    }
    const entityId = closestEntityId(event.target);
    if (entityId !== null && entityId !== undefined && entityId !== "") {
      selectEntity(entityId, true, event.shiftKey);
    }
  }

  return (
    <main class="app-shell">
      {blockEditing !== null && artifacts !== null && <div class="dialog-backdrop"><section class="block-edit-dialog" role="dialog" aria-modal="true" aria-label="Edit block contents">
        <h2>Edit block: {blockEditing.name}</h2>
        <p>Changes are staged until Save contents. All references to this definition will update together.</p>
        <div class="block-content-preview" dangerouslySetInnerHTML={{ __html: blockEditing.svg }} />
        <button type="button" disabled={isEditSaving} onClick={() => {
          const layer = activeLayerEditable();
          if (layer === null) return;
          const entity: EditorEntity = { schema_version: "0.3", id: newDraftEntityId(), type: "line", layer, pen: null, p1: [0, 0], p2: [100, 0] };
          stageBlockEntities([...blockEditing.entities, entity]);
          setBlockEntityIndex(blockEditing.entities.length);
          setBlockEditMessage("New line staged. Set its coordinates below, then Save contents.");
        }}>Add line</button>
        <label>Entity<select aria-label="Block entity" value={blockEntityIndex} onChange={event => setBlockEntityIndex(Number(event.currentTarget.value))}>{blockEditing.entities.map((entity, index) => <option key={entity.id} value={index}>{entity.type} — {entity.id}</option>)}</select></label>
        {blockEditing.entities[blockEntityIndex] !== undefined && <EntityPropertyEditor entity={blockEditing.entities[blockEntityIndex]} layers={artifacts.layers} pens={artifacts.editor.pens} message={blockEditMessage} onReplace={replaceBlockEntity} onTranslate={(delta, duplicate) => {
          const entity = blockEditing.entities[blockEntityIndex];
          const translated = translateEditorEntity(entity, delta);
          if (translated === null) { setBlockEditMessage("Use coordinate properties for this entity type"); return; }
          if (duplicate) stageBlockEntities([...blockEditing.entities, { ...translated, id: newDraftEntityId() }]);
          else replaceBlockEntity(translated);
        }} onDelete={() => {
          const id = blockEditing.entities[blockEntityIndex].id;
          const dimensions = blockEditing.entities.filter(entity => entity.type === "dimension" && dimensionReferences(entity.measurement, id)).map(entity => entity.id);
          if (dimensions.length > 0) { setBlockDeletion({ id, dimensions, resolutions: {} }); return; }
          stageBlockEntities(blockEditing.entities.filter((_, index) => index !== blockEntityIndex)); setBlockEntityIndex(0);
        }} />}
        {blockDeletion !== null && <section aria-label="Resolve block dimension references">
          <p>Deleting this geometry affects these dimensions. Choose before changing the draft.</p>
          {blockDeletion.dimensions.map(id => <label key={id}>{id}<select aria-label={`Block resolution for ${id}`} value={blockDeletion.resolutions[id] ?? ""} onChange={event => { const action = event.currentTarget.value; if (action === "delete" || action === "detach") setBlockDeletion(current => current === null ? null : { ...current, resolutions: { ...current.resolutions, [id]: action } }); }}><option value="">Choose…</option><option value="detach">Keep current coordinates</option><option value="delete">Delete dimension</option></select></label>)}
          <button type="button" disabled={blockDeletion.dimensions.some(id => blockDeletion.resolutions[id] === undefined)} onClick={() => void resolveBlockDeletion()}>Resolve in draft</button>
          <button type="button" onClick={() => setBlockDeletion(null)}>Keep geometry</button>
        </section>}
        <label>Independent copy name<input aria-label="Independent block name" value={draftingOptions.blockName} onInput={event => setDraftingOptions(options => ({ ...options, blockName: event.currentTarget.value }))} /></label>
        <p role="status">{blockEditMessage}</p>
        <button type="button" disabled={isEditSaving || blockDeletion !== null} onClick={() => void saveBlockOperation({ type: "update_contents", block: blockEditing.block, expected_block_revision: blockEditing.revision, entities: blockEditing.entities }, blockEditing.drawingRevision)}>Save contents</button>
        <button type="button" disabled={isEditSaving || !draftingOptions.blockName.trim()} onClick={() => void saveBlockOperation({ type: "duplicate", source_block: blockEditing.block, expected_block_revision: blockEditing.revision, block: `block_${crypto.randomUUID().replaceAll("-", "")}`, name: draftingOptions.blockName.trim(), entities: blockEditing.entities }, blockEditing.drawingRevision)}>Duplicate block</button>
        <button type="button" disabled={isEditSaving} onClick={() => { setBlockEditing(null); selectEditorMode("select"); }}>Cancel</button>
      </section></div>}
      <header class="topbar">
        <div class="brand">
          <Layers3 size={20} aria-hidden="true" />
          <span>CAD Review</span>
        </div>
        <div class="project-actions" aria-label="Project controls">
          {isDesktop && (
            <>
              <button type="button" class="tool-button" onClick={chooseProject}>
                <FolderOpen size={17} aria-hidden="true" />
                Open Project
              </button>
              <button type="button" class="tool-button" onClick={() => setProjectSetupMode("new")}>
                <FilePlus2 size={17} aria-hidden="true" />
                New Project
              </button>
              <button type="button" class="tool-button" onClick={chooseJwwImport}>
                <FileInput size={17} aria-hidden="true" />
                Import JWW
              </button>
              <button
                type="button"
                class="tool-button"
                disabled={projectState === null}
                onClick={() => {
                  setExportReport(null);
                  setExportOpen(true);
                }}
              >
                <FileOutput size={17} aria-hidden="true" />
                Export JWW
              </button>
              {projectState?.jww_edit_capability === "mapped_v600" && (
                <button type="button" class="tool-button" onClick={() => void performCompatibleJwwSave()}>
                  <FileOutput size={17} aria-hidden="true" />
                  Save JWW Compatible
                </button>
              )}
              {projectState?.jww_compatibility_state != null && (
                <button type="button" class="tool-button" onClick={() => void performOriginalJwwExtraction()}>
                  <FileOutput size={17} aria-hidden="true" />
                  Extract Original
                </button>
              )}
            </>
          )}
          <button
            type="button"
            class="tool-button"
            onClick={rerunReview}
            disabled={isDesktop && projectState === null}
          >
            <RefreshCw size={17} aria-hidden="true" />
            Re-run Review
          </button>
          {isDesktop && projectState !== null && <LiveReviewStatus state={liveReviewState} />}
          <span class="diff-source">HEAD vs working tree</span>
        </div>
        {isDesktop && artifacts !== null && (
        <fieldset class="editor-tools" aria-label="Drawing tools" disabled={isEditSaving || isHistoryBusy || pdfBusy || projectState?.editable === false}>
            <select
              class="drawing-select"
              aria-label="Current drawing"
              value={artifacts.currentDrawing}
              onChange={(event) => void changeDrawing(event.currentTarget.value)}
            >
              {artifacts.drawingNames.map((drawing) => (
                <option value={drawing} key={drawing}>{drawing}</option>
              ))}
            </select>
            <button type="button" class="icon-button" aria-label="Add drawing" title="Add drawing" onClick={() => setProjectSetupMode("add")}>
              <FilePlus2 size={16} />
            </button>
            <button type="button" class="icon-button" aria-label="Duplicate drawing" title="Duplicate drawing" onClick={() => setProjectSetupMode("duplicate")}>
              <Copy size={16} />
            </button>
            {(artifacts.layouts?.length ?? 0) > 0 && (
              <select
                class="drawing-select"
                aria-label="Active layout"
                value={artifacts.layouts?.find((layout) => layout.active)?.id ?? artifacts.layouts?.[0]?.id ?? ""}
                onChange={(event) => void commitDrawingEdit({
                  kind: "update_layout",
                  layout: "active_layout",
                  properties: event.currentTarget.value,
                })}
              >
                {artifacts.layouts?.map((layout) => (
                  <option value={layout.id} key={layout.id}>{layout.id}</option>
                ))}
              </select>
            )}
            <EditorToolButton mode="select" active={editorMode} label="Select" icon={<MousePointer2 size={16} />} onSelect={selectEditorMode} />
            <button
              type="button"
              class="icon-button"
              aria-label="Undo"
              title="Undo"
              disabled={historyState === null || historyState.context_blocked != null || historyState.undo.length === 0 || isHistoryBusy || isEditSaving}
              onClick={() => void runHistoryAction(true)}
            >
              <Undo2 size={16} />
            </button>
            <button
              type="button"
              class="icon-button"
              aria-label="Redo"
              title="Redo"
              disabled={historyState === null || historyState.context_blocked != null || historyState.redo.length === 0 || isHistoryBusy || isEditSaving}
              onClick={() => void runHistoryAction(false)}
            >
              <Redo2 size={16} />
            </button>
            <EditorToolButton mode="move" active={editorMode} label="Move" icon={<Waypoints size={16} />} onSelect={selectEditorMode} />
            {(["endpoint", "stretch", "rectangle", "fillet", "chamfer", "rectangular_array", "create_block"] as const).map(mode => <EditorToolButton key={mode} mode={mode} active={editorMode} label={commandLabel(mode).replaceAll("_", " ")} icon={<PenLine size={16} />} onSelect={selectEditorMode} />)}
            <EditorToolButton mode="copy" active={editorMode} label="Copy" icon={<Plus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="line" active={editorMode} label="Line" icon={<Minus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="polyline" active={editorMode} label="Polyline" icon={<PenLine size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="circle" active={editorMode} label="Circle" icon={<Circle size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="arc" active={editorMode} label="Arc" icon={<RotateCcw size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="text" active={editorMode} label="Text" icon={<TypeIcon size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="dimension" active={editorMode} label="Dimension" icon={<Ruler size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="point" active={editorMode} label="Point" icon={<Plus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="rotate" active={editorMode} label="Rotate" icon={<RotateCcw size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="mirror" active={editorMode} label="Mirror" icon={<Waypoints size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="offset" active={editorMode} label="Offset" icon={<Plus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="trim" active={editorMode} label="Trim" icon={<Minus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="extend" active={editorMode} label="Extend" icon={<Maximize2 size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="insert_block" active={editorMode} label="Insert Block" icon={<Plus size={16} />} onSelect={selectEditorMode} />
            <button
              type="button"
              class="icon-button"
              aria-label="Edit Block"
              title="Edit Block"
              disabled={selectedEditorEntity?.type !== "block_ref"}
              onClick={editSelectedBlock}
            >
              Edit Block
            </button>
            <EditorToolButton mode="hatch" active={editorMode} label="Hatch" icon={<Layers3 size={16} />} onSelect={selectEditorMode} />
            <button
              type="button"
              class={cadCommand === "layout" ? "icon-button is-active" : "icon-button"}
              aria-label="Layout"
              title="Layout"
              onClick={() => {
                setCadCommand("layout");
                setEditorMode("layout");
                setEditMessage("Active layout is shown in the paper frame");
              }}
            >
              Layout
            </button>
            <button
              type="button"
              class={cadCommand === "print_preview" ? "icon-button is-active" : "icon-button"}
              aria-label="Print Preview"
              title="Print Preview"
              onClick={() => {
                setCadCommand("print_preview");
                setEditorMode("print_preview");
                setViewMode("sheet");
                setEditMessage("Print preview uses the active layout");
              }}
            >
              Preview
            </button>
            <button
              type="button"
              class="icon-button"
              aria-label="Export PDF"
              title="Export PDF"
              disabled={!isDesktop || pdfBusy}
              onClick={() => void performPdfExport()}
            >
              {pdfBusy ? "Exporting…" : "PDF"}
            </button>
            <button type="button" class="icon-button" aria-label="Delete" title="Delete" onClick={() => void deleteSelectedEntities()}>
              Delete
            </button>
          </fieldset>
        )}
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
            onClick={() => zoomBy(1 / ZOOM_STEP_FACTOR)}
          >
            <ZoomOut size={18} aria-hidden="true" />
          </button>
          <output class="zoom-readout" aria-label="Zoom">
            {formatZoomScale(zoomScale)}
          </output>
          <button
            type="button"
            class="icon-button"
            aria-label="Zoom in"
            title="Zoom in"
            onClick={() => zoomBy(ZOOM_STEP_FACTOR)}
          >
            <ZoomIn size={18} aria-hidden="true" />
          </button>
          <button
            type="button"
            class="icon-button"
            aria-label="Previous view"
            title="Previous view"
            disabled={previousViewBox === null}
            onClick={goToPreviousView}
          >
            <Undo2 size={18} aria-hidden="true" />
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
          <button
            type="button"
            class="icon-button"
            aria-label="Zoom to content"
            title="Zoom to content"
            onClick={zoomToContent}
          >
            <ScanSearch size={18} aria-hidden="true" />
          </button>
          <button
            type="button"
            class={isZoomAreaActive ? "icon-button is-active" : "icon-button"}
            aria-label="Zoom area"
            title="Zoom area"
            aria-pressed={isZoomAreaActive}
            onClick={() => {
              setIsZoomAreaActive((active) => !active);
              setSelectionRect(null);
            }}
          >
            <Scan size={18} aria-hidden="true" />
          </button>
        </div>
      </header>

      <section class="review-grid">
        <aside class="side-panel" aria-label="Review results">
          <PanelHeader
            artifacts={artifacts}
            importMessage={importMessage}
            liveReviewState={liveReviewState}
            loadState={loadState}
            projectState={projectState}
          />
          {artifacts !== null && (
            <>
              {(artifacts.blocks?.length ?? 0) > 0 && (
                <section class="workspace-section" aria-label="Block definitions">
                  <h3>Blocks</h3>
                  {artifacts.blocks?.map((block) => (
                    <p class="detail-line" key={block.id}>
                      <strong>{block.name}</strong>
                      <span>{block.entity_count} entities</span>
                    </p>
                  ))}
                </section>
              )}
              {(artifacts.layouts?.length ?? 0) > 0 && (
                <section class="workspace-section" aria-label="Layouts">
                  <h3>Layouts</h3>
                  {artifacts.layouts?.map((layout) => (
                    <p class="detail-line" key={layout.id}>
                      <strong>{layout.id}{layout.active ? " (active)" : ""}</strong>
                      <span>
                        {layout.paper} {layout.orientation} {layout.scale} ·
                        margins {layout.margins.join(", ")} ·
                        plot {layout.plot_area?.join(", ") ?? "full"}
                      </span>
                    </p>
                  ))}
                </section>
              )}
              <LayerWorkspace
                state={artifacts.layers}
                canPersist={!isDesktop || (projectState !== null && !isEditSaving && !isHistoryBusy)}
                canRestore={previousLayerVisibilityRef.current !== null}
                onChange={updateLayerWorkspace}
                onRestore={restoreLayerVisibility}
              />
              <ResultPanel
                artifacts={artifacts}
                selectedEntityId={selectedEntityId}
                onSelectEntity={selectEntity}
                canEditComments={isDesktop && projectState !== null && !isEditSaving && !isHistoryBusy}
                onCreateComment={() => void createCommentForSelection()}
                onToggleCommentStatus={(comment) => void changeCommentStatus(comment)}
              />
            </>
          )}
        </aside>

        <section class="canvas-panel" aria-label="CAD paper">
          {isDesktop && artifacts !== null && <DraftingPanel command={cadCommand} options={draftingOptions} artifacts={artifacts} onChange={options => {
            setDraftingOptions(options);
            if (editorMode === "dimension" && (options.dimension !== draftingOptions.dimension || options.associate !== draftingOptions.associate)) {
              setPendingOperation(null);
              setDraftPoints([]);
              setEditPreview(null);
              setDraftParameterError(false);
              setEditMessage("Dimension settings changed. Pick the measurement points again.");
              return;
            }
            if (pendingOperation !== null) {
              const updated = retuneDraftOperation(pendingOperation, options);
              setDraftParameterError(updated === null);
              if (updated !== null) setPendingOperation(updated);
              else setEditMessage("Correct invalid parameters before applying");
            }
          }} onApply={applyDraftingParameters} onCancel={() => selectEditorMode("select")} message={editMessage} busy={isEditSaving} invalid={draftParameterError} />}
          {editorMode === "insert_block" && blockThumbnail !== "" && <div class="block-thumbnail" aria-label="Block preview" dangerouslySetInnerHTML={{ __html: blockThumbnail }} />}
          {editPreview !== null && editPreview.dimension_impacts.length > 0 && <section class="drafting-panel" aria-label="Resolve dimension references">
            <p>These dimensions would lose their references. Choose a resolution; nothing has been saved.</p>
            {editPreview.dimension_impacts.map(id => <label key={id}>{id}<select aria-label={`Resolution for ${id}`} value={dimensionResolutions[id] ?? ""} onChange={event => { const action = event.currentTarget.value; if (action === "delete" || action === "detach") setDimensionResolutions(current => ({ ...current, [id]: action })); }}><option value="">Choose…</option><option value="detach">Keep current coordinates</option><option value="delete">Delete dimension</option></select></label>)}
            <button type="button" disabled={isEditSaving || editPreview.dimension_impacts.some(id => dimensionResolutions[id] === undefined)} onClick={() => pendingOperation !== null && void commitDrawingEdit(pendingOperation)}>Apply with resolutions</button>
            <button type="button" onClick={() => selectEditorMode("select")}>Cancel</button>
          </section>}
          {loadState === "idle" && (
            <div class="empty-state">Open a CAD project folder to start desktop review.</div>
          )}
          {loadState === "loading" && <div class="empty-state">Loading generated artifacts</div>}
          {loadState === "error" && <div class="empty-state is-error">{errorMessage}</div>}
          {loadState === "ready" && activeSvg !== undefined && (
            <div
              ref={drawingStageRef}
              class={
                isPanning
                  ? "drawing-stage is-panning"
                  : isZoomAreaActive
                    ? "drawing-stage is-zoom-area"
                    : "drawing-stage"
              }
              onWheel={handleWheel}
              onPointerDown={handlePointerDown}
              onMouseDownCapture={handleChordMouseDown}
              onPointerMove={handlePointerMove}
              onPointerUp={handlePointerUp}
              onPointerCancel={handlePointerCancel}
              onDblClick={() => {
                if (editorMode === "polyline") {
                  completePolyline();
                }
              }}
              onContextMenu={(event) => {
                event.preventDefault();
                setCadCommand("select");
                setEditorMode("select");
                setDraftPoints([]);
                setPendingOperation(null);
                setSnapCandidate(null);
                setSelectionRect(null);
              }}
            >
              <div
                ref={svgSurfaceRef}
                class="svg-surface"
                onClick={handleSvgClick}
                dangerouslySetInnerHTML={{ __html: activeSvg }}
              />
              {isDesktop && viewMode === "sheet" && currentViewBox !== null && (
                <DraftOverlay
                  viewBox={currentViewBox}
                  points={draftPoints}
                  snap={snapCandidate}
                />
              )}
              {isDesktop && viewMode === "sheet" && currentViewBox !== null && selectedEditorEntity !== null && ["select", "endpoint"].includes(editorMode) && entityEditableInView(selectedEditorEntity, artifacts!) && (
                <svg class="vertex-overlay" viewBox={formatViewBox(currentViewBox)} preserveAspectRatio="xMinYMin meet" aria-label="Entity vertices">
                  {entityVertices(selectedEditorEntity).map((point, index) => <circle key={index} role="button" aria-label={`Vertex ${index + 1}`} cx={point[0]} cy={-point[1]} r={currentViewBox.width / Math.max(drawingStageRef.current?.clientWidth ?? 1000, 1) * 5} onPointerDown={event => beginEndpoint(event, selectedEditorEntity, index, point)} />)}
                </svg>
              )}
              {selectionRect !== null && (
                <div
                  class="zoom-area-rect"
                  style={{
                    left: `${selectionRect.x}px`,
                    top: `${selectionRect.y}px`,
                    width: `${selectionRect.width}px`,
                    height: `${selectionRect.height}px`,
                  }}
                />
              )}
            </div>
          )}
        </section>

        <aside class="detail-panel" aria-label="Selected entity">
          <h2>
            <MousePointer2 size={17} aria-hidden="true" />
            Selection
          </h2>
          {isDesktop && <AiContextStatusLine state={aiContextState} />}
          {selectedEntityId === "" || selectedSummary === null ? (
            <p class="muted">Select an entity on the paper or from a result list.</p>
          ) : (
            <>
              <EntityDetails entityId={selectedEntityId} summary={selectedSummary} />
              {isDesktop && artifacts !== null && selectedEditorEntity !== null && (
                <EntityPropertyEditor
                  key={`${selectedEditorEntity.id}:${artifacts.editor.revision}`}
                  entity={selectedEditorEntity}
                  layers={artifacts.layers}
                  pens={artifacts.editor.pens}
                  message={editMessage}
                  onReplace={(entity) => void commitDrawingEdit({ kind: "replace", entity_id: entity.id, entity })}
                  onTranslate={(delta, duplicate) => {
                    const entityIds = selectedEntityIds.size > 0 ? Array.from(selectedEntityIds) : [selectedEditorEntity.id];
                    void commitDrawingEdit(entityIds.length > 1
                      ? { kind: "translate_many", entity_ids: entityIds, delta, duplicate }
                      : { kind: "translate", entity_id: selectedEditorEntity.id, delta, duplicate });
                  }}
                  onDelete={() => void confirm("Delete the selected entity?", { title: "Delete entity", kind: "warning" }).then((accepted) => {
                    if (accepted) {
                      const entityIds = selectedEntityIds.size > 0 ? Array.from(selectedEntityIds) : [selectedEditorEntity.id];
                      return commitDrawingEdit(entityIds.length > 1
                        ? { kind: "delete_many", entity_ids: entityIds }
                        : { kind: "delete", entity_id: selectedEditorEntity.id });
                    }
                  })}
                />
              )}
            </>
          )}
        </aside>
      </section>
      <footer class="command-bar">
        <span class="command-prompt">{commandLabel(cadCommand)}</span>
        <input
          aria-label="Command or coordinate"
          disabled={isEditSaving || isHistoryBusy}
          value={commandInput}
          placeholder="Command or x,y · @dx,dy · @distance<angle"
          onInput={(event) => setCommandInput(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submitCommandInput();
            } else if (event.key === "Escape") {
              setCommandInput("");
            } else if (event.key === "Backspace" && commandInput === "" && draftPoints.length > 0) {
              event.preventDefault();
              setDraftPoints(points => points.slice(0, -1));
              setPendingOperation(null);
            }
          }}
        />
        <button type="button" class={snapEnabled ? "command-toggle is-active" : "command-toggle"} onClick={() => setSnapEnabled((enabled) => !enabled)}>F3 SNAP</button>
        <button type="button" class={orthoEnabled ? "command-toggle is-active" : "command-toggle"} onClick={() => setOrthoEnabled((enabled) => !enabled)}>F8 ORTHO</button>
        <span class="command-status" aria-live="polite">
          {snapCandidate !== null ? `${snapCandidate.kind} ${snapCandidate.point[0].toFixed(2)}, ${snapCandidate.point[1].toFixed(2)}` : selectedEntityIds.size > 0 ? `${selectedEntityIds.size} selected` : historyState?.context_blocked ?? historyMessage}
        </span>
      </footer>
      {exportOpen && artifacts !== null && (
        <ExportJwwDialog
          drawingNames={artifacts.drawingNames}
          busy={exportBusy}
          report={exportReport}
          onCancel={() => setExportOpen(false)}
          onExport={performJwwExport}
        />
      )}
      {projectSetupMode !== null && (
        <ProjectSetupDialog
          mode={projectSetupMode}
          busy={projectSetupBusy}
          drawingNames={artifacts?.drawingNames ?? projectState?.drawings ?? []}
          onCancel={() => setProjectSetupMode(null)}
          onChooseParent={async () => {
            const selected = await open({ directory: true, multiple: false, title: "Choose project parent folder" });
            return typeof selected === "string" ? selected : null;
          }}
          onSubmit={submitProjectSetup}
        />
      )}
    </main>
  );
}

function applyOrtho(
  point: [number, number],
  base: [number, number] | null,
  enabled: boolean,
): [number, number] {
  if (!enabled || base === null) return point;
  return Math.abs(point[0] - base[0]) >= Math.abs(point[1] - base[1])
    ? [point[0], base[1]]
    : [base[0], point[1]];
}

function entityAnchor(entity: EditorEntity | null): { x: number; y: number } {
  if (entity === null) {
    return { x: 0, y: 0 };
  }
  const record = entity as Record<string, unknown>;
  const points = Array.isArray(record.points) ? record.points : [];
  const candidate = record.center ?? record.at ?? record.p1 ?? points[0];
  if (Array.isArray(candidate) && candidate.length >= 2 &&
      typeof candidate[0] === "number" && typeof candidate[1] === "number") {
    return { x: candidate[0], y: candidate[1] };
  }
  return { x: 0, y: 0 };
}

function EditorToolButton(props: {
  mode: EditorMode;
  active: EditorMode;
  label: string;
  icon: ComponentChildren;
  onSelect: (mode: EditorMode) => void;
}) {
  return (
    <button
      type="button"
      class={props.active === props.mode ? "icon-button is-active" : "icon-button"}
      aria-label={props.label}
      title={props.label}
      aria-pressed={props.active === props.mode}
      onClick={() => props.onSelect(props.mode)}
    >
      {props.icon}
    </button>
  );
}

function DraftOverlay(props: {
  viewBox: ViewBox;
  points: Array<[number, number]>;
  snap: SnapCandidate | null;
}) {
  const points = props.points.map(([x, y]) => `${x},${-y}`).join(" ");
  const snap = props.snap?.point;
  const markerSize = props.viewBox.width / 180;
  return (
    <svg
      class="draft-overlay"
      viewBox={formatViewBox(props.viewBox)}
      preserveAspectRatio="xMinYMin meet"
      aria-hidden="true"
    >
      {props.points.length > 1 && (
        <polyline points={points} fill="none" stroke="#007c89" stroke-width={markerSize / 4} />
      )}
      {props.points.map(([x, y], index) => (
        <circle key={`${x}:${y}:${index}`} cx={x} cy={-y} r={markerSize / 3} fill="#007c89" />
      ))}
      {snap !== undefined && (
        <g transform={`translate(${snap[0]} ${-snap[1]})`}>
          <circle r={markerSize} fill="none" stroke="#d04a00" stroke-width={markerSize / 4} />
          <path d={`M ${-markerSize} 0 H ${markerSize} M 0 ${-markerSize} V ${markerSize}`} stroke="#d04a00" stroke-width={markerSize / 4} />
        </g>
      )}
    </svg>
  );
}

function asCadPoint(value: unknown): [number, number] | null {
  return Array.isArray(value) && value.length === 2 && value.every(n => typeof n === "number" && Number.isFinite(n)) ? value as [number, number] : null;
}

function newDraftEntityId(): string {
  const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let value = 0n;
  for (const byte of crypto.getRandomValues(new Uint8Array(16))) value = (value << 8n) | BigInt(byte);
  let encoded = "";
  for (let i = 0; i < 26; i++) { encoded = alphabet[Number(value & 31n)] + encoded; value >>= 5n; }
  return `ent_${encoded}`;
}

function dimensionReferences(value: unknown, entityId: string): boolean {
  if (value === null || typeof value !== "object") return false;
  if ("entity_id" in value && value.entity_id === entityId) return true;
  return Object.values(value).some(child => dimensionReferences(child, entityId));
}

function entityVertices(entity: EditorEntity): [number, number][] {
  if (entity.type === "line") return [asCadPoint(entity.p1), asCadPoint(entity.p2)].filter((p): p is [number, number] => p !== null);
  if (entity.type === "polyline" && Array.isArray(entity.points)) return entity.points.map(asCadPoint).filter((p): p is [number, number] => p !== null);
  return [];
}

function entityEditableInView(entity: EditorEntity, artifacts: Artifacts): boolean {
  const layer = artifacts.layers.layers.find(layer => layer.id === entity.layer);
  const group = artifacts.layers.groups.find(group => group.id === layer?.group);
  return layer !== undefined && layer.visible && !layer.locked && group?.visible !== false && group?.locked !== true;
}

function offsetSide(entity: EditorEntity | undefined, point: [number, number]): number {
  if (entity === undefined) return 1;
  const center = asCadPoint(entity.center);
  if (center !== null && typeof entity.radius === "number") return pointArrayDistance(center, point) >= entity.radius ? 1 : -1;
  const vertices = entityVertices(entity);
  if (entity.type === "polyline" && entity.closed === true && vertices.length > 2 && pointArrayDistance(vertices[0], vertices[vertices.length - 1]) > 1e-9) vertices.push(vertices[0]);
  let best = Infinity, side = 1;
  for (let i = 1; i < vertices.length; i++) {
    const a = vertices[i - 1], b = vertices[i];
    const dx = b[0] - a[0], dy = b[1] - a[1], length2 = dx * dx + dy * dy;
    if (length2 === 0) continue;
    const t = Math.max(0, Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length2));
    const distance = Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
    if (distance < best) { best = distance; side = dx * (point[1] - a[1]) - dy * (point[0] - a[0]) >= 0 ? 1 : -1; }
  }
  return side;
}

function dimensionAnchorAt(artifacts: Artifacts, point: [number, number]): Record<string, unknown> {
  for (const entity of artifacts.editor.entities) {
    const layer = artifacts.layers.layers.find(layer => layer.id === entity.layer);
    const group = artifacts.layers.groups.find(group => group.id === layer?.group);
    if (!layer?.visible || group?.visible === false) continue;
    for (const [index, vertex] of entityVertices(entity).entries()) {
      if (pointArrayDistance(vertex, point) < 1e-7) return { kind: "entity", entity_id: entity.id, feature: entity.type === "line" ? index === 0 ? "start" : "end" : "vertex", ...(entity.type === "polyline" ? { index } : {}) };
    }
    const center = asCadPoint(entity.center);
    if (center !== null && ["circle", "arc"].includes(entity.type)) {
      if (pointArrayDistance(center, point) < 1e-7) return { kind: "entity", entity_id: entity.id, feature: "center" };
      if (entity.type === "arc" && typeof entity.radius === "number") for (const feature of ["start", "end"] as const) {
        const angle = Number(entity[`${feature}_deg`]) * Math.PI / 180;
        const endpoint: [number, number] = [center[0] + entity.radius * Math.cos(angle), center[1] + entity.radius * Math.sin(angle)];
        if (pointArrayDistance(endpoint, point) < 1e-7) return { kind: "entity", entity_id: entity.id, feature };
      }
    }
  }
  return { kind: "fixed", point };
}

function translateEditorEntity(entity: EditorEntity, delta: [number, number]): EditorEntity | null {
  const result = structuredClone(entity);
  const move = (point: [number, number]): [number, number] => [point[0] + delta[0], point[1] + delta[1]];
  if (["line", "polyline"].includes(entity.type)) {
    if (entity.type === "line") { result.p1 = move(asCadPoint(entity.p1)!); result.p2 = move(asCadPoint(entity.p2)!); }
    else result.points = entityVertices(entity).map(move);
    return result;
  }
  for (const field of ["at", "center"]) {
    const point = asCadPoint(entity[field]);
    if (point !== null) { result[field] = move(point); return result; }
  }
  return null;
}

type PropertyPath = Array<string | number>;

function EntityPropertyEditor(props: {
  entity: EditorEntity;
  layers: LayerWorkspaceState;
  pens: string[];
  message: string;
  onReplace: (entity: EditorEntity) => void;
  onTranslate: (delta: [number, number], duplicate: boolean) => void;
  onDelete: () => void;
}) {
  const [draft, setDraft] = useState<EditorEntity>(() => structuredClone(props.entity));
  const [dx, setDx] = useState(0);
  const [dy, setDy] = useState(0);
  const layer = props.layers.layers.find((candidate) => candidate.id === props.entity.layer);
  const group = props.layers.groups.find((candidate) => candidate.id === (layer?.group ?? "default"));
  const editable = layer?.visible !== false && layer?.locked !== true && group?.visible !== false && group?.locked !== true;
  const leaves = editableLeaves(draft);

  useEffect(() => {
    setDraft(structuredClone(props.entity));
  }, [props.entity]);

  function update(path: PropertyPath, value: unknown) {
    setDraft((current) => setPathValue(current, path, value));
  }

  return (
    <section class="entity-editor" aria-label="Entity properties">
      <h3>{draft.type} properties</h3>
      <label class="property-row">
        <span>layer</span>
        <select value={draft.layer} disabled={!editable} onChange={(event) => update(["layer"], event.currentTarget.value)}>
          {props.layers.layers.map((candidate) => (
            <option value={candidate.id} key={candidate.id}>{candidate.name}</option>
          ))}
        </select>
      </label>
      <label class="property-row">
        <span>pen</span>
        <select value={draft.pen ?? ""} disabled={!editable} onChange={(event) => update(["pen"], event.currentTarget.value || null)}>
          <option value="">Layer default</option>
          {props.pens.map((pen) => <option value={pen} key={pen}>{pen}</option>)}
        </select>
      </label>
      {leaves.map((leaf) => (
        <label class="property-row" key={leaf.path.join(".")}>
          <span>{leaf.path.join(".")}</span>
          {typeof leaf.value === "boolean" ? (
            <input type="checkbox" checked={leaf.value} disabled={!editable} onChange={(event) => update(leaf.path, event.currentTarget.checked)} />
          ) : (
            <input
              type={typeof leaf.value === "number" ? "number" : "text"}
              step={typeof leaf.value === "number" ? "any" : undefined}
              value={leaf.value === null ? "" : String(leaf.value)}
              disabled={!editable}
              onInput={(event) => {
                const value = typeof leaf.value === "number"
                  ? Number(event.currentTarget.value)
                  : event.currentTarget.value;
                update(leaf.path, value);
              }}
            />
          )}
        </label>
      ))}
      <div class="entity-actions">
        <button type="button" disabled={!editable} onClick={() => props.onReplace(draft)}>Apply</button>
        <button type="button" disabled={!editable} onClick={props.onDelete}>Delete</button>
      </div>
      <div class="translate-controls">
        <label>dx<input type="number" step="any" value={dx} onInput={(event) => setDx(Number(event.currentTarget.value))} /></label>
        <label>dy<input type="number" step="any" value={dy} onInput={(event) => setDy(Number(event.currentTarget.value))} /></label>
        <button type="button" disabled={!editable} onClick={() => props.onTranslate([dx, dy], false)}>Move</button>
        <button type="button" disabled={!editable} onClick={() => props.onTranslate([dx, dy], true)}>Copy</button>
      </div>
      {props.message !== "" && <p class="editor-message">{props.message}</p>}
    </section>
  );
}

function editableLeaves(entity: EditorEntity): Array<{ path: PropertyPath; value: string | number | boolean | null }> {
  const output: Array<{ path: PropertyPath; value: string | number | boolean | null }> = [];
  const ignored = new Set(["schema_version", "id", "type", "layer", "pen"]);
  function visit(value: unknown, path: PropertyPath) {
    if (path.length === 1 && ignored.has(String(path[0]))) {
      return;
    }
    if (value === null || typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      output.push({ path, value });
      return;
    }
    if (Array.isArray(value)) {
      value.forEach((item, index) => visit(item, [...path, index]));
      return;
    }
    if (typeof value === "object") {
      Object.entries(value as Record<string, unknown>).forEach(([key, item]) => visit(item, [...path, key]));
    }
  }
  Object.entries(entity).forEach(([key, value]) => visit(value, [key]));
  return output;
}

function setPathValue(entity: EditorEntity, path: PropertyPath, value: unknown): EditorEntity {
  const next = structuredClone(entity) as Record<string, unknown>;
  let target: Record<string | number, unknown> | unknown[] = next;
  path.slice(0, -1).forEach((segment) => {
    target = (target as Record<string | number, Record<string | number, unknown> | unknown[]>)[segment];
  });
  (target as Record<string | number, unknown>)[path[path.length - 1]] = value;
  return next as EditorEntity;
}

function LayerWorkspace(props: {
  state: LayerWorkspaceState;
  canPersist: boolean;
  canRestore: boolean;
  onChange: (
    next: LayerWorkspaceState,
    patch: LayerRulesPatch,
    rememberVisibility?: boolean,
  ) => void | Promise<void>;
  onRestore: () => void | Promise<void>;
}) {
  const { state, canPersist, canRestore, onChange, onRestore } = props;
  const [query, setQuery] = useState("");
  const [usedOnly, setUsedOnly] = useState(true);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const normalizedQuery = query.trim().toLowerCase();

  function layersForGroup(groupId: string) {
    return state.layers.filter((layer) => (layer.group ?? "default") === groupId);
  }

  function toggleGroupExpanded(groupId: string) {
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(groupId)) next.delete(groupId);
      else next.add(groupId);
      return next;
    });
  }

  function setLayerVisible(layerId: string, visible: boolean) {
    const next = {
      ...state,
      layers: state.layers.map((layer) => (layer.id === layerId ? { ...layer, visible } : layer)),
    };
    void onChange(next, { expectedRevision: state.revision, layers: [{ id: layerId, visible }] });
  }

  function setLayerLocked(layerId: string, locked: boolean) {
    const next = {
      ...state,
      layers: state.layers.map((layer) => (layer.id === layerId ? { ...layer, locked } : layer)),
    };
    void onChange(next, { expectedRevision: state.revision, layers: [{ id: layerId, locked }] });
  }

  function setGroupVisible(groupId: string, visible: boolean) {
    const isSyntheticDefault = groupId === "default" && state.layers.every((layer) => !layer.group);
    const childIds = isSyntheticDefault
      ? new Set(layersForGroup(groupId).map((layer) => layer.id))
      : new Set<string>();
    const next = {
      ...state,
      groups: state.groups.map((group) => (group.id === groupId ? { ...group, visible } : group)),
      layers: state.layers.map((layer) =>
        childIds.has(layer.id) ? { ...layer, visible } : layer,
      ),
    };
    void onChange(next, {
      expectedRevision: state.revision,
      groups: isSyntheticDefault ? [] : [{ id: groupId, visible }],
      layers: Array.from(childIds, (id) => ({ id, visible })),
    });
  }

  function setGroupLocked(groupId: string, locked: boolean) {
    const isSyntheticDefault = groupId === "default" && state.layers.every((layer) => !layer.group);
    const childIds = isSyntheticDefault
      ? new Set(layersForGroup(groupId).map((layer) => layer.id))
      : new Set<string>();
    const next = {
      ...state,
      groups: state.groups.map((group) => (group.id === groupId ? { ...group, locked } : group)),
      layers: state.layers.map((layer) =>
        childIds.has(layer.id) ? { ...layer, locked } : layer,
      ),
    };
    void onChange(next, {
      expectedRevision: state.revision,
      groups: isSyntheticDefault ? [] : [{ id: groupId, locked }],
      layers: Array.from(childIds, (id) => ({ id, locked })),
    });
  }

  function isolateLayer(layerId: string) {
    const updates = state.layers.map((layer) => ({ id: layer.id, visible: layer.id === layerId }));
    const next = {
      ...state,
      groups: state.groups.map((group) => ({
        ...group,
        visible: layersForGroup(group.id).some((layer) => layer.id === layerId),
      })),
      layers: state.layers.map((layer) => ({ ...layer, visible: layer.id === layerId })),
    };
    void onChange(
      next,
      {
        expectedRevision: state.revision,
        layers: updates,
        groups: next.groups.map((group) => ({ id: group.id, visible: group.visible })),
      },
      true,
    );
  }

  return (
    <section class="layer-workspace" aria-label="Layers">
      <div class="layer-heading">
        <h2>
          <Layers3 size={17} aria-hidden="true" />
          Layers
        </h2>
        <button
          type="button"
          class="icon-button compact"
          title="Restore visibility"
          aria-label="Restore visibility"
          disabled={!canRestore || !canPersist}
          onClick={() => void onRestore()}
        >
          <RotateCcw size={15} aria-hidden="true" />
        </button>
      </div>
      <div class="layer-filters">
        <input
          type="search"
          value={query}
          placeholder="Filter layers"
          aria-label="Filter layers"
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <label>
          <input
            type="checkbox"
            checked={usedOnly}
            onChange={(event) => setUsedOnly(event.currentTarget.checked)}
          />
          Used only
        </label>
      </div>
      <div class="layer-groups">
        {state.groups.map((group) => {
          const layers = layersForGroup(group.id).filter((layer) => {
            if (usedOnly && layer.used_entity_count === 0) return false;
            if (normalizedQuery === "") return true;
            return `${layer.id} ${layer.name}`.toLowerCase().includes(normalizedQuery);
          });
          if (layers.length === 0 && (usedOnly || normalizedQuery !== "")) return null;
          const isCollapsed = collapsed.has(group.id);
          return (
            <section class="layer-group" key={group.id}>
              <div class="layer-group-row">
                <button
                  type="button"
                  class="icon-button compact"
                  aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${group.name}`}
                  onClick={() => toggleGroupExpanded(group.id)}
                >
                  {isCollapsed ? <ChevronRight size={15} /> : <ChevronDown size={15} />}
                </button>
                <button
                  type="button"
                  class="icon-button compact"
                  aria-label={`${group.visible ? "Hide" : "Show"} group ${group.name}`}
                  disabled={!canPersist}
                  onClick={() => setGroupVisible(group.id, !group.visible)}
                >
                  {group.visible ? <Eye size={15} /> : <EyeOff size={15} />}
                </button>
                <strong title={`1/${group.scale_denominator}`}>{group.name}</strong>
                <button
                  type="button"
                  class="icon-button compact layer-lock"
                  aria-label={`${group.locked ? "Unlock" : "Lock"} group ${group.name}`}
                  disabled={!canPersist}
                  onClick={() => setGroupLocked(group.id, !group.locked)}
                >
                  {group.locked ? <Lock size={14} /> : <Unlock size={14} />}
                </button>
              </div>
              {!isCollapsed && (
                <div class="layer-list">
                  {layers.map((layer) => (
                    <div class={state.active_layer === layer.id ? "layer-row is-active" : "layer-row"} key={layer.id}>
                      <button
                        type="button"
                        class="icon-button compact"
                        aria-label={`${layer.visible ? "Hide" : "Show"} ${layer.name}`}
                        disabled={!canPersist}
                        onClick={() => setLayerVisible(layer.id, !layer.visible)}
                      >
                        {layer.visible ? <Eye size={14} /> : <EyeOff size={14} />}
                      </button>
                      <button
                        type="button"
                        class="layer-name"
                        title={layer.id}
                        disabled={!canPersist}
                        onClick={() =>
                          void onChange(
                            { ...state, active_layer: layer.id },
                            { expectedRevision: state.revision, activeLayer: layer.id },
                          )
                        }
                      >
                        <span>{layer.name}</span>
                        <small>{layer.used_entity_count}</small>
                      </button>
                      <button
                        type="button"
                        class="icon-button compact"
                        aria-label={`Isolate ${layer.name}`}
                        title="Isolate layer"
                        disabled={!canPersist}
                        onClick={() => isolateLayer(layer.id)}
                      >
                        <ScanSearch size={14} />
                      </button>
                      <button
                        type="button"
                        class="icon-button compact"
                        aria-label={`${layer.locked ? "Unlock" : "Lock"} ${layer.name}`}
                        disabled={!canPersist}
                        onClick={() => setLayerLocked(layer.id, !layer.locked)}
                      >
                        {layer.locked ? <Lock size={14} /> : <Unlock size={14} />}
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </section>
          );
        })}
      </div>
    </section>
  );
}

function ProjectSetupDialog(props: {
  mode: ProjectSetupMode;
  busy: boolean;
  drawingNames: string[];
  onCancel: () => void;
  onChooseParent: () => Promise<string | null>;
  onSubmit: (values: {
    parentDir: string;
    folderName: string;
    projectName: string;
    drawing: string;
    paper: string;
    orientation: "landscape" | "portrait";
    scaleDenominator: number;
    sourceDrawing: string;
  }) => void | Promise<void>;
}) {
  const { mode, busy, drawingNames, onCancel, onChooseParent, onSubmit } = props;
  const [parentDir, setParentDir] = useState("");
  const [folderName, setFolderName] = useState("my-cad-project");
  const [projectName, setProjectName] = useState("My CAD Project");
  const [drawing, setDrawing] = useState(mode === "new" ? "plan" : "new-drawing");
  const [paper, setPaper] = useState("A3");
  const [orientation, setOrientation] = useState<"landscape" | "portrait">("landscape");
  const [scaleDenominator, setScaleDenominator] = useState(100);
  const [sourceDrawing, setSourceDrawing] = useState(drawingNames[0] ?? "");
  const title = mode === "new" ? "New Project" : mode === "add" ? "Add Drawing" : "Duplicate Drawing";
  const valid = drawing.trim() !== ""
    && (mode !== "new" || (parentDir !== "" && folderName.trim() !== "" && projectName.trim() !== ""))
    && (mode !== "duplicate" || sourceDrawing !== "");

  return (
    <div class="dialog-backdrop" role="presentation">
      <section class="export-dialog project-setup-dialog" role="dialog" aria-modal="true" aria-labelledby="project-setup-title">
        <header>
          <div>
            <p class="eyebrow">Canonical CAD source</p>
            <h2 id="project-setup-title">{title}</h2>
          </div>
          <button type="button" class="tool-button" disabled={busy} onClick={onCancel}>Close</button>
        </header>
        {mode === "new" && (
          <>
            <label class="field-label">
              Parent folder
              <span class="folder-picker-row">
                <input value={parentDir} readOnly placeholder="Choose a folder" />
                <button type="button" class="tool-button" disabled={busy} onClick={() => void onChooseParent().then((path) => path !== null && setParentDir(path))}>Choose</button>
              </span>
            </label>
            <label class="field-label">Project folder<input value={folderName} onInput={(event) => setFolderName(event.currentTarget.value)} /></label>
            <label class="field-label">Project name<input value={projectName} onInput={(event) => setProjectName(event.currentTarget.value)} /></label>
          </>
        )}
        {mode === "duplicate" && (
          <label class="field-label">Source drawing<select value={sourceDrawing} onChange={(event) => setSourceDrawing(event.currentTarget.value)}>{drawingNames.map((name) => <option value={name} key={name}>{name}</option>)}</select></label>
        )}
        <label class="field-label">New drawing name<input value={drawing} onInput={(event) => setDrawing(event.currentTarget.value)} /></label>
        {mode !== "duplicate" && (
          <div class="template-grid">
            <label class="field-label">Paper<select value={paper} onChange={(event) => setPaper(event.currentTarget.value)}><option>A3</option><option>A4</option></select></label>
            <label class="field-label">Orientation<select value={orientation} onChange={(event) => setOrientation(event.currentTarget.value as "landscape" | "portrait")}><option value="landscape">Landscape</option><option value="portrait">Portrait</option></select></label>
            <label class="field-label">Scale<select value={scaleDenominator} onChange={(event) => setScaleDenominator(Number(event.currentTarget.value))}>{[20, 50, 100, 200].map((value) => <option value={value} key={value}>1:{value}</option>)}</select></label>
          </div>
        )}
        <footer>
          <button type="button" class="tool-button primary" disabled={busy || !valid} onClick={() => void onSubmit({ parentDir, folderName, projectName, drawing, paper, orientation, scaleDenominator, sourceDrawing })}>
            <FilePlus2 size={16} aria-hidden="true" />
            {busy ? "Working" : title}
          </button>
        </footer>
      </section>
    </div>
  );
}

function ExportJwwDialog(props: {
  drawingNames: string[];
  busy: boolean;
  report: ExportReport | null;
  onCancel: () => void;
  onExport: (drawing: string, allowLossy: boolean) => void | Promise<void>;
}) {
  const { drawingNames, busy, report, onCancel, onExport } = props;
  const [drawing, setDrawing] = useState(drawingNames[0] ?? "");
  const [allowLossy, setAllowLossy] = useState(false);
  return (
    <div class="dialog-backdrop" role="presentation">
      <section class="export-dialog" role="dialog" aria-modal="true" aria-labelledby="export-title">
        <header>
          <div>
            <p class="eyebrow">Experimental</p>
            <h2 id="export-title">Export JWW</h2>
          </div>
          <button type="button" class="tool-button" disabled={busy} onClick={onCancel}>Close</button>
        </header>
        <label class="field-label">
          Drawing
          <select value={drawing} onChange={(event) => setDrawing(event.currentTarget.value)}>
            {drawingNames.map((name) => <option value={name} key={name}>{name}</option>)}
          </select>
        </label>
        <label class="checkbox-row">
          <input type="checkbox" checked={allowLossy} onChange={(event) => setAllowLossy(event.currentTarget.checked)} />
          Allow lossy export
        </label>
        {report?.status === "blocked" && (
          <div class="export-issues">
            <strong>{report.blockers.length} blocker(s)</strong>
            {report.blockers.map((issue, index) => (
              <p key={`${issue.code}-${index}`}><code>{issue.code}</code> {issue.message}</p>
            ))}
          </div>
        )}
        <footer>
          <button
            type="button"
            class="tool-button primary"
            disabled={busy || drawing === ""}
            onClick={() => void onExport(drawing, allowLossy)}
          >
            <FileOutput size={16} aria-hidden="true" />
            {busy ? "Exporting" : "Choose Destination"}
          </button>
        </footer>
      </section>
    </div>
  );
}

function PanelHeader(props: {
  artifacts: Artifacts | null;
  importMessage: string;
  liveReviewState: LiveReviewState;
  loadState: LoadState;
  projectState: ProjectState | null;
}) {
  const { artifacts, importMessage, liveReviewState, loadState, projectState } = props;
  const status = artifacts === null ? loadState : `${artifacts.check.status} / ${artifacts.diff.status}`;
  return (
    <div class="panel-header">
      <div>
        <p class="eyebrow">Generated review</p>
        <h1>{projectState?.project_name ?? "plan_1f"}</h1>
        {projectState !== null && <p class="project-path">{projectState.project_path}</p>}
        {importMessage !== "" && <p class="warning-line">{importMessage}</p>}
        {projectState?.jww_edit_capability === "mapped_v600" && (
          <p class="success-line">JWW v600 record mapping verified; compatible edits can be saved</p>
        )}
        {projectState?.jww_compatibility_state === "editable_lossless"
          && projectState.jww_edit_capability !== "mapped_v600" && (
          <p class="warning-line">JWW original verified; this import is exact-extraction only</p>
        )}
        {projectState?.jww_compatibility_state != null
          && projectState.jww_compatibility_state !== "editable_lossless" && (
          <p class="warning-line">
            JWW preserved read-only: {projectState.jww_compatibility_reason ?? "unsupported compatibility state"}
          </p>
        )}
        {liveReviewState.status === "error" && (
          <p class="warning-line">Live review: {liveReviewState.message ?? "unknown error"}</p>
        )}
      </div>
      <span class="status-chip">{status}</span>
    </div>
  );
}

function AiContextStatusLine(props: { state: AiContextState }) {
  const { state } = props;
  if (state.status === "ready") {
    return (
      <p class="success-line" title={state.markdown_path ?? undefined}>
        <CheckCircle2 size={16} aria-hidden="true" />
        AI Context: ready
      </p>
    );
  }
  if (state.status === "error") {
    return <p class="warning-line">AI Context: write failed: {state.message ?? "unknown error"}</p>;
  }
  return <p class="warning-line">AI Context: no entity selected</p>;
}

function LiveReviewStatus(props: { state: LiveReviewState }) {
  const { state } = props;
  const label = `Live: ${state.status}`;
  const icon =
    state.status === "watching" ? (
      <CheckCircle2 size={15} aria-hidden="true" />
    ) : state.status === "error" ? (
      <AlertTriangle size={15} aria-hidden="true" />
    ) : (
      <RefreshCw
        size={15}
        class={state.status === "refreshing" ? "is-spinning" : undefined}
        aria-hidden="true"
      />
    );
  return (
    <span
      class={`live-review-status is-${state.status}`}
      title={state.message}
      aria-live="polite"
    >
      {icon}
      {label}
    </span>
  );
}

function ResultPanel(props: {
  artifacts: Artifacts;
  selectedEntityId: string;
  onSelectEntity: (entityId: string, focus: boolean) => void;
  canEditComments: boolean;
  onCreateComment: () => void;
  onToggleCommentStatus: (comment: CommentRecord) => void;
}) {
  const {
    artifacts,
    selectedEntityId,
    onSelectEntity,
    canEditComments,
    onCreateComment,
    onToggleCommentStatus,
  } = props;
  return (
    <div class="result-stack">
      <section class="result-section">
        <h2>
          <FileJson2 size={17} aria-hidden="true" />
          Diff
        </h2>
        {artifacts.diffUnavailable !== undefined && (
          <p class="warning-line">{artifacts.diffUnavailable}</p>
        )}
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
              onClick={() => onSelectEntity(change.entity_id, true)}
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
                    onSelectEntity(diagnostic.entity_id, true);
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
        <h2>
          Comments
          {canEditComments && selectedEntityId !== "" && (
            <button type="button" class="icon-button" onClick={onCreateComment} aria-label="Add comment">
              <Plus size={15} aria-hidden="true" />
            </button>
          )}
        </h2>
        <div class="comment-list">
          {artifacts.comments.map((comment) => (
            <div
              class="comment-row"
              key={comment.id}
            >
              <button
                type="button"
                class="comment-select-button"
                onClick={() => {
                  const entityId = comment.entity_ids[0] ?? "";
                  if (entityId !== "") {
                    onSelectEntity(entityId, true);
                  }
                }}
              >
                <span>{comment.text}</span>
              </button>
              <small>
                {comment.status}
                {canEditComments && (
                  <button
                    type="button"
                    class="comment-status-button"
                    onClick={(event) => {
                      event.stopPropagation();
                      onToggleCommentStatus(comment);
                    }}
                  >
                    {comment.status === "resolved" ? "Reopen" : "Resolve"}
                  </button>
                )}
              </small>
            </div>
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
  const safeSheetSvg = sanitizeSvg(sheetSvg);
  return {
    sheetSvg: safeSheetSvg,
    diffSvg: sanitizeSvg(diffSvg),
    check: parseCheckReport(checkValue),
    diff: parseDiffReport(diffValue),
    comments: parseComments(commentsText),
    commentsRevision: "",
    layers: layerWorkspaceFromSvg(safeSheetSvg),
    drawingNames: ["plan_1f"],
    currentDrawing: "plan_1f",
    editor: {
      drawing: "plan_1f",
      revision: "web",
      entities: [],
      text_styles: [],
      dimension_styles: [],
      pens: [],
      fills: [],
    },
    blocks: [],
    layouts: [],
  };
}

function layerWorkspaceFromSvg(svgText: string): LayerWorkspaceState {
  const document = new DOMParser().parseFromString(svgText, "image/svg+xml");
  if (document.querySelector("parsererror") !== null) {
    return emptyLayerWorkspace;
  }
  const counts = new Map<string, number>();
  document.querySelectorAll("[data-entity-id][data-layer]").forEach((element) => {
    const layer = element.getAttribute("data-layer");
    if (layer !== null && layer !== "") {
      counts.set(layer, (counts.get(layer) ?? 0) + 1);
    }
  });
  return {
    revision: "web-artifact",
    active_layer: null,
    groups: [
      {
        id: "default",
        name: "Default",
        order: 0,
        scale_denominator: 1,
        visible: true,
        locked: false,
      },
    ],
    layers: Array.from(counts.entries())
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([id, used], order) => ({
        id,
        name: id,
        group: "default",
        order,
        visible: true,
        locked: false,
        printable: true,
        used_entity_count: used,
      })),
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function viewBoxCenter(viewBox: ViewBox): Point {
  return {
    x: viewBox.minX + viewBox.width / 2,
    y: viewBox.minY + viewBox.height / 2,
  };
}

function pointsToBBox(first: Point, second: Point): BBox {
  return {
    minX: Math.min(first.x, second.x),
    minY: Math.min(first.y, second.y),
    maxX: Math.max(first.x, second.x),
    maxY: Math.max(first.y, second.y),
  };
}

function pointDistance(first: Point, second: Point): number {
  return Math.hypot(second.x - first.x, second.y - first.y);
}

function pointArrayDistance(first: [number, number], second: [number, number]): number {
  return Math.hypot(second[0] - first[0], second[1] - first[1]);
}

function pointAngle(center: [number, number], point: [number, number]): number {
  return (Math.atan2(point[1] - center[1], point[0] - center[0]) * 180) / Math.PI;
}

function signedLineOffset(
  first: [number, number],
  second: [number, number],
  point: [number, number],
): number {
  const dx = second[0] - first[0];
  const dy = second[1] - first[1];
  const length = Math.hypot(dx, dy);
  return length === 0 ? 0 : ((point[0] - first[0]) * -dy + (point[1] - first[1]) * dx) / length;
}

function parseSvgViewBox(svgText: string | undefined): ViewBox | null {
  if (svgText === undefined) {
    return null;
  }
  const document = new DOMParser().parseFromString(svgText, "image/svg+xml");
  if (document.querySelector("parsererror") !== null) {
    return null;
  }
  return parseViewBox(document.documentElement.getAttribute("viewBox"));
}

function sameViewBox(left: ViewBox, right: ViewBox): boolean {
  return (
    left.minX === right.minX &&
    left.minY === right.minY &&
    left.width === right.width &&
    left.height === right.height
  );
}

function sanitizeSvg(svgText: string): string {
  const document = new DOMParser().parseFromString(svgText, "image/svg+xml");
  if (document.querySelector("parsererror") !== null) {
    throw new Error("SVG parse failed");
  }
  document.querySelectorAll("script, foreignObject, image, use, iframe, object, embed").forEach((element) => element.remove());
  document.querySelectorAll("*").forEach((element) => {
    Array.from(element.attributes).forEach((attribute) => {
      const attributeName = attribute.name.toLowerCase();
      const attributeValue = attribute.value.trim().toLowerCase();
      if (
        attributeName.startsWith("on")
        || attributeName === "href"
        || attributeName.endsWith(":href")
        || attributeValue.includes("javascript:")
        || attributeValue.includes("url(")
      ) {
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
    configuration_changes: Array.isArray(value.configuration_changes)
      ? value.configuration_changes.map((change) => {
        if (!isRecord(change)) throw new Error("configuration change is not an object");
        return {
          path: readString(change.path),
          kind: readString(change.kind) as "added" | "removed" | "modified",
          before: change.before,
          after: change.after,
        };
      })
      : [],
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

function importWarningMessage(projectState: ProjectState): string {
  const count = projectState.import_warning_count ?? 0;
  if (count === 0) {
    return "";
  }
  return `JWW import completed with ${count} warning(s).`;
}

const root = document.getElementById("app");

if (root === null) {
  throw new Error("Missing #app root element");
}

render(<App />, root);
