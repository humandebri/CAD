import type { CadCommand } from "./command-registry";
import type { Artifacts, EditOperation } from "./artifacts";

export type DraftingOptions = {
  distance: string; secondDistance: string; angle: string; width: string; height: string;
  rows: string; columns: string; rowSpacing: string; columnSpacing: string;
  text: string; style: string; pattern: string; fill: string;
  block: string; blockName: string; blockSearch: string; scale: string;
  mirrorX: boolean; mirrorY: boolean; replaceOriginals: boolean;
  detachExternalDimensions: boolean;
  dimension: string; associate: boolean; hatchRegion: boolean;
};

export const initialDraftingOptions: DraftingOptions = {
  distance: "10", secondDistance: "10", angle: "0", width: "1000", height: "1000",
  rows: "1", columns: "2", rowSpacing: "1000", columnSpacing: "1000",
  text: "", style: "", pattern: "solid", fill: "", block: "", blockName: "",
  blockSearch: "", scale: "1", mirrorX: false, mirrorY: false, replaceOriginals: true,
  detachExternalDimensions: false,
  dimension: "aligned", associate: true, hatchRegion: false,
};

export function finiteDraftNumber(value: string): number | null {
  if (value.trim() === "") return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

export function repeatableCommand(command: CadCommand): boolean {
  return !["select", "delete", "undo", "redo", "layout", "print_preview", "edit_block", "create_block"].includes(command);
}

export function retuneDraftOperation(operation: EditOperation, options: DraftingOptions): EditOperation | null {
  const distance = finiteDraftNumber(options.distance), second = finiteDraftNumber(options.secondDistance), angle = finiteDraftNumber(options.angle), scale = finiteDraftNumber(options.scale);
  switch (operation.kind) {
    case "rotate": return angle === null ? null : { ...operation, angle_deg: angle };
    case "offset": return distance === null || distance <= 0 ? null : { ...operation, distance: Math.sign(operation.distance) * distance };
    case "fillet": return distance === null || distance <= 0 ? null : { ...operation, radius: distance };
    case "chamfer": return distance === null || second === null || distance <= 0 || second <= 0 ? null : { ...operation, first_distance: distance, second_distance: second };
    case "insert_block": return angle === null || scale === null || scale <= 0 ? null : { ...operation, block: options.block || operation.block, rotation_deg: angle, scale, mirror_x: options.mirrorX, mirror_y: options.mirrorY };
    case "rectangle": {
      const width = finiteDraftNumber(options.width), height = finiteDraftNumber(options.height);
      return width === null || height === null || width <= 0 || height <= 0 ? null : { ...operation, p2: [operation.p1[0] + (Math.sign(operation.p2[0] - operation.p1[0]) || 1) * width, operation.p1[1] + (Math.sign(operation.p2[1] - operation.p1[1]) || 1) * height] };
    }
    case "rectangular_array": {
      const rows = finiteDraftNumber(options.rows), columns = finiteDraftNumber(options.columns), row = finiteDraftNumber(options.rowSpacing), column = finiteDraftNumber(options.columnSpacing);
      return rows === null || columns === null || row === null || column === null || rows < 1 || columns < 1 || !Number.isInteger(rows) || !Number.isInteger(columns) ? null : { ...operation, rows, columns, row_spacing: row, column_spacing: column };
    }
    case "batch": {
      const operations = operation.operations.map(item => retuneDraftOperation(item, options));
      return operations.some(item => item === null) ? null : { kind: "batch", operations: operations as EditOperation[] };
    }
    case "create": {
      const entity = operation.entity;
      if (entity.type === "line" && Array.isArray(entity.p1)) {
        if (distance === null || angle === null || distance <= 0) return null;
        const start = entity.p1 as [number, number], radians = angle * Math.PI / 180;
        return { ...operation, entity: { ...entity, p2: [start[0] + distance * Math.cos(radians), start[1] + distance * Math.sin(radians)] } };
      }
      if (entity.type === "text") return !options.text.trim() ? null : { ...operation, entity: { ...entity, value: options.text, style: options.style || entity.style } };
      if (entity.type === "hatch") return angle === null || distance === null || distance <= 0 ? null : { ...operation, entity: { ...entity, angle_deg: angle, scale: distance, fill: options.fill || entity.fill, pattern: options.pattern } };
      if (entity.type === "dimension") {
        const measurement = entity.measurement as Record<string, unknown> | undefined;
        if (measurement && ["aligned", "horizontal", "vertical"].includes(String(measurement.kind)) && ["aligned", "horizontal", "vertical"].includes(options.dimension)) return { ...operation, entity: { ...entity, measurement: { ...measurement, kind: options.dimension } } };
      }
      return operation;
    }
    default: return operation;
  }
}

export function DraftingPanel(props: {
  command: CadCommand; options: DraftingOptions; artifacts: Artifacts;
  onChange: (options: DraftingOptions) => void; onApply: () => void; onCancel: () => void;
  message: string; busy: boolean; invalid?: boolean;
}) {
  const { command, options, artifacts } = props;
  if (["select", "layout", "print_preview", "undo", "redo", "delete"].includes(command)) return null;
  const set = <K extends keyof DraftingOptions>(key: K, value: DraftingOptions[K]) => props.onChange({ ...options, [key]: value });
  const numeric = (key: keyof DraftingOptions, label: string) => (
    <label>{label}<input aria-label={label} type="number" step="any" value={String(options[key])} onInput={e => set(key, e.currentTarget.value)} /></label>
  );
  return <section class="drafting-panel" aria-label="Drafting parameters">
    <strong>{command.replaceAll("_", " ")}</strong>
    <div class="drafting-fields">
      {command === "rectangle" && <>{numeric("width", "Width")}{numeric("height", "Height")}</>}
      {command === "line" && <>{numeric("distance", "Length")}{numeric("angle", "Angle")}</>}
      {["offset", "fillet"].includes(command) && numeric("distance", command === "fillet" ? "Radius" : "Distance")}
      {command === "chamfer" && <>{numeric("distance", "First distance")}{numeric("secondDistance", "Second distance")}</>}
      {["rotate", "hatch", "insert_block"].includes(command) && numeric("angle", "Angle")}
      {command === "rectangular_array" && <>{numeric("rows", "Rows")}{numeric("columns", "Columns")}{numeric("rowSpacing", "Row spacing")}{numeric("columnSpacing", "Column spacing")}</>}
      {command === "text" && <>
        <label>Text<textarea aria-label="Text value" value={options.text} onInput={e => set("text", e.currentTarget.value)} /></label>
        <label>Style<select aria-label="Text style" value={options.style || artifacts.editor.text_styles[0]} onChange={e => set("style", e.currentTarget.value)}>{artifacts.editor.text_styles.map(style => <option key={style}>{style}</option>)}</select></label>
      </>}
      {command === "dimension" && <>
        <label>Dimension<select aria-label="Dimension type" value={options.dimension} onChange={e => set("dimension", e.currentTarget.value)}>{["aligned", "horizontal", "vertical", "chain", "baseline", "angle", "radius", "diameter"].map(type => <option key={type}>{type}</option>)}</select></label>
        <label><input type="checkbox" checked={options.associate} onChange={e => set("associate", e.currentTarget.checked)} />Associate with geometry</label>
      </>}
      {command === "hatch" && <>
        <label>Pattern<select aria-label="Hatch pattern" value={options.pattern} onChange={e => set("pattern", e.currentTarget.value)}>{["solid", "parallel", "cross"].map(pattern => <option key={pattern}>{pattern}</option>)}</select></label>
        <label>Fill<select aria-label="Hatch fill" value={options.fill || artifacts.editor.fills[0]} onChange={e => set("fill", e.currentTarget.value)}>{artifacts.editor.fills.map(fill => <option key={fill}>{fill}</option>)}</select></label>
        {options.pattern !== "solid" && numeric("distance", "Pitch")}
        <label><input type="checkbox" checked={options.hatchRegion} onChange={e => set("hatchRegion", e.currentTarget.checked)} />Pick enclosed region</label>
      </>}
      {command === "insert_block" && <>
        <label>Search<input aria-label="Find block" value={options.blockSearch} onInput={e => set("blockSearch", e.currentTarget.value)} /></label>
        <label>Block<select aria-label="Block" value={options.block || artifacts.blocks?.[0]?.id} onChange={e => set("block", e.currentTarget.value)}>{artifacts.blocks?.filter(b => b.name.toLowerCase().includes(options.blockSearch.toLowerCase())).map(b => <option key={b.id} value={b.id}>{b.name} ({b.entity_count})</option>)}</select></label>
        {numeric("scale", "Scale")}
        <label><input type="checkbox" checked={options.mirrorX} onChange={e => set("mirrorX", e.currentTarget.checked)} />Mirror X</label>
        <label><input type="checkbox" checked={options.mirrorY} onChange={e => set("mirrorY", e.currentTarget.checked)} />Mirror Y</label>
      </>}
      {["create_block", "edit_block"].includes(command) && <label>Block name<input aria-label="Block name" value={options.blockName} onInput={e => set("blockName", e.currentTarget.value)} /></label>}
      {command === "create_block" && <><label><input type="checkbox" checked={options.replaceOriginals} onChange={e => set("replaceOriginals", e.currentTarget.checked)} />Replace originals with reference</label><label><input type="checkbox" checked={options.detachExternalDimensions} onChange={e => set("detachExternalDimensions", e.currentTarget.checked)} />Detach dimensions that cross the block boundary</label></>}
    </div>
    <p role="status">{props.message}</p>
    <button type="button" disabled={props.busy || props.invalid} onClick={props.onApply}>Apply</button>
    <button type="button" disabled={props.busy} onClick={props.onCancel}>Cancel</button>
  </section>;
}
