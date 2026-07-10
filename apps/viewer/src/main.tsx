/**
 * apps/viewer: generated CAD review artifacts are loaded from fixed build paths.
 * The viewer keeps SVG interactive by inlining it and reading data-entity-id.
 */
import { render } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  CheckCircle2,
  FileInput,
  FileJson2,
  FolderOpen,
  Layers3,
  Maximize2,
  MousePointer2,
  RefreshCw,
  Scan,
  ScanSearch,
  Undo2,
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
  type ProjectState,
} from "./artifacts";
import { formatError } from "./app-errors";
import { LatestAiContextWriteQueue } from "./desktop-ai-context";
import { importJwwFromDesktop } from "./desktop-import";
import { loadArtifactsFromDesktop } from "./desktop-loader";
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
type DragMode = "pan" | "jw-gesture" | "zoom-area" | "right-wait";

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
  const [aiContextState, setAiContextState] = useState<AiContextState>({
    status: "no_entity_selected",
    message: "no entity selected",
  });
  const [viewMode, setViewMode] = useState<ViewMode>("diff");
  const [selectedEntityId, setSelectedEntityId] = useState("");
  const [baseViewBox, setBaseViewBox] = useState<ViewBox | null>(null);
  const [currentViewBox, setCurrentViewBox] = useState<ViewBox | null>(null);
  const [previousViewBox, setPreviousViewBox] = useState<ViewBox | null>(null);
  const [isPanning, setIsPanning] = useState(false);
  const [isZoomAreaActive, setIsZoomAreaActive] = useState(false);
  const [selectionRect, setSelectionRect] = useState<SelectionRect | null>(null);
  const drawingStageRef = useRef<HTMLDivElement>(null);
  const svgSurfaceRef = useRef<HTMLDivElement>(null);
  const currentViewBoxRef = useRef<ViewBox | null>(null);
  const dragInteraction = useRef<DragInteraction | null>(null);
  const suppressClickUntil = useRef(0);
  const wheelHistoryStart = useRef<ViewBox | null>(null);
  const wheelHistoryTimer = useRef<number | null>(null);
  const aiContextWriteQueue = useMemo(() => new LatestAiContextWriteQueue(), []);
  const isDesktop = isTauriRuntime();

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

  useEffect(() => {
    setBaseViewBox(activeBaseViewBox);
    setCurrentViewBox(activeBaseViewBox);
    currentViewBoxRef.current = activeBaseViewBox;
    setPreviousViewBox(null);
    setIsPanning(false);
    setIsZoomAreaActive(false);
    setSelectionRect(null);
    dragInteraction.current = null;
    clearPendingWheelHistory();
  }, [activeBaseViewBox]);

  useEffect(() => {
    function cancelZoomArea(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setIsZoomAreaActive(false);
        setSelectionRect(null);
        dragInteraction.current = null;
      }
    }
    window.addEventListener("keydown", cancelZoomArea);
    return () => window.removeEventListener("keydown", cancelZoomArea);
  }, []);

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
    if (!isDesktop || projectState === null || loadState !== "ready") {
      return;
    }
    if (selectedEntityId === "") {
      setAiContextState({
        status: "no_entity_selected",
        message: "no entity selected",
      });
      return;
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
    const content = bboxFromElements(svg.querySelectorAll("[data-entity-id][data-bbox]"));
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
        matchedElements.push(element);
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

  function loadWebArtifacts() {
    setLoadState("loading");
    loadArtifacts()
      .then((loaded) => {
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
    if (clearSelection) {
      setSelectedEntityId("");
    }
    const state = await invoke<ProjectState>("open_project", { projectPath });
    await loadDesktopProjectState(state);
  }

  async function loadDesktopProjectState(state: ProjectState) {
    await invoke("save_last_project", { projectPath: state.project_path });
    const loaded = await loadArtifactsFromDesktop(state.project_path, sanitizeSvg);
    setProjectState(state);
    setImportMessage(importWarningMessage(state));
    setArtifacts(loaded);
    setLoadState("ready");
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
    setSelectedEntityId("");
    try {
      const state = await importJwwFromDesktop(selectedFile, selectedParent);
      await loadDesktopProjectState(state);
    } catch (error: unknown) {
      setErrorMessage(formatError(error, "unknown JWW import error"));
      setLoadState("error");
    }
  }

  function rerunReview() {
    if (isDesktop && projectState !== null) {
      openDesktopProject(projectState.project_path, false).catch((error: unknown) => {
        setSelectedEntityId("");
        setErrorMessage(formatError(error, "unknown review error"));
        setLoadState("error");
      });
      return;
    }
    loadWebArtifacts();
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
          <span class="diff-source">HEAD vs working tree</span>
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
            loadState={loadState}
            projectState={projectState}
          />
          {artifacts !== null && (
            <ResultPanel
              artifacts={artifacts}
              selectedEntityId={selectedEntityId}
              onSelectEntity={selectEntity}
            />
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
              onContextMenu={(event) => event.preventDefault()}
            >
              <div
                ref={svgSurfaceRef}
                class="svg-surface"
                onClick={handleSvgClick}
                dangerouslySetInnerHTML={{ __html: activeSvg }}
              />
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
            <EntityDetails entityId={selectedEntityId} summary={selectedSummary} />
          )}
        </aside>
      </section>
    </main>
  );
}

function PanelHeader(props: {
  artifacts: Artifacts | null;
  importMessage: string;
  loadState: LoadState;
  projectState: ProjectState | null;
}) {
  const { artifacts, importMessage, loadState, projectState } = props;
  const status = artifacts === null ? loadState : `${artifacts.check.status} / ${artifacts.diff.status}`;
  return (
    <div class="panel-header">
      <div>
        <p class="eyebrow">Generated review</p>
        <h1>{projectState?.project_name ?? "plan_1f"}</h1>
        {projectState !== null && <p class="project-path">{projectState.project_path}</p>}
        {importMessage !== "" && <p class="warning-line">{importMessage}</p>}
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

function ResultPanel(props: {
  artifacts: Artifacts;
  selectedEntityId: string;
  onSelectEntity: (entityId: string, focus: boolean) => void;
}) {
  const { artifacts, selectedEntityId, onSelectEntity } = props;
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
        <h2>Comments</h2>
        <div class="comment-list">
          {artifacts.comments.map((comment) => (
            <button
              type="button"
              class="comment-row"
              key={comment.id}
              onClick={() => {
                const entityId = comment.entity_ids[0] ?? "";
                if (entityId !== "") {
                  onSelectEntity(entityId, true);
                }
              }}
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
