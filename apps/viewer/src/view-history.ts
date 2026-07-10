/**
 * apps/viewer: keeps one reversible viewBox transition independent from DOM
 * events so wheel bursts and Jw_cad previous-view swaps remain testable.
 */
import { type ViewBox } from "./view-fit";

export type ViewBoxSwap = {
  current: ViewBox;
  previous: ViewBox;
};

export function beginWheelHistory(
  wheelStart: ViewBox | null,
  currentViewBox: ViewBox,
): ViewBox {
  return wheelStart ?? currentViewBox;
}

export function committedWheelPrevious(
  wheelStart: ViewBox | null,
  currentViewBox: ViewBox | null,
): ViewBox | null {
  if (wheelStart === null || currentViewBox === null || sameViewBox(wheelStart, currentViewBox)) {
    return null;
  }
  return wheelStart;
}

export function swapPreviousView(
  currentViewBox: ViewBox | null,
  previousViewBox: ViewBox | null,
): ViewBoxSwap | null {
  if (currentViewBox === null || previousViewBox === null) {
    return null;
  }
  return { current: previousViewBox, previous: currentViewBox };
}

function sameViewBox(left: ViewBox, right: ViewBox): boolean {
  return (
    left.minX === right.minX &&
    left.minY === right.minY &&
    left.width === right.width &&
    left.height === right.height
  );
}
