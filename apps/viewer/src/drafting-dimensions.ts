import type { Artifacts } from "./artifacts";

type Point = [number, number];

export function dimensionOffset(kind: string, first: Point, second: Point, pick: Point): number {
  if (kind === "horizontal") return pick[1] - first[1];
  if (kind === "vertical") return pick[0] - first[0];
  const dx = second[0] - first[0], dy = second[1] - first[1];
  return ((pick[0] - first[0]) * -dy + (pick[1] - first[1]) * dx) / (Math.hypot(dx, dy) || 1);
}

export function dimensionAnchorAt(artifacts: Artifacts, point: Point): Record<string, unknown> {
  for (const entity of artifacts.editor.entities) {
    const layer = artifacts.layers.layers.find(layer => layer.id === entity.layer);
    const group = artifacts.layers.groups.find(group => group.id === layer?.group);
    if (!layer?.visible || group?.visible === false) continue;
    const candidates: Array<{ point: Point; feature: string; index?: number }> = [];
    const vertices = entity.type === "line" ? [entity.p1, entity.p2] as Point[] : entity.type === "polyline" ? entity.points as Point[] : [];
    vertices.forEach((point, index) => candidates.push({ point, feature: entity.type === "line" ? index === 0 ? "start" : "end" : "vertex", ...(entity.type === "polyline" ? { index } : {}) }));
    for (let i = 0; i < vertices.length; i++) {
      const a = vertices[i], b = vertices[i + 1] ?? (entity.closed ? vertices[0] : undefined);
      if (b) candidates.push({ point: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2], feature: "midpoint", ...(entity.type === "polyline" ? { index: i } : {}) });
    }
    if (["circle", "arc", "ellipse"].includes(entity.type)) {
      const center = entity.center as Point;
      const radial = (angle: number): Point => {
        const a = angle * Math.PI / 180, rotation = Number(entity.rotation_deg ?? 0) * Math.PI / 180;
        const x = Number(entity.radius ?? entity.radius_x) * Math.cos(a), y = Number(entity.radius ?? entity.radius_y) * Math.sin(a);
        return [center[0] + x * Math.cos(rotation) - y * Math.sin(rotation), center[1] + x * Math.sin(rotation) + y * Math.cos(rotation)];
      };
      candidates.push({ point: center, feature: "center" });
      if (entity.type !== "circle") {
        candidates.push({ point: radial(Number(entity.start_deg)), feature: "start" }, { point: radial(Number(entity.end_deg)), feature: "end" }, { point: radial((Number(entity.start_deg) + Number(entity.end_deg)) / 2), feature: "midpoint" });
      }
      for (let index = 0; index < 4; index++) {
        if (entity.type !== "circle") {
          const start = Number(entity.start_deg), sweep = Number(entity.end_deg) - start;
          const delta = sweep >= 0 ? index * 90 - start : start - index * 90;
          const normalized = ((delta % 360) + 360) % 360;
          if (Math.abs(sweep) < 360 - 1e-9 && normalized > Math.abs(sweep) + 1e-9) continue;
        }
        candidates.push({ point: radial(index * 90), feature: "quadrant", index });
      }
    }
    const match = candidates.find(candidate => Math.hypot(candidate.point[0] - point[0], candidate.point[1] - point[1]) < 1e-7);
    if (match) return { kind: "entity", entity_id: entity.id, feature: match.feature, ...(match.index === undefined ? {} : { index: match.index }) };
  }
  return { kind: "fixed", point };
}
