import { useState } from "preact/hooks";
import { sanitizeSvg } from "./svg-sanitizer";

export type StagePreview = {
  svg: string;
  report: {
    plan_hash: string; expanded_ids: string[]; changed_files: string[];
    cad_check: { status: string; diagnostics: Array<{ message: string }> };
    diff: { changes: Array<{ drawing: string; entity_id: string; kind: string }>; configuration_changes: unknown[] };
  };
};
export function GitStagePanel(props: {
  selected: string[]; disabled: boolean;
  onPreview: () => Promise<StagePreview>; onApply: (hash: string) => Promise<void>;
}) {
  const [preview,setPreview]=useState<StagePreview|null>(null);
  const [busy,setBusy]=useState(false);
  const [message,setMessage]=useState("");
  const disabled=busy || props.disabled;
  return <details class="git-stage-panel selective-stage-panel"><summary>Stage selected CAD entities</summary>
    <p>Review selected entities and their dimension, layer, style, and block dependencies. Shared definitions can affect other drawings. Existing staged changes outside this set are preserved.</p>
    {preview===null && <button type="button" disabled={disabled || props.selected.length===0} onClick={() => {
      setBusy(true); setPreview(null);
      void props.onPreview().then(value => {setPreview(value);setMessage("Review the candidate before staging.");}).catch(error=>setMessage(String(error))).finally(()=>setBusy(false));
    }}>Preview staging</button>}
    {preview!==null && <section aria-label="Staging candidate">
      <p>Entities including dependencies: {preview.report.expanded_ids.length} · Changed source files: {preview.report.changed_files.length}</p>
      <ul>{preview.report.diff.changes.filter(change=>change.kind!=="unchanged").map(change=><li key={`${change.drawing}:${change.entity_id}`}>{change.drawing} · {change.entity_id} · {change.kind}</li>)}</ul>
      <p>Configuration changes: {preview.report.diff.configuration_changes.length}</p>
      {preview.report.cad_check.diagnostics.length>0 && <ul>{preview.report.cad_check.diagnostics.map((diagnostic,index)=><li key={index}>{diagnostic.message}</li>)}</ul>}
      <div class="git-stage-preview" dangerouslySetInnerHTML={{__html:sanitizeSvg(preview.svg)}} />
      <button type="button" disabled={disabled || preview.report.cad_check.status!=="ok" || preview.report.changed_files.length===0} onClick={()=>{
        setBusy(true);
        void props.onApply(preview.report.plan_hash).then(()=>{setPreview(null);setMessage("Selected candidate staged. Working drawing files were preserved.");}).catch(error=>{setPreview(null);setMessage(`${String(error)} Review a new candidate.`);}).finally(()=>setBusy(false));
      }}>Stage reviewed candidate</button>
      <button type="button" disabled={busy} onClick={()=>setPreview(null)}>Discard staging preview</button>
    </section>}
    <p role="status">{message}</p>
  </details>;
}
