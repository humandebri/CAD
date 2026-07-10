/**
 * apps/viewer: computes paper-independent SVG viewBox updates from entity
 * metadata so zooming remains vector-rendered in the desktop webview.
 */
export type BBox = {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
};

export type RectSize = {
  width: number;
  height: number;
};

export type ViewBox = {
  minX: number;
  minY: number;
  width: number;
  height: number;
};

export type Point = {
  x: number;
  y: number;
};

export type JwCadGesture =
  | "recenter"
  | "zoom-area"
  | "zoom-out"
  | "zoom-all"
  | "previous"
  | "none";

export type FitViewBox = {
  viewBox: ViewBox;
  scale: number;
};

export const CONTENT_FIT_PADDING_PX = 48;
export const MIN_ZOOM_SCALE = 0.05;
export const MAX_ZOOM_SCALE = 4096;
export const ZOOM_STEP_FACTOR = 1.25;
export const GESTURE_THRESHOLD_PX = 8;

const MIN_MEASURABLE_SIZE = 0.000001;
const WHEEL_PIXEL_FACTOR = 0.002;
const WHEEL_LINE_HEIGHT_PX = 16;
const WHEEL_DELTA_LINE = 1;
const WHEEL_DELTA_PAGE = 2;

export function parseDataBBox(value: string | null): BBox | null {
  if (value === null) {
    return null;
  }
  const numbers = value.split(",").map((part) => Number(part.trim()));
  if (numbers.length !== 4 || numbers.some((number) => !Number.isFinite(number))) {
    return null;
  }
  const [x1, y1, x2, y2] = numbers;
  return {
    minX: Math.min(x1, x2),
    minY: Math.min(y1, y2),
    maxX: Math.max(x1, x2),
    maxY: Math.max(y1, y2),
  };
}

export function parseViewBox(value: string | null): ViewBox | null {
  if (value === null) {
    return null;
  }
  const numbers = value
    .trim()
    .split(/[\s,]+/)
    .map((part) => Number(part));
  if (numbers.length !== 4 || numbers.some((number) => !Number.isFinite(number))) {
    return null;
  }
  const [minX, minY, width, height] = numbers;
  if (width <= 0 || height <= 0) {
    return null;
  }
  return { minX, minY, width, height };
}

export function unionBBoxes(boxes: Iterable<BBox>): BBox | null {
  let union: BBox | null = null;
  for (const box of boxes) {
    if (union === null) {
      union = { ...box };
      continue;
    }
    union = {
      minX: Math.min(union.minX, box.minX),
      minY: Math.min(union.minY, box.minY),
      maxX: Math.max(union.maxX, box.maxX),
      maxY: Math.max(union.maxY, box.maxY),
    };
  }
  return union;
}

export function bboxFromElements(elements: Iterable<Element>): BBox | null {
  const values: (string | null)[] = [];
  for (const element of elements) {
    values.push(element.getAttribute("data-bbox"));
  }
  return bboxFromDataBBoxValues(values);
}

export function bboxFromDataBBoxValues(values: Iterable<string | null>): BBox | null {
  const boxes: BBox[] = [];
  for (const value of values) {
    const box = parseDataBBox(value);
    if (box !== null) {
      boxes.push(box);
    }
  }
  return unionBBoxes(boxes);
}

export function computeFitViewBox(params: {
  stage: RectSize;
  baseViewBox: ViewBox;
  content: BBox;
  paddingPx?: number;
  minScale?: number;
  maxScale?: number;
}): FitViewBox | null {
  return computeFitSvgViewBox({
    ...params,
    content: cadBBoxToSvgBBox(params.content),
  });
}

/** Fits an SVG-coordinate rectangle while preserving the visible stage ratio. */
export function computeFitSvgViewBox(params: {
  stage: RectSize;
  baseViewBox: ViewBox;
  content: BBox;
  paddingPx?: number;
  minScale?: number;
  maxScale?: number;
}): FitViewBox | null {
  const paddingPx = params.paddingPx ?? CONTENT_FIT_PADDING_PX;
  const minScale = params.minScale ?? MIN_ZOOM_SCALE;
  const maxScale = params.maxScale ?? MAX_ZOOM_SCALE;
  const stageWidth = params.stage.width;
  const stageHeight = params.stage.height;
  if (
    stageWidth <= 0 ||
    stageHeight <= 0 ||
    params.baseViewBox.width <= 0 ||
    params.baseViewBox.height <= 0 ||
    minScale <= 0 ||
    maxScale < minScale
  ) {
    return null;
  }

  const svgContent = params.content;
  const contentWidth = svgContent.maxX - svgContent.minX;
  const contentHeight = svgContent.maxY - svgContent.minY;
  const drawableWidth = stageWidth - paddingPx * 2;
  const drawableHeight = stageHeight - paddingPx * 2;
  if (drawableWidth <= 0 || drawableHeight <= 0) {
    return null;
  }

  const viewWidthForContent =
    contentWidth > MIN_MEASURABLE_SIZE ? (contentWidth * stageWidth) / drawableWidth : null;
  const viewHeightForContent =
    contentHeight > MIN_MEASURABLE_SIZE ? (contentHeight * stageHeight) / drawableHeight : null;
  if (viewWidthForContent === null && viewHeightForContent === null) {
    return null;
  }

  const stageAspect = stageWidth / stageHeight;
  let width = viewWidthForContent ?? (viewHeightForContent ?? params.baseViewBox.height) * stageAspect;
  let height = width / stageAspect;
  if (viewHeightForContent !== null && height < viewHeightForContent) {
    height = viewHeightForContent;
    width = height * stageAspect;
  }

  const contentCenterX = (svgContent.minX + svgContent.maxX) / 2;
  const contentCenterY = (svgContent.minY + svgContent.maxY) / 2;
  if (!Number.isFinite(contentCenterX) || !Number.isFinite(contentCenterY)) {
    return null;
  }

  const clampedSize = clampViewBoxSize(params.baseViewBox, { width, height }, minScale, maxScale);
  const viewBox = {
    minX: contentCenterX - clampedSize.width / 2,
    minY: contentCenterY - clampedSize.height / 2,
    width: clampedSize.width,
    height: clampedSize.height,
  };
  return { viewBox, scale: viewBoxZoomScale(params.baseViewBox, viewBox) };
}

/** Zooms around a fixed SVG coordinate so the point under the cursor does not move. */
export function zoomViewBoxAtPoint(params: {
  baseViewBox: ViewBox;
  currentViewBox: ViewBox;
  targetScale: number;
  focusPoint: Point;
  minScale?: number;
  maxScale?: number;
}): FitViewBox | null {
  const minScale = params.minScale ?? MIN_ZOOM_SCALE;
  const maxScale = params.maxScale ?? MAX_ZOOM_SCALE;
  if (params.targetScale <= 0 || minScale <= 0 || maxScale < minScale) {
    return null;
  }
  const currentScale = viewBoxZoomScale(params.baseViewBox, params.currentViewBox);
  if (!Number.isFinite(currentScale) || currentScale <= 0) {
    return null;
  }
  const targetScale = clamp(params.targetScale, minScale, maxScale);
  const resize = currentScale / targetScale;
  const width = params.currentViewBox.width * resize;
  const height = params.currentViewBox.height * resize;
  const focusOffsetX = params.focusPoint.x - params.currentViewBox.minX;
  const focusOffsetY = params.focusPoint.y - params.currentViewBox.minY;
  if (!Number.isFinite(focusOffsetX) || !Number.isFinite(focusOffsetY)) {
    return null;
  }
  const viewBox = {
    minX: params.focusPoint.x - focusOffsetX * resize,
    minY: params.focusPoint.y - focusOffsetY * resize,
    width,
    height,
  };
  return { viewBox, scale: viewBoxZoomScale(params.baseViewBox, viewBox) };
}

export function centerViewBoxAtPoint(currentViewBox: ViewBox, point: Point): ViewBox | null {
  if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) {
    return null;
  }
  return {
    ...currentViewBox,
    minX: point.x - currentViewBox.width / 2,
    minY: point.y - currentViewBox.height / 2,
  };
}

export function panViewBox(params: {
  currentViewBox: ViewBox;
  viewport: RectSize;
  deltaPx: { x: number; y: number };
}): ViewBox | null {
  const pxPerUnit = Math.min(
    params.viewport.width / params.currentViewBox.width,
    params.viewport.height / params.currentViewBox.height,
  );
  if (!Number.isFinite(pxPerUnit) || pxPerUnit <= 0) {
    return null;
  }
  return {
    ...params.currentViewBox,
    minX: params.currentViewBox.minX - params.deltaPx.x / pxPerUnit,
    minY: params.currentViewBox.minY - params.deltaPx.y / pxPerUnit,
  };
}

export function viewBoxZoomScale(baseViewBox: ViewBox, currentViewBox: ViewBox): number {
  return Math.min(baseViewBox.width / currentViewBox.width, baseViewBox.height / currentViewBox.height);
}

export function formatZoomScale(scale: number): string {
  if (!Number.isFinite(scale) || scale <= 0) {
    return "--";
  }
  if (scale < 10) {
    return `${Math.round(scale * 100)}%`;
  }
  if (scale < 100) {
    return `${scale.toFixed(1)}x`;
  }
  return `${Math.round(scale)}x`;
}

/** Converts browser wheel units into a bounded, continuous scale multiplier. */
export function wheelDeltaToScaleFactor(
  deltaY: number,
  deltaMode: number,
  pageHeightPx: number,
): number | null {
  if (!Number.isFinite(deltaY) || !Number.isFinite(pageHeightPx) || pageHeightPx <= 0) {
    return null;
  }
  let pixelDelta = deltaY;
  if (deltaMode === WHEEL_DELTA_LINE) {
    pixelDelta *= WHEEL_LINE_HEIGHT_PX;
  } else if (deltaMode === WHEEL_DELTA_PAGE) {
    pixelDelta *= pageHeightPx;
  }
  const factor = Math.exp(-pixelDelta * WHEEL_PIXEL_FACTOR);
  return Number.isFinite(factor) && factor > 0 ? clamp(factor, 0.5, 2) : null;
}

export function classifyJwCadGesture(start: Point, end: Point): JwCadGesture {
  const deltaX = end.x - start.x;
  const deltaY = end.y - start.y;
  const horizontalDrag = Math.abs(deltaX) >= GESTURE_THRESHOLD_PX;
  const verticalDrag = Math.abs(deltaY) >= GESTURE_THRESHOLD_PX;
  if (!horizontalDrag && !verticalDrag) {
    return "recenter";
  }
  if (!horizontalDrag || !verticalDrag) {
    return "none";
  }
  if (deltaX > 0 && deltaY > 0) {
    return "zoom-area";
  }
  if (deltaX < 0 && deltaY < 0) {
    return "zoom-out";
  }
  if (deltaX > 0 && deltaY < 0) {
    return "zoom-all";
  }
  return "previous";
}

export function formatViewBox(viewBox: ViewBox): string {
  return `${viewBox.minX} ${viewBox.minY} ${viewBox.width} ${viewBox.height}`;
}

function cadBBoxToSvgBBox(box: BBox): BBox {
  return {
    minX: box.minX,
    minY: -box.maxY,
    maxX: box.maxX,
    maxY: -box.minY,
  };
}

function clampViewBoxSize(
  baseViewBox: ViewBox,
  size: RectSize,
  minScale: number,
  maxScale: number,
): RectSize {
  const scale = viewBoxZoomScale(baseViewBox, {
    minX: 0,
    minY: 0,
    width: size.width,
    height: size.height,
  });
  if (!Number.isFinite(scale) || scale <= 0) {
    return size;
  }
  if (scale > maxScale) {
    const resize = scale / maxScale;
    return { width: size.width * resize, height: size.height * resize };
  }
  if (scale < minScale) {
    const resize = scale / minScale;
    return { width: size.width * resize, height: size.height * resize };
  }
  return size;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
