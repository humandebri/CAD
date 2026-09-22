import type { EditorEntity, EditorDrawingState } from "./artifacts";

export const BLOCK_ENTITY_TYPES = ["line", "polyline", "circle", "arc", "ellipse", "text", "point", "hatch"];

export function newBlockEntity(type: string, id: string, layer: string, styles: Pick<EditorDrawingState, "text_styles" | "fills">): EditorEntity | null {
  const base = { schema_version: "0.3", id, type, layer, pen: null };
  switch (type) {
    case "line": return { ...base, p1: [0, 0], p2: [100, 0] };
    case "polyline": return { ...base, points: [[0, 0], [100, 0], [100, 100]], closed: false };
    case "circle": return { ...base, center: [0, 0], radius: 100 };
    case "arc": return { ...base, center: [0, 0], radius: 100, start_deg: 0, end_deg: 90 };
    case "ellipse": return { ...base, center: [0, 0], radius_x: 100, radius_y: 50, rotation_deg: 0, start_deg: 0, end_deg: 360 };
    case "text": return styles.text_styles[0] ? { ...base, at: [0, 0], style: styles.text_styles[0], value: "Text", rotation_deg: 0, mirror_y: false } : null;
    case "point": return { ...base, at: [0, 0] };
    case "hatch": return styles.fills[0] ? { ...base, loops: [[[0, 0], [100, 0], [100, 100], [0, 100]]], pattern: "solid", angle_deg: 0, scale: 10, fill: styles.fills[0] } : null;
    default: return null;
  }
}
