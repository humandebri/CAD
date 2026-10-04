import {useState} from 'preact/hooks';
export type EntityHistory = {
  pinned_revision: string; scanned_commits: number; truncated: boolean; warnings: string[];
  events: Array<{commit_oid:string;parent_oid:string|null;subject:string;timestamp:number;drawing:string;kind:string;reasons:string[];before:unknown;after:unknown}>;
};
export type FieldCommit = {commit_oid:string;parent_oid:string|null;subject:string;author:string;timestamp:number};
export type EntityBlame = {
  pinned_revision:string;drawing:string;drawing_origin:FieldCommit|null;scanned_commits:number;truncated:boolean;reliable:boolean;warnings:string[];
  fields:Array<{pointer:string;value:unknown;origin:FieldCommit|null}>;
  dependency_changes:Array<{commit:FieldCommit;reasons:string[]}>;
};
export function EntityHistoryPanel(props:{onLoad:(limit:number)=>Promise<EntityHistory>;onLoadBlame?:(limit:number)=>Promise<EntityBlame>;onCompare:(base:string,head:string)=>void;disabled:boolean}) {
  const [history,setHistory]=useState<EntityHistory|null>(null);
  const [blame,setBlame]=useState<EntityBlame|null>(null);
  const [busy,setBusy]=useState(false);
  const [limit,setLimit]=useState(50);
  const [message,setMessage]=useState('');
  return <details class="git-stage-panel entity-history-panel"><summary>Selected entity history</summary>
    <label>Commits to scan<input aria-label="History commit limit" type="number" min="1" max="500" step="1" value={limit} onInput={event=>setLimit(Number(event.currentTarget.value))} /></label>
    <button type="button" disabled={props.disabled||busy||!Number.isInteger(limit)||limit<1||limit>500} onClick={()=>{
      setBusy(true);setMessage('Loading entity history…');
      void props.onLoad(limit).then(value=>{setHistory(value);setMessage(history?.pinned_revision===value.pinned_revision ? `History is current at ${value.pinned_revision.slice(0,12)}.` : `Loaded ${value.events.length} entity changes.`);}).catch(error=>setMessage(String(error))).finally(()=>setBusy(false));
    }}>Load entity history</button>
    {props.onLoadBlame && <button type="button" disabled={props.disabled||busy||!Number.isInteger(limit)||limit<1||limit>500} onClick={()=>{
      setBusy(true);setMessage('Loading committed field origins…');
      void props.onLoadBlame!(limit).then(value=>{setBlame(value);setMessage(blame?.pinned_revision===value.pinned_revision ? `Field origins are current at ${value.pinned_revision.slice(0,12)}.` : `Loaded ${value.fields.length} committed field origins.`);}).catch(error=>{setBlame(null);setMessage(String(error));}).finally(()=>setBusy(false));
    }}>Load field origins</button>}
    <p role="status">{message}</p>
    {blame!==null && <section aria-label="Entity field origins">
      <p>Committed fields at {blame.pinned_revision.slice(0,12)} · {blame.drawing} · {blame.scanned_commits} commits scanned.</p>
      <p>First-parent attribution of normalized fields. Coordinate arrays are grouped. Working edits are not included.</p>
      {!blame.reliable && <p class="field-origin-warning">History is incomplete; displayed commits are provisional.{blame.truncated?' Scan limit reached.':''}</p>}
      {blame.warnings.length>0 && <ul>{blame.warnings.map((warning,index)=><li key={index}>{warning}</li>)}</ul>}
      <p>Drawing location: {blame.drawing_origin?.subject ?? 'Origin unresolved'}</p>
      <dl class="field-origins">{blame.fields.map(field=><div key={field.pointer}>
        <dt><code>{field.pointer}</code></dt><dd>
          <details class="field-value"><summary>View field value</summary><pre>{JSON.stringify(field.value,null,2)}</pre></details>
          {field.origin===null ? <p>Origin unresolved in scanned history.</p> : <>
            <p>{field.origin.subject} · {field.origin.author} · <code>{field.origin.commit_oid.slice(0,12)}</code></p>
            <button type="button" disabled={props.disabled||field.origin.parent_oid===null} onClick={()=>field.origin?.parent_oid && props.onCompare(field.origin.parent_oid,field.origin.commit_oid)}>Compare field change {field.pointer}</button>
          </>}
        </dd>
      </div>)}</dl>
      {blame.dependency_changes.length>0 && <>
        <h4>Display changes with unchanged entity fields</h4>
        <ul>{blame.dependency_changes.map(change=><li key={change.commit.commit_oid}>{change.commit.subject} · {change.reasons.join(', ')} · <code>{change.commit.commit_oid.slice(0,12)}</code></li>)}</ul>
      </>}
    </section>}
    {history!==null && <section aria-label="Entity Git history">
      <p>First-parent history at {history.pinned_revision.slice(0,12)} · {history.scanned_commits} commits scanned{history.truncated?' · Scan limit reached; older changes may exist.':''}</p>
      {history.warnings.length>0 && <ul>{history.warnings.map((warning,index)=><li key={index}>{warning}</li>)}</ul>}
      {history.events.length===0 && <p>No entity changes found in the scanned commits.</p>}
      <ol>{history.events.map((event,index)=><li key={`${event.commit_oid}:${event.drawing}:${index}`}>
        <p>{event.subject} · {event.kind} · {event.drawing}<br /><code>{event.commit_oid.slice(0,12)}</code> · {new Date(event.timestamp*1000).toLocaleString()}<br />{event.reasons.join(', ')}</p>
        <button type="button" disabled={props.disabled||event.parent_oid===null} onClick={()=>event.parent_oid!==null&&props.onCompare(event.parent_oid,event.commit_oid)}>Compare this change</button>
      </li>)}</ol>
    </section>}
  </details>;
}
