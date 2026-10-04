import {useState} from "preact/hooks";
import {sanitizeSvg} from "./svg-sanitizer";
import type {CadClipboard,JwsPart} from "./clipboard-panel";
export type PartLibraryRequest={directory:string;offset:number;limit:number;coordinate_scale:number|null};
export type PartLibraryEntry={name:string;path:string|null;kind:string;status:string;bytes:number;source_blake3:string|null;entities:number;blocks:number;svg:string|null;warnings:string[];blockers:string[];jws_report:JwsPart["report"]|null};
export type PartLibraryReport={directory:string;total:number;offset:number;limit:number;entries:PartLibraryEntry[]};
export type LoadedLibraryPart={document:CadClipboard;jws_report:JwsPart["report"]|null};
export function PartLibraryPanel(props:{disabled:boolean;onBusy:(busy:boolean)=>void;onChoose:()=>Promise<string|null>;onList:(request:PartLibraryRequest)=>Promise<PartLibraryReport>;onLoad:(path:string,hash:string,scale:number|null)=>Promise<LoadedLibraryPart>;onDocument:(document:CadClipboard)=>void;onJwsReport:(report:JwsPart["report"]|null)=>void}) {
  const [directory,setDirectory]=useState(""),[scale,setScale]=useState("");
  const [page,setPage]=useState<PartLibraryReport|null>(null),[pageScale,setPageScale]=useState<number|null>(null);
  const [busy,setBusy]=useState(false),[message,setMessage]=useState("");const disabled=busy||props.disabled;
  async function action(work:()=>Promise<void>) {setBusy(true);props.onBusy(true);setMessage("");try{await work();}catch(error){setMessage(String(error));}finally{setBusy(false);props.onBusy(false);}}
  async function list(offset:number) {
    const value=scale.trim()?Number(scale):null;
    if(value!==null&&(!Number.isFinite(value)||value<=0))throw new Error("JWS coordinate scale must be finite and positive.");
    setPage(null);const report=await props.onList({directory:directory.trim(),offset,limit:12,coordinate_scale:value});setPageScale(value);setPage(report);setMessage(`${report.total} parts. Thumbnails use each part's original layer visibility.`);
  }
  return <details class="part-library-panel"><summary>Part folder palette</summary>
    <p>Browse .cadpart.json and .jws files in one folder. Checked thumbnails show evaluated dimensions and blocks. Large thumbnails may be omitted; selection still validates the complete part. JWS requires an explicit coordinate scale.</p>
    <div class="drafting-fields clipboard-fields">
      <label>Part folder<input aria-label="Part library folder" value={directory} disabled={disabled} onInput={event=>{setPage(null);setDirectory(event.currentTarget.value);}}/></label>
      <label>JWS coordinate scale (optional)<input aria-label="Part library JWS scale" value={scale} disabled={disabled} onInput={event=>{setPage(null);setScale(event.currentTarget.value);}}/></label>
    </div>
    <button type="button" disabled={disabled} onClick={()=>void action(async()=>{const selected=await props.onChoose();if(selected!==null){setPage(null);setDirectory(selected);}})}>Browse part folder</button>
    <button type="button" disabled={disabled||!directory.trim()} onClick={()=>void action(()=>list(0))}>Load part folder</button>
    {page!==null && <section aria-label="Part library results">
      <p>{page.directory} · {page.total} parts · {Math.min(page.offset+1,page.total)}–{Math.min(page.offset+page.limit,page.total)}</p>
      <button type="button" disabled={disabled||page.offset===0} onClick={()=>void action(()=>list(Math.max(0,page.offset-page.limit)))}>Previous part page</button>
      <button type="button" disabled={disabled||page.offset+page.limit>=page.total} onClick={()=>void action(()=>list(page.offset+page.limit))}>Next part page</button>
      <div class="part-library-grid">{page.entries.map(entry=><article class="part-library-card" key={entry.path??entry.name}>
        <h3>{entry.name}</h3><p>{entry.kind} · {entry.status} · {entry.entities} entities · {entry.blocks} blocks</p>
        {entry.svg!==null&&<div class="part-library-thumbnail" aria-label={`Preview ${entry.name}`} dangerouslySetInnerHTML={{__html:sanitizeSvg(entry.svg)}}/>}
        {entry.blockers.length>0&&<ul>{entry.blockers.map((blocker,index)=><li key={index}>{blocker}</li>)}</ul>}
        {entry.warnings.length>0&&<details><summary>Part warnings ({entry.warnings.length})</summary><ul>{entry.warnings.map((warning,index)=><li key={index}>{warning}</li>)}</ul></details>}
        {entry.jws_report!==null&&<details><summary>JWS compatibility report</summary><textarea aria-label={`JWS report for ${entry.name}`} readOnly value={JSON.stringify(entry.jws_report,null,2)}/></details>}
        <button type="button" disabled={disabled||entry.status!=="ready"||entry.path===null||entry.source_blake3===null} onClick={()=>void action(async()=>{
          const loaded=await props.onLoad(entry.path!,entry.source_blake3!,pageScale);props.onJwsReport(loaded.jws_report);props.onDocument(loaded.document);setMessage(`Loaded ${entry.name} into the CAD clipboard. Preview placement before applying.`);
        })}>Use part {entry.name}</button>
      </article>)}</div>
    </section>}
    <p role="status">{busy?"Loading checked part data…":message}</p>
  </details>;
}
