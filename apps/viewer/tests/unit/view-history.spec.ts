/**
 * apps/viewer: verifies one-step view history without relying on browser event
 * timing, including grouped wheel input and Jw_cad previous-view swaps.
 */
import { expect, test } from "@playwright/test";
import {
  beginWheelHistory,
  committedWheelPrevious,
  swapPreviousView,
} from "../../src/view-history";

const baseViewBox = { minX: 0, minY: -800, width: 1000, height: 800 };
const zoomedViewBox = { minX: 250, minY: -600, width: 500, height: 400 };

test("keeps the first viewBox across a wheel burst", () => {
  const started = beginWheelHistory(null, baseViewBox);
  const unchangedStart = beginWheelHistory(started, zoomedViewBox);

  expect(unchangedStart).toEqual(baseViewBox);
  expect(committedWheelPrevious(unchangedStart, zoomedViewBox)).toEqual(baseViewBox);
});

test("does not create wheel history when the view did not change", () => {
  expect(committedWheelPrevious(baseViewBox, baseViewBox)).toBeNull();
});

test("swaps current and previous viewBoxes", () => {
  expect(swapPreviousView(zoomedViewBox, baseViewBox)).toEqual({
    current: baseViewBox,
    previous: zoomedViewBox,
  });
  expect(swapPreviousView(zoomedViewBox, null)).toBeNull();
});
