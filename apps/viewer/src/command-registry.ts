export const CAD_COMMANDS = [
  "select", "line", "polyline", "circle", "arc", "text", "dimension", "point",
  "endpoint", "stretch", "rectangle", "fillet", "chamfer", "rectangular_array", "create_block",
  "move", "copy", "delete", "trim", "extend", "offset", "rotate", "mirror",
  "insert_block", "edit_block", "hatch", "layout", "print_preview", "undo", "redo",
] as const;

export type CadCommand = typeof CAD_COMMANDS[number];

const CAD_COMMAND_SET = new Set<string>(CAD_COMMANDS);

export const CAD_COMMAND_SHORTCUTS: Record<string, CadCommand> = {
  escape: "select",
  l: "line",
  p: "polyline",
  c: "circle",
  a: "arc",
  t: "text",
  d: "dimension",
  point: "point",
  m: "move",
  o: "offset",
  r: "rotate",
  x: "trim",
  e: "extend",
  y: "mirror",
  b: "insert_block",
  h: "hatch",
  q: "layout",
  v: "print_preview",
  delete: "delete",
  z: "undo",
};

export function commandForKeyboardEvent(event: KeyboardEvent): CadCommand | null {
  if (event.key === "Escape") return "select";
  if (event.key === "Enter") return null;
  const key = event.key.toLowerCase();
  if (key === "z" && (event.ctrlKey || event.metaKey || event.shiftKey)) {
    return event.shiftKey ? "redo" : "undo";
  }
  return CAD_COMMAND_SHORTCUTS[key] ?? null;
}

export function commandLabel(command: CadCommand): string {
  return command[0].toUpperCase() + command.slice(1);
}

export function commandFromText(value: string): CadCommand | null {
  const normalized = value.trim().toLowerCase().replaceAll(" ", "_");
  if (normalized === "") return null;
  return CAD_COMMAND_SHORTCUTS[normalized]
    ?? (CAD_COMMAND_SET.has(normalized)
      ? normalized as CadCommand
      : null);
}

export function parseCoordinateInput(
  value: string,
  base: [number, number] | null,
): [number, number] | null {
  const text = value.trim();
  const polar = /^@([^<]+)<(.+)$/.exec(text);
  if (polar !== null && base !== null) {
    const distance = Number(polar[1]);
    const angle = Number(polar[2]);
    if (Number.isFinite(distance) && distance >= 0 && Number.isFinite(angle)) {
      const radians = angle * Math.PI / 180;
      return [base[0] + distance * Math.cos(radians), base[1] + distance * Math.sin(radians)];
    }
    return null;
  }
  const relative = text.startsWith("@");
  const coordinates = (relative ? text.slice(1) : text).split(",").map(Number);
  if (coordinates.length !== 2 || coordinates.some((coordinate) => !Number.isFinite(coordinate))) {
    return null;
  }
  if (relative) {
    return base === null ? null : [base[0] + coordinates[0], base[1] + coordinates[1]];
  }
  return [coordinates[0], coordinates[1]];
}
