import { test, expect } from "@playwright/test";
import { dimensionOffset, dimensionAnchorAt } from "../../src/drafting-dimensions";
import type { Artifacts } from "../../src/artifacts";

test("axis dimensions pass through the picked position regardless of endpoint order", () => {
  expect(dimensionOffset("vertical", [0, 0], [0, 100], [30, 50])).toBe(30);
  expect(dimensionOffset("vertical", [0, 100], [0, 0], [-30, 50])).toBe(-30);
  expect(dimensionOffset("horizontal", [0, 0], [100, 100], [50, 150])).toBe(150);
  expect(dimensionOffset("aligned", [0, 0], [100, 0], [50, -30])).toBe(-30);
});

test("midpoints and quadrants carry references; hidden geometry and arbitrary points do not", () => {
  const artifacts = { layers: { layers: [{ id: "a", visible: true }], groups: [] }, editor: { entities: [
    { id: "line", type: "line", layer: "a", p1: [0, 0], p2: [100, 0] },
    { id: "poly", type: "polyline", layer: "a", points: [[200, 0], [300, 0], [300, 100]], closed: true },
    { id: "circle", type: "circle", layer: "a", center: [0, 200], radius: 50 },
  ] } } as unknown as Artifacts;
  expect(dimensionAnchorAt(artifacts, [50, 0])).toMatchObject({ entity_id: "line", feature: "midpoint" });
  expect(dimensionAnchorAt(artifacts, [250, 50])).toMatchObject({ entity_id: "poly", feature: "midpoint", index: 2 });
  expect(dimensionAnchorAt(artifacts, [0, 250])).toMatchObject({ entity_id: "circle", feature: "quadrant", index: 1 });
  expect(dimensionAnchorAt(artifacts, [25, 0])).toEqual({ kind: "fixed", point: [25, 0] });
  artifacts.layers.layers[0].visible = false;
  expect(dimensionAnchorAt(artifacts, [50, 0])).toEqual({ kind: "fixed", point: [50, 0] });
});

for (const type of ["arc", "ellipse"]) {
  test(`${type} quadrants respect signed, wrapped and full sweeps`, () => {
    const curve = { id: "curve", type, layer: "a", center: [0, 0], ...(type === "arc" ? { radius: 100 } : { radius_x: 100, radius_y: 50 }), rotation_deg: 0, start_deg: 0, end_deg: 90 };
    const artifacts = { layers: { layers: [{ id: "a", visible: true }], groups: [] }, editor: { entities: [
      curve, { id: "line", type: "line", layer: "a", p1: [-100, 0], p2: [-200, 0] },
    ] } } as unknown as Artifacts;
    expect(dimensionAnchorAt(artifacts, [-100, 0])).toMatchObject({ entity_id: "line", feature: "start" });
    const bottom: [number, number] = [0, type === "arc" ? -100 : -50];
    expect(dimensionAnchorAt(artifacts, bottom)).toEqual({ kind: "fixed", point: bottom });
    for (const [start, end] of [[20, -100], [260, 380], [30, 390], [30, -330]]) {
      curve.start_deg = start; curve.end_deg = end;
      expect(dimensionAnchorAt(artifacts, bottom)).toMatchObject({ entity_id: "curve", feature: "quadrant", index: 3 });
    }
    curve.rotation_deg = 90;
    curve.start_deg = 0; curve.end_deg = 90;
    if (type === "ellipse") expect(dimensionAnchorAt(artifacts, [50, 0])).toEqual({ kind: "fixed", point: [50, 0] });
  });
}
