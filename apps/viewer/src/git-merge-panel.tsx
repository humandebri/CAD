import {useRef, useState} from "preact/hooks";
import {sanitizeSvg} from "./svg-sanitizer";

export type MergePreview = {svg: string; report: {
  plan_hash: string; status: string; changed_files: string[]; blockers: string[];
  merge: {base: {commit_oid: string|null}; theirs: {commit_oid: string|null}; conflicts: Array<{file:string;field:string;kind:string;base:unknown;ours:unknown;theirs:unknown}>};
  cad_check: {status: string; diagnostics: Array<{message:string}>};
  diff: {changes: Array<{drawing:string;entity_id:string;kind:string}>; configuration_changes:unknown[]}|null;
}};
export function GitMergePanel(props: {disabled:boolean; onPreview:(base:string,theirs:string)=>Promise<MergePreview>; onApply:(base:string,theirs:string,hash:string)=>Promise<unknown>}) {
  const [base,setBase]=useState("HEAD~1"), [theirs,setTheirs]=useState("HEAD");
  const [preview,setPreview]=useState<MergePreview|null>(null), [busy,setBusy]=useState(false), [message,setMessage]=useState("");
  const sequence=useRef(0);
  const disabled=props.disabled || busy;
  function discard() {++sequence.current;setPreview(null);setMessage("");}
  return <details class="git-stage-panel git-merge-panel"><summary>Merge CAD source changes</summary>
    <p>Compare a common base, the working project, and an incoming commit. Apply a reviewed clean candidate to existing source files with Undo. Git branches and the index are preserved. Comments, provenance, and file creation/deletion require a separate migration.</p>
    <div class="drafting-fields merge-revisions">
      <label>Common base commit<input aria-label="Merge base commit" value={base} disabled={disabled} onInput={event=>{const value=event.currentTarget.value;discard();setBase(value);}} /></label>
      <label>Incoming commit<input aria-label="Merge incoming commit" value={theirs} disabled={disabled} onInput={event=>{const value=event.currentTarget.value;discard();setTheirs(value);}} /></label>
    </div>
    <button type="button" disabled={disabled || !base.trim() || !theirs.trim()} onClick={()=>{
      const request=++sequence.current;setBusy(true);setPreview(null);setMessage("Preparing merge candidate...");
      void props.onPreview(base.trim(),theirs.trim()).then(value=>{if(request===sequence.current){setPreview(value);setMessage(value.report.status==="ready"?"Review all changed files before applying.":"Candidate is blocked. Resolve the listed conflicts or restrictions first.");}}).catch(error=>{if(request===sequence.current)setMessage(String(error));}).finally(()=>setBusy(false));
    }}>Preview CAD merge</button>
    {preview!==null && <section aria-label="CAD merge candidate">
      <p>Base: <code>{preview.report.merge.base.commit_oid}</code><br/>Incoming: <code>{preview.report.merge.theirs.commit_oid}</code></p>
      <p>Changed source files: {preview.report.changed_files.length} · Configuration changes: {preview.report.diff?.configuration_changes.length ?? 0}</p>
      <ul>{preview.report.changed_files.map(file=><li key={file}>{file}</li>)}</ul>
      {preview.report.blockers.length>0 && <ul class="merge-blockers">{preview.report.blockers.map((item,index)=><li key={index}>{item}</li>)}</ul>}
      {preview.report.merge.conflicts.map((conflict,index)=><details key={index} class="merge-conflict"><summary>{conflict.file} · {conflict.field} · {conflict.kind}</summary>
        <dl>{(["base","ours","theirs"] as const).map(key=><><dt>{key}</dt><dd><pre>{JSON.stringify(conflict[key],null,2)}</pre></dd></>)}</dl>
      </details>)}
      {preview.report.cad_check.diagnostics.length>0 && <ul>{preview.report.cad_check.diagnostics.map((diagnostic,index)=><li key={index}>{diagnostic.message}</li>)}</ul>}
      {preview.report.diff!==null && <ul>{preview.report.diff.changes.filter(change=>change.kind!=="unchanged").map(change=><li key={`${change.drawing}:${change.entity_id}`}>{change.drawing} · {change.entity_id} · {change.kind}</li>)}</ul>}
      {preview.svg!=="" && <div class="git-stage-preview" dangerouslySetInnerHTML={{__html:sanitizeSvg(preview.svg)}}/>}
      <button type="button" disabled={disabled || preview.report.status!=="ready" || preview.report.cad_check.status!=="ok" || preview.report.blockers.length>0} onClick={()=>{
        setBusy(true);setMessage("Applying reviewed source changes...");
        void props.onApply(base.trim(),theirs.trim(),preview.report.plan_hash).then(()=>{setPreview(null);setMessage("CAD merge applied. Undo restores the previous working files.");}).catch(error=>{setPreview(null);setMessage(`${String(error)} Review a new candidate.`);}).finally(()=>setBusy(false));
      }}>Apply reviewed CAD merge</button>
      <button type="button" disabled={busy} onClick={discard}>Discard merge preview</button>
    </section>}
    <p role="status">{message}</p>
  </details>;
}
