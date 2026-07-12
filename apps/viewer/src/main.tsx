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
  FileJson2,
  FileOutput,
  FolderOpen,
  Minus,
  Layers3,
  Lock,
  Maximize2,
  MousePointer2,
  PenLine,
  Plus,
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
  type EditorEntity,
  type LayerRulesPatch,
  type LayerWorkspaceState,
  type ProjectState,
  type ProjectWatchEvent,
  type SnapCandidate,
  emptyLayerWorkspace,
} from "./artifacts";
import { formatError } from "./app-errors";
import { applyDrawingEdit, queryDrawingSnap } from "./desktop-editor";
import { createDesktopComment, updateDesktopCommentStatus } from "./desktop-comments";
import { LatestAiContextWriteQueue } from "./desktop-ai-context";
import { importJwwFromDesktop } from "./desktop-import";
import { loadReviewSnapshotFromDesktop } from "./desktop-loader";
import {
  exportJwwFromDesktop,
  LatestLayerRulesQueue,
} from "./desktop-layers";
import {
  DrawingLoadGuard,
  LatestReviewQueue,
  isRelevantProjectSourcePath,
  mergeProjectStateFromReview,
  sheetSvgContainsEntity,
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
type DragMode = "pan" | "jw-gesture" | "zoom-area" | "right-wait" | "edit-move";
type EditorMode =
  | "select"
  | "move"
  | "copy"
  | "line"
  | "polyline"
  | "circle"
  | "arc"
  | "text"
  | "dimension"
  | "point";

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
  const [exportReport, setExportReport] = useState<ExportReport | null>(null);
  const [exportBusy, setExportBusy] = useState(false);
  const [liveReviewState, setLiveReviewState] = useState<LiveReviewState>({
    status: "starting",
  });
  const [aiContextState, setAiContextState] = useState<AiContextState>({
    status: "no_entity_selected",
    message: "no entity selected",
  });
  const [viewMode, setViewMode] = useState<ViewMode>("diff");
  const [selectedEntityId, setSelectedEntityId] = useState("");
  const [editorMode, setEditorMode] = useState<EditorMode>("select");
  const [draftPoints, setDraftPoints] = useState<Array<[number, number]>>([]);
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
      loadLastDesktopProject()
        .then((projectPath) => {
          if (projectPath === null) {
            setLoadState("idle");
            return;
          }
          return openDesktopProject(projectPath);
        })
        .catch((error: unknown) => {
          setErrorMessage(formatError(error, "unknown desktop load error"));
          setLoadState("error");
        });
      return;
    }
    loadWebArtifacts();
  }, [isDesktop]);

  const activeSvg = viewMode === "sheet" ? artifacts?.sheetSvg : artifacts?.diffSvg;
  const activeBaseViewBox = useMemo(() => parseSvgViewBox(activeSvg), [activeSvg]);
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
      if (event.key === "Escape") {
        setIsZoomAreaActive(false);
        setSelectionRect(null);
        dragInteraction.current = null;
        setEditorMode("select");
        setDraftPoints([]);
        setSnapCandidate(null);
      }
      if (event.key === "Enter" && editorMode === "polyline") {
        completePolyline();
      }
    }
    window.addEventListener("keydown", cancelActiveOperation);
    return () => window.removeEventListener("keydown", cancelActiveOperation);
  }, [editorMode, draftPoints, artifacts]);

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
    document
      .querySelectorAll(`.drawing-stage [data-entity-id="${selectedEntityId}"]`)
      .forEach((element) => element.classList.add("is-selected"));
  }, [activeSvg, selectedEntityId]);

  useEffect(() => {
    lastCommentAnchorRef.current = null;
  }, [artifacts?.currentDrawing, selectedEntityId]);

  useEffect(() => {
    if (artifacts !== null) {
      applyLayerWorkspaceToSvg(artifacts.layers);
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
      const targetId = event.target instanceof Element
        ? event.target.closest("[data-entity-id]")?.getAttribute("data-entity-id")
        : null;
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

  function handlePointerMove(event: PointerEvent) {
    if (editorMode !== "select" && viewMode === "sheet") {
      void updateSnapForPointer(event);
    }
    let interaction = dragInteraction.current;
    if (interaction === null) {
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
    if (interaction.mode === "zoom-area") {
      setSelectionRect(clientSelectionRect(interaction.startClient, endClient));
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
        void commitDrawingEdit({
          kind: "translate",
          entity_id: selectedEntityId,
          delta: [end[0] - start[0], end[1] - start[1]],
          duplicate: editorMode === "copy",
        });
      } else {
        setDraftPoints([]);
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

  function selectEntity(entityId: string, focus: boolean) {
    if (entityId === "") {
      return;
    }
    setSelectedEntityId(entityId);
    if (focus) {
      focusSelectedEntity(entityId);
    }
  }

  async function createCommentForSelection() {
    if (!isDesktop || projectState === null || artifacts === null || selectedEntityId === "") {
      return;
    }
    const text = window.prompt("Comment");
    if (text === null || text.trim() === "") {
      return;
    }
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
    } catch (error: unknown) {
      setEditMessage(formatError(error, "failed to create comment"));
    }
  }

  async function changeCommentStatus(comment: CommentRecord) {
    if (!isDesktop || projectState === null || artifacts === null) {
      return;
    }
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
    } catch (error: unknown) {
      setEditMessage(formatError(error, "failed to update comment status"));
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
    return invoke<string | null>("load_last_project");
  }

  async function openDesktopProject(projectPath: string, clearSelection = true) {
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
    }
    const state = await invoke<ProjectState>("open_project", { projectPath });
    await loadDesktopProjectState(state);
  }

  async function loadDesktopProjectState(state: ProjectState) {
    await invoke("save_last_project", { projectPath: state.project_path });
    const loaded = await loadReviewSnapshotFromDesktop(state.project_path, sanitizeSvg);
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
      await startProjectWatchWithCatchUp(state.project_path, (projectPath) => {
        watchActiveRef.current = true;
        watchErrorRef.current = null;
        enqueueDesktopReview(projectPath);
      });
    } catch (error: unknown) {
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
    try {
      const state = await importJwwFromDesktop(selectedFile, selectedParent);
      await loadDesktopProjectState(state);
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
    if (artifacts === null) {
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
    try {
      const result = await layerRulesQueue.enqueue(projectState.project_path, {
        ...patch,
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
    if (!event.paths.some(isRelevantProjectSourcePath)) {
      return;
    }
    watchActiveRef.current = true;
    watchErrorRef.current = null;
    enqueueDesktopReview(currentProject.project_path);
  }

  function enqueueDesktopReview(projectPath: string) {
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
        setSelectedEntityId((entityId) =>
          entityId === "" || sheetSvgContainsEntity(loaded.artifacts.sheetSvg, entityId)
            ? entityId
            : "",
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
    setEditorMode("select");
    setDraftPoints([]);
    setSnapCandidate(null);
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
    setEditorMode(mode);
    setDraftPoints([]);
    setSnapCandidate(null);
    if (mode !== "select") {
      setViewMode("sheet");
    }
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
    if (isEditSaving || !isDesktop || projectState === null || artifacts === null) {
      return;
    }
    setIsEditSaving(true);
    setEditMessage("Saving edit...");
    try {
      const result = await applyDrawingEdit(projectState.project_path, {
        drawing: artifacts.currentDrawing,
        expected_revision: artifacts.editor.revision,
        operation,
      });
      setArtifacts((current) =>
        current === null
          ? current
          : { ...current, editor: { ...current.editor, revision: result.revision } },
      );
      setSelectedEntityId(result.entity_id ?? "");
      setDraftPoints([]);
      setSnapCandidate(null);
      setEditorMode("select");
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

  function updateSnapForPointer(event: PointerEvent) {
    if (event.shiftKey || projectState === null || artifacts === null) {
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
      void queryDrawingSnap(request.projectPath, request.drawing, request.revision, request.point, tolerance)
        .then((candidate) => {
          if (sequence === snapSequenceRef.current) setSnapCandidate(candidate);
        })
        .catch(() => {
          if (sequence === snapSequenceRef.current) setSnapCandidate(null);
        });
    }, 16);
  }

  function clickedCadPoint(event: MouseEvent): [number, number] | null {
    if (snapCandidate !== null && !event.shiftKey) {
      return snapCandidate.point;
    }
    const point = clientPointToSvg({ x: event.clientX, y: event.clientY });
    return point === null ? null : [point.x, -point.y];
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
    if (point === null || artifacts === null) {
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
      void commitDrawingEdit({
        kind: "translate",
        entity_id: selectedEntityId,
        delta: [point[0] - draftPoints[0][0], point[1] - draftPoints[0][1]],
        duplicate: editorMode === "copy",
      });
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
      const value = window.prompt("Text");
      if (value !== null && value !== "") {
        const style = artifacts.editor.text_styles[0];
        if (style === undefined) {
          setEditMessage("No text style is defined");
        } else {
          void commitDrawingEdit({
            kind: "create",
            entity: { type: "text", layer, pen: null, style, at: point, rotation_deg: 0, mirror_y: false, value },
          });
        }
      }
      return true;
    }
    const points = [...draftPoints, point];
    setDraftPoints(points);
    if (editorMode === "polyline") {
      return true;
    }
    const needed = editorMode === "arc" || editorMode === "dimension" ? 3 : 2;
    if (points.length < needed) {
      return true;
    }
    if (editorMode === "line") {
      void commitDrawingEdit({ kind: "create", entity: { type: "line", layer, pen: null, p1: points[0], p2: points[1] } });
    } else if (editorMode === "circle") {
      void commitDrawingEdit({ kind: "create", entity: { type: "circle", layer, pen: null, center: points[0], radius: pointArrayDistance(points[0], points[1]) } });
    } else if (editorMode === "arc") {
      void commitDrawingEdit({ kind: "create", entity: { type: "arc", layer, pen: null, center: points[0], radius: pointArrayDistance(points[0], points[1]), start_deg: pointAngle(points[0], points[1]), end_deg: pointAngle(points[0], points[2]) } });
    } else if (editorMode === "dimension") {
      const style = artifacts.editor.dimension_styles[0];
      if (style === undefined) {
        setEditMessage("No dimension style is defined");
      } else {
        void commitDrawingEdit({ kind: "create", entity: { type: "dimension", layer, pen: null, style, p1: points[0], p2: points[1], offset: signedLineOffset(points[0], points[1], points[2]), text_rotation_deg: 0, text_mirror_y: false, value: null } });
      }
    }
    return true;
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
    const entityElement = event.target.closest("[data-entity-id]");
    const entityId = entityElement?.getAttribute("data-entity-id");
    if (entityId !== null && entityId !== undefined && entityId !== "") {
      selectEntity(entityId, true);
    }
  }

  return (
    <main class="app-shell">
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
          <fieldset class="editor-tools" aria-label="Drawing tools" disabled={isEditSaving}>
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
            <EditorToolButton mode="select" active={editorMode} label="Select" icon={<MousePointer2 size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="move" active={editorMode} label="Move" icon={<Waypoints size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="copy" active={editorMode} label="Copy" icon={<Plus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="line" active={editorMode} label="Line" icon={<Minus size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="polyline" active={editorMode} label="Polyline" icon={<PenLine size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="circle" active={editorMode} label="Circle" icon={<Circle size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="arc" active={editorMode} label="Arc" icon={<RotateCcw size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="text" active={editorMode} label="Text" icon={<TypeIcon size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="dimension" active={editorMode} label="Dimension" icon={<Ruler size={16} />} onSelect={selectEditorMode} />
            <EditorToolButton mode="point" active={editorMode} label="Point" icon={<Plus size={16} />} onSelect={selectEditorMode} />
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
              <LayerWorkspace
                state={artifacts.layers}
                canPersist={!isDesktop || projectState !== null}
                canRestore={previousLayerVisibilityRef.current !== null}
                onChange={updateLayerWorkspace}
                onRestore={restoreLayerVisibility}
              />
              <ResultPanel
                artifacts={artifacts}
                selectedEntityId={selectedEntityId}
                onSelectEntity={selectEntity}
                canEditComments={isDesktop && projectState !== null}
                onCreateComment={() => void createCommentForSelection()}
                onToggleCommentStatus={(comment) => void changeCommentStatus(comment)}
              />
            </>
          )}
        </aside>

        <section class="canvas-panel" aria-label="CAD paper">
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
              onContextMenu={(event) => event.preventDefault()}
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
                  onTranslate={(delta, duplicate) => void commitDrawingEdit({ kind: "translate", entity_id: selectedEditorEntity.id, delta, duplicate })}
                  onDelete={() => void confirm("Delete the selected entity?", { title: "Delete entity", kind: "warning" }).then((accepted) => {
                    if (accepted) {
                      return commitDrawingEdit({ kind: "delete", entity_id: selectedEditorEntity.id });
                    }
                  })}
                />
              )}
            </>
          )}
        </aside>
      </section>
      {exportOpen && artifacts !== null && (
        <ExportJwwDialog
          drawingNames={artifacts.drawingNames}
          busy={exportBusy}
          report={exportReport}
          onCancel={() => setExportOpen(false)}
          onExport={performJwwExport}
        />
      )}
    </main>
  );
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
    },
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
