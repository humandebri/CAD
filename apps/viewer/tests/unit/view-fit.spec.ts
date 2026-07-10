/**
 * apps/viewer: verifies content-fit math without a browser DOM so toolbar zoom
 * behavior stays stable as renderer viewBox sizing changes.
 */
import { expect, test } from "@playwright/test";
import {
  MAX_ZOOM_SCALE,
  ZOOM_STEP_FACTOR,
  bboxFromDataBBoxValues,
  classifyJwCadGesture,
  computeFitSvgViewBox,
  computeFitViewBox,
  formatZoomScale,
  panViewBox,
  parseDataBBox,
  parseViewBox,
  viewBoxZoomScale,
  wheelDeltaToScaleFactor,
  zoomViewBoxAtPoint,
} from "../../src/view-fit";

test("unions valid data-bbox values and ignores invalid values", () => {
  expect(bboxFromDataBBoxValues(["10,20,30,40", "5,-10,15,0", "", "bad"])).toEqual({
    minX: 5,
    minY: -10,
    maxX: 30,
    maxY: 40,
  });
});

test("returns null for empty or invalid bbox input", () => {
  expect(parseDataBBox(null)).toBeNull();
  expect(parseDataBBox("")).toBeNull();
  expect(parseDataBBox("1,2,3")).toBeNull();
  expect(parseDataBBox("1,2,three,4")).toBeNull();
  expect(bboxFromDataBBoxValues([null, "", "1,2,3"])).toBeNull();
});

test("parses positive SVG viewBox values", () => {
  expect(parseViewBox("-10 -20 300 200")).toEqual({
    minX: -10,
    minY: -20,
    width: 300,
    height: 200,
  });
  expect(parseViewBox("0 0 0 200")).toBeNull();
  expect(parseViewBox("0 0 200 -1")).toBeNull();
});

test("computes a clamped content fit viewBox with CAD y-axis inversion", () => {
  const fit = computeFitViewBox({
    stage: { width: 1000, height: 800 },
    baseViewBox: { minX: 0, minY: -800, width: 1000, height: 800 },
    content: { minX: 100, minY: 100, maxX: 300, maxY: 300 },
    maxScale: 2.5,
  });

  expect(fit).not.toBeNull();
  expect(fit?.scale).toBe(2.5);
  expect(fit?.viewBox).toEqual({
    minX: 0,
    minY: -360,
    width: 400,
    height: 320,
  });
});

test("fits single-axis geometry such as a horizontal line", () => {
  const fit = computeFitViewBox({
    stage: { width: 500, height: 300 },
    baseViewBox: { minX: 0, minY: -300, width: 500, height: 300 },
    content: { minX: 100, minY: 100, maxX: 400, maxY: 100 },
  });

  expect(fit).not.toBeNull();
  expect(fit?.scale).toBeCloseTo(1.3466666667);
  expect(fit?.viewBox.minX).toBeCloseTo(64.3564356436);
  expect(fit?.viewBox.minY).toBeCloseTo(-211.3861386139);
});

test("uses the visible stage aspect ratio for content fit", () => {
  const fit = computeFitViewBox({
    stage: { width: 650, height: 668 },
    baseViewBox: { minX: -1428, minY: -2081, width: 4625, height: 3390 },
    content: { minX: -218, minY: -99, maxX: 333, maxY: 99 },
    maxScale: 2.5,
  });

  expect(fit).not.toBeNull();
  expect(fit?.scale).toBe(2.5);
  expect(fit?.viewBox.width).toBeCloseTo(1319.4610778443);
  expect(fit?.viewBox.height).toBeCloseTo(1356);
});

test("allows content fit to use a higher explicit max scale", () => {
  const fit = computeFitViewBox({
    stage: { width: 1000, height: 800 },
    baseViewBox: { minX: 0, minY: -800, width: 1000, height: 800 },
    content: { minX: 100, minY: 100, maxX: 200, maxY: 200 },
    maxScale: 8,
  });

  expect(fit).not.toBeNull();
  expect(fit?.scale).toBeCloseTo(7.04);
});

test("fits a rectangle already expressed in SVG coordinates", () => {
  const fit = computeFitSvgViewBox({
    stage: { width: 1000, height: 800 },
    baseViewBox: { minX: 0, minY: -800, width: 1000, height: 800 },
    content: { minX: 200, minY: -500, maxX: 400, maxY: -300 },
    paddingPx: 0,
  });

  expect(fit?.viewBox).toEqual({ minX: 175, minY: -500, width: 250, height: 200 });
  expect(fit?.scale).toBe(4);
});

test("zooms beyond 800 percent and clamps at 4096x", () => {
  const baseViewBox = { minX: 0, minY: -800, width: 1000, height: 800 };
  const zoom = zoomViewBoxAtPoint({
    baseViewBox,
    currentViewBox: baseViewBox,
    targetScale: 10_000,
    focusPoint: { x: 500, y: -400 },
  });

  expect(zoom?.scale).toBe(MAX_ZOOM_SCALE);
  expect(zoom?.viewBox.width).toBeCloseTo(1000 / MAX_ZOOM_SCALE);
});

test("keeps the cursor coordinate fixed while zooming", () => {
  const baseViewBox = { minX: 0, minY: -800, width: 1000, height: 800 };
  const focusPoint = { x: 250, y: -200 };
  const zoom = zoomViewBoxAtPoint({
    baseViewBox,
    currentViewBox: baseViewBox,
    targetScale: 16,
    focusPoint,
  });

  expect(zoom).not.toBeNull();
  expect((focusPoint.x - (zoom?.viewBox.minX ?? 0)) / (zoom?.viewBox.width ?? 1)).toBeCloseTo(
    0.25,
  );
  expect((focusPoint.y - (zoom?.viewBox.minY ?? 0)) / (zoom?.viewBox.height ?? 1)).toBeCloseTo(
    0.75,
  );
});

test("uses a multiplicative toolbar zoom step", () => {
  const baseViewBox = { minX: 0, minY: -800, width: 1000, height: 800 };
  const zoom = zoomViewBoxAtPoint({
    baseViewBox,
    currentViewBox: baseViewBox,
    targetScale: ZOOM_STEP_FACTOR,
    focusPoint: { x: 500, y: -400 },
  });

  expect(zoom?.scale).toBe(ZOOM_STEP_FACTOR);
});

test("classifies Jw_cad diagonal gestures and rejects axis-only drags", () => {
  const start = { x: 100, y: 100 };
  expect(classifyJwCadGesture(start, { x: 104, y: 104 })).toBe("recenter");
  expect(classifyJwCadGesture(start, { x: 140, y: 140 })).toBe("zoom-area");
  expect(classifyJwCadGesture(start, { x: 60, y: 60 })).toBe("zoom-out");
  expect(classifyJwCadGesture(start, { x: 140, y: 60 })).toBe("zoom-all");
  expect(classifyJwCadGesture(start, { x: 60, y: 140 })).toBe("previous");
  expect(classifyJwCadGesture(start, { x: 140, y: 104 })).toBe("none");
});

test("normalizes wheel input and formats deep zoom compactly", () => {
  expect(wheelDeltaToScaleFactor(-100, 0, 800)).toBeCloseTo(Math.exp(0.2));
  expect(wheelDeltaToScaleFactor(100, 0, 800)).toBeCloseTo(Math.exp(-0.2));
  expect(formatZoomScale(8)).toBe("800%");
  expect(formatZoomScale(12.5)).toBe("12.5x");
  expect(formatZoomScale(4096)).toBe("4096x");
});

test("converts drag pixels into SVG viewBox pan", () => {
  const viewBox = panViewBox({
    currentViewBox: { minX: 0, minY: -800, width: 1000, height: 800 },
    viewport: { width: 1000, height: 800 },
    deltaPx: { x: 100, y: -50 },
  });

  expect(viewBox).toEqual({ minX: -100, minY: -750, width: 1000, height: 800 });
});

test("computes zoom scale from base and current viewBox", () => {
  expect(
    viewBoxZoomScale(
      { minX: 0, minY: -800, width: 1000, height: 800 },
      { minX: 0, minY: -360, width: 400, height: 320 },
    ),
  ).toBe(2.5);
});
