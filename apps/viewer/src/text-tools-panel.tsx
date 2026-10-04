import { useEffect, useRef, useState } from "preact/hooks";
import type { EditorEntity, EditOperation } from "./artifacts";

export type TextEditRequest =
  | { kind: "replace"; find: string; replace: string; entity_ids: string[]; style: string | null }
  | { kind: "set_style"; entity_ids: string[]; style: string }
  | { kind: "set_writing_mode"; entity_ids: string[]; writing_mode: "horizontal" | "vertical_upright" };

export function textMatches(entities: EditorEntity[], selected: string[], find: string, selectionOnly: boolean) {
  const ids = new Set(selected);
  return entities.filter(entity => entity.type === "text" && typeof entity.value === "string"
    && (!selectionOnly || ids.has(entity.id)) && entity.value.includes(find));
}

export function TextToolsPanel(props: {
  entities: EditorEntity[]; styles: string[]; selected: string[]; disabled: boolean;
  previewReady: boolean; sourceRevision: string;
  onGenerate: (request: TextEditRequest) => Promise<{ operation: EditOperation; warnings: string[] }>;
  onPreview: (operation: EditOperation) => void; onApply: () => void; onCancel: () => void;
  onSelect: (id: string) => void;
}) {
  const [find, setFind] = useState("");
  const [replacement, setReplacement] = useState("");
  const [style, setStyle] = useState("");
  const [writingMode, setWritingMode] = useState<"horizontal" | "vertical_upright">("vertical_upright");
  const [selectionOnly, setSelectionOnly] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const generation = useRef(0);
  const cancelRef = useRef(props.onCancel); cancelRef.current = props.onCancel;
  const readyRef = useRef(props.previewReady); readyRef.current = props.previewReady;
  const selection = selectionOnly ? props.selected.slice().sort().join(",") : "";
  useEffect(() => {
    ++generation.current;
    if (readyRef.current) cancelRef.current();
    setMessage("");
  }, [selection, props.sourceRevision]);
  useEffect(() => () => { ++generation.current; }, []);
  const matches = textMatches(props.entities, props.selected, find, selectionOnly);
  const changes = (kind: "replace" | "set_style" | "set_writing_mode") => matches.filter(entity => kind === "set_writing_mode"
    ? (entity.writing_mode || "horizontal") !== writingMode :
    (kind === "replace" && find !== "" && (entity.value as string).replaceAll(find, replacement) !== entity.value)
      || (style !== "" && entity.style !== style));
  const invalidate = () => {
    ++generation.current;
    if (props.previewReady) props.onCancel();
    setMessage("");
  };
  const preview = async (kind: "replace" | "set_style" | "set_writing_mode") => {
    if (props.disabled || busy) return;
    invalidate();
    const sequence = ++generation.current;
    const candidates = changes(kind);
    if (candidates.length === 0 || (kind === "replace" && !find) || (kind === "set_style" && !style)) return;
    setBusy(true);
    try {
      const entity_ids = candidates.map(entity => entity.id);
      const request: TextEditRequest = kind === "replace"
        ? { kind, find, replace: replacement, entity_ids, style: style || null }
        : kind === "set_style" ? { kind, entity_ids, style } : {kind, entity_ids, writing_mode:writingMode};
      const generated = await props.onGenerate(request);
      if (sequence !== generation.current) return;
      props.onPreview(generated.operation);
      setMessage([...generated.warnings, `${candidates.length} text entities in the preview. Review the drawing before applying.`].join(" "));
    } catch (error) {
      if (sequence === generation.current) setMessage(error instanceof Error ? error.message : String(error));
    } finally { setBusy(false); }
  };
  const disabled = props.disabled || busy;
  return <details class="text-tools-panel">
    <summary>Text search and batch edits</summary>
    <p>Literal, case-sensitive search of annotation text in this drawing. Dimension labels and block contents are excluded.</p>
    <fieldset disabled={disabled}><legend>Text edit scope</legend><div class="drafting-fields">
      <label>Find text<input aria-label="Find annotation text" value={find} onInput={e => {const value=e.currentTarget.value; invalidate(); setFind(value);}} /></label>
      <label>Replace with<textarea aria-label="Replacement text" value={replacement} onInput={e => {const value=e.currentTarget.value; invalidate(); setReplacement(value);}} /></label>
      <label>Scope<select aria-label="Text search scope" value={selectionOnly ? "selection" : "drawing"} onChange={e => {const value=e.currentTarget.value; invalidate(); setSelectionOnly(value === "selection");}}><option value="drawing">Entire drawing</option><option value="selection">Selected entities</option></select></label>
      <label>Target style<select aria-label="Batch text style" value={style} onChange={e => {const value=e.currentTarget.value; invalidate(); setStyle(value);}}><option value="">Keep current style</option>{props.styles.map(s => <option key={s}>{s}</option>)}</select></label>
      <label>Writing direction<select aria-label="Batch text writing direction" value={writingMode} onChange={e => {const value=e.currentTarget.value as typeof writingMode; invalidate(); setWritingMode(value);}}><option value="horizontal">Horizontal</option><option value="vertical_upright">Upright vertical columns</option></select></label>
    </div>
    <button type="button" disabled={!find || changes("replace").length === 0} onClick={() => void preview("replace")}>Preview text replacement</button>
    <button type="button" disabled={!style || changes("set_style").length === 0} onClick={() => void preview("set_style")}>Preview style change</button>
    <button type="button" disabled={changes("set_writing_mode").length === 0} onClick={() => void preview("set_writing_mode")}>Preview writing direction</button>
    </fieldset>
    <p>Upright columns use one Unicode scalar per cell and start new columns to the left at LF. Vertical punctuation, combining-character shaping, ruby and tate-chu-yoko are not applied.</p>
    <p role="status">{message || `${matches.length} matching text entities.`}</p>
    <button type="button" disabled={disabled || !props.previewReady} onClick={props.onApply}>Apply text preview</button>
    <button type="button" disabled={disabled || !props.previewReady} onClick={() => {invalidate(); setMessage("Text preview cancelled.");}}>Cancel text preview</button>
    <ul aria-label="Matching annotation texts">{matches.slice(0, 100).map(entity => <li key={entity.id}>
      <button type="button" disabled={disabled} onClick={() => props.onSelect(entity.id)}>Show {entity.id}</button>
      <p class="text-match-value">{entity.value as string}</p><small>{entity.layer} · {String(entity.style)}</small>
      {find && <p class="text-match-value">Replacement: {(entity.value as string).replaceAll(find, replacement)}</p>}
      {style && <small>Target style: {style}</small>}
    </li>)}</ul>
    {matches.length > 100 && <p>Showing the first 100 of {matches.length} matches. Preview includes every changed match in the chosen scope.</p>}
  </details>;
}
