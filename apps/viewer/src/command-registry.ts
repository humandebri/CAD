export type CadCommand =
  | "select"
  | "line"
  | "polyline"
  | "circle"
  | "arc"
  | "text"
  | "dimension"
  | "point"
  | "move"
  | "copy"
  | "delete"
  | "trim"
  | "extend"
  | "offset"
  | "rotate"
  | "mirror"
  | "insert_block"
  | "edit_block"
  | "hatch"
  | "layout"
  | "print_preview"
  | "undo"
  | "redo";

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
  return command === "select" ? "Select" : command[0].toUpperCase() + command.slice(1);
}
