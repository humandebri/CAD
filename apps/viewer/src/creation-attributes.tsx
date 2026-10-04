import type { EditorDrawingState, EditorEntity, LayerWorkspaceState } from "./artifacts";

export type CreationAttributes = {
  sourceId: string; layer: string; pen: string | null;
  textStyle: string | null; dimensionStyle: string | null; fill: string | null;
};

export function acquireCreationAttributes(entity: EditorEntity, editor: EditorDrawingState, layers: LayerWorkspaceState): CreationAttributes {
  if (!layers.layers.some(layer => layer.id === entity.layer)) throw new Error("Source layer is unavailable.");
  const reference = (value: unknown, choices: string[], name: string): string | null => {
    if (value == null) return null;
    if (typeof value !== "string" || !choices.includes(value)) throw new Error(`Source ${name} is unavailable.`);
    return value;
  };
  return {
    sourceId: entity.id, layer: entity.layer, pen: reference(entity.pen, editor.pens, "pen"),
    textStyle: entity.type === "text" ? reference(entity.style, editor.text_styles, "text style") : null,
    dimensionStyle: entity.type === "dimension" ? reference(entity.style, editor.dimension_styles, "dimension style") : null,
    fill: ["solid", "curve_solid", "hatch"].includes(entity.type) ? reference(entity.fill, editor.fills, "fill") : null,
  };
}

export function CreationAttributesPanel(props: {
  source: EditorEntity | null; acquired: CreationAttributes | null; disabled: boolean;
  onAcquire: () => void; onClear: () => void;
}) {
  return <section class="creation-attributes-panel" aria-label="Next drawing attributes">
    <h3>Next drawing attributes</h3>
    <p>Pick layer, pen and applicable text/dimension style or fill from the selected entity. Geometry is not copied. Picking or clearing attributes cancels the current draft.</p>
    <button type="button" disabled={props.disabled || props.source === null} onClick={props.onAcquire}>Use selected attributes</button>
    <button type="button" disabled={props.disabled || props.acquired === null} onClick={props.onClear}>Clear acquired attributes</button>
    <p role="status">{props.acquired === null ? "Active layer and drafting parameters are used." : `Acquired from ${props.acquired.sourceId}: layer ${props.acquired.layer}, pen ${props.acquired.pen ?? "layer pen"}${props.acquired.textStyle ? `, text style ${props.acquired.textStyle}` : ""}${props.acquired.dimensionStyle ? `, dimension style ${props.acquired.dimensionStyle}` : ""}${props.acquired.fill ? `, fill ${props.acquired.fill}` : ""}. The acquired layer overrides the active layer until cleared.`}</p>
  </section>;
}
