import {useState} from "preact/hooks";
import {sanitizeSvg} from "./svg-sanitizer";
import {PartLibraryPanel,type PartLibraryRequest,type PartLibraryReport,type LoadedLibraryPart} from "./part-library-panel";
export type CadClipboard = {
  schema_version:string; source_project:{name:string}; source_drawing:string;
  base_point:[number,number]; requested_ids:string[]; expanded_ids:string[]; warnings:string[];
  entities:unknown[]; blocks:Record<string,unknown>;
  [key:string]:unknown;
};
export type PastePreview = {svg:string;report:{plan_hash:string;status:string;entity_ids:string[];changed_files:string[];warnings:string[];
  cad_check:{status:string;diagnostics:Array<{message:string}>};mappings:Record<string,Record<string,string>>;
}};
export type JwsPart = {document:CadClipboard|null;report:{status:string;coordinate_scale:number;source_hash:string;blockers:string[];warnings:Array<{code:string;message:string;record_type:string}>;exact_round_trip:boolean;[key:string]:unknown}};
function point(value:string):[number,number] {
  const coordinates=value.split(',').map(part=>Number(part.trim()));
  if(value.split(',').some(part=>part.trim()==="")||coordinates.length!==2||!coordinates.every(Number.isFinite))throw new Error("Enter finite x,y coordinates in model millimetres.");
  return [coordinates[0],coordinates[1]];
}
export function ClipboardPanel(props:{selected:string[];disabled:boolean;pasteDisabled?:boolean;document:CadClipboard|null;onDocument:(document:CadClipboard|null)=>void;
  onCopy:(base:[number,number],dimensions:string)=>Promise<CadClipboard>;
  onPreview:(document:CadClipboard,at:[number,number],rotation:number,scale:number)=>Promise<PastePreview>;
  onApply:(document:CadClipboard,at:[number,number],hash:string,rotation:number,scale:number)=>Promise<unknown>;
  onChoosePart:(saving:boolean)=>Promise<string|null>;onSavePart:(path:string,document:CadClipboard)=>Promise<void>;onLoadPart:(path:string)=>Promise<CadClipboard>;
  onChooseJws?:()=>Promise<string|null>;onLoadJws?:(path:string,scale:number)=>Promise<JwsPart>;
  jwsReport?:JwsPart["report"]|null;onJwsReport?:(report:JwsPart["report"]|null)=>void;
  onChooseLibrary?:()=>Promise<string|null>;onListLibrary?:(request:PartLibraryRequest)=>Promise<PartLibraryReport>;onLoadLibrary?:(path:string,hash:string,scale:number|null)=>Promise<LoadedLibraryPart>;
}) {
  const [base,setBase]=useState("0,0"),[at,setAt]=useState("0,0"),[dimensions,setDimensions]=useState("include_references");
  const [rotation,setRotation]=useState("0"),[scale,setScale]=useState("1");
  function transform():[number,number] {const angle=Number(rotation),factor=Number(scale);if(!rotation.trim()||!scale.trim()||!Number.isFinite(angle)||!Number.isFinite(factor)||factor<=0)throw new Error("Paste rotation must be finite and scale must be positive and finite.");return [angle,factor];}
  const [partPath,setPartPath]=useState("");
  const [jwsPath,setJwsPath]=useState(""),[jwsScale,setJwsScale]=useState(""),[localJwsReport,setLocalJwsReport]=useState<JwsPart["report"]|null>(null);
  const jwsReport=props.jwsReport===undefined?localJwsReport:props.jwsReport;
  const setJwsReport=(report:JwsPart["report"]|null)=>{setLocalJwsReport(report);props.onJwsReport?.(report);};
  const [busy,setBusy]=useState(false),[message,setMessage]=useState(""),[preview,setPreview]=useState<PastePreview|null>(null);
  const [libraryBusy,setLibraryBusy]=useState(false);
  const disabled=busy||libraryBusy||props.disabled;
  async function action(work:()=>Promise<void>) {setBusy(true);setMessage("");try{await work();}catch(error){setMessage(String(error));}finally{setBusy(false);}}
  const document=props.document;
  return <details class="git-stage-panel clipboard-panel"><summary>CAD clipboard and parts</summary>
    <p>Copy selected geometry with dimensions and block definitions. The clipboard remains available when opening another drawing or project. Placement uses model millimetres; copied layer visibility and locks are preserved.</p>
    <div class="drafting-fields clipboard-fields">
      <label>Copy base x,y<input aria-label="Clipboard base point" value={base} disabled={disabled} onInput={event=>setBase(event.currentTarget.value)}/></label>
      <label>Dimension references<select aria-label="Clipboard dimension policy" value={dimensions} disabled={disabled} onChange={event=>setDimensions(event.currentTarget.value)}>
        <option value="include_references">Include referenced geometry</option><option value="detach_external">Detach outside selection</option><option value="reject_external">Reject outside selection</option>
      </select></label>
    </div>
    <button type="button" disabled={disabled||props.selected.length===0} onClick={()=>void action(async()=>{
      setPreview(null);const copied=await props.onCopy(point(base),dimensions);props.onDocument(copied);setMessage(`Copied ${copied.entities.length} entities and ${Object.keys(copied.blocks).length} blocks. Source files were preserved.`);
    })}>Copy selected to CAD clipboard</button>
    {document!==null && <section aria-label="CAD clipboard contents">
      <p>{document.source_project.name} · {document.source_drawing} · {document.entities.length} entities · {Object.keys(document.blocks).length} blocks<br/>Base: {document.base_point.join(", ")}</p>
      {document.warnings.length>0 && <ul>{document.warnings.map((warning,index)=><li key={index}>{warning}</li>)}</ul>}
      <label class="clipboard-placement">Paste at x,y<input aria-label="Clipboard paste point" value={at} disabled={disabled} onInput={event=>{const value=event.currentTarget.value;setPreview(null);setAt(value);}}/></label>
      <div class="drafting-fields clipboard-fields">
        <label>Paste rotation (degrees)<input aria-label="Clipboard paste rotation" value={rotation} disabled={disabled} onInput={event=>{setPreview(null);setRotation(event.currentTarget.value);}}/></label>
        <label>Uniform scale<input aria-label="Clipboard paste scale" value={scale} disabled={disabled} onInput={event=>{setPreview(null);setScale(event.currentTarget.value);}}/></label>
      </div>
      <p>Shared paper-unit text and pen styles remain at their original sizes. Horizontal/vertical reference dimensions require rotation in multiples of 90°.</p>
      <button type="button" disabled={disabled||props.pasteDisabled} onClick={()=>void action(async()=>{setPreview(null);const [angle,factor]=transform();setPreview(await props.onPreview(document,point(at),angle,factor));setMessage("Review pasted geometry, definition mappings, and all changed files.");})}>Preview CAD paste</button>
      <button type="button" disabled={disabled} onClick={()=>{setPreview(null);props.onDocument(null);setMessage("Clipboard cleared.");}}>Clear CAD clipboard</button>
    </section>}
    {preview!==null && document!==null && <section aria-label="CAD paste candidate">
      <p>Pasted entities: {preview.report.entity_ids.length} · Changed files: {preview.report.changed_files.length}</p>
      <ul>{preview.report.changed_files.map(file=><li key={file}>{file}</li>)}</ul>
      {preview.report.warnings.length>0 && <ul>{preview.report.warnings.map((warning,index)=><li key={index}>{warning}</li>)}</ul>}
      {preview.report.cad_check.diagnostics.length>0 && <ul>{preview.report.cad_check.diagnostics.map((diagnostic,index)=><li key={index}>{diagnostic.message}</li>)}</ul>}
      <details class="clipboard-mappings"><summary>Imported definition and ID mappings</summary><dl>{Object.entries(preview.report.mappings).flatMap(([kind,map])=>Object.entries(map).map(([old,value])=><><dt>{kind}: {old}</dt><dd>{value}</dd></>))}</dl></details>
      <div class="git-stage-preview" dangerouslySetInnerHTML={{__html:sanitizeSvg(preview.svg)}}/>
      <button type="button" disabled={disabled||props.pasteDisabled||preview.report.status!=="ready"||preview.report.cad_check.status!=="ok"} onClick={()=>void action(async()=>{
        const hash=preview.report.plan_hash;setPreview(null);const [angle,factor]=transform();await props.onApply(document,point(at),hash,angle,factor);setMessage("CAD paste applied. Undo restores source files and imported definitions.");
      })}>Apply reviewed CAD paste</button>
      <button type="button" disabled={busy} onClick={()=>setPreview(null)}>Discard paste preview</button>
    </section>}
    <label class="clipboard-part-path">Portable part file<input aria-label="CAD part file" value={partPath} disabled={disabled} onInput={event=>setPartPath(event.currentTarget.value)}/></label>
    <button type="button" disabled={disabled} onClick={()=>void action(async()=>{const path=await props.onChoosePart(false);if(path!==null)setPartPath(path);})}>Browse CAD part</button>
    <button type="button" disabled={disabled||!partPath.trim()} onClick={()=>void action(async()=>{setPreview(null);const loaded=await props.onLoadPart(partPath.trim());props.onDocument(loaded);setMessage("Validated CAD part loaded into the clipboard.");})}>Load CAD part</button>
    <button type="button" disabled={disabled||document===null} onClick={()=>void action(async()=>{
      const path=partPath.trim()||await props.onChoosePart(true);if(path===null||document===null)return;
      await props.onSavePart(path,document);setPartPath(path);setMessage("CAD part saved. Existing files are never overwritten.");
    })}>Save new CAD part</button>
    {props.onChooseLibrary!==undefined&&props.onListLibrary!==undefined&&props.onLoadLibrary!==undefined&&<PartLibraryPanel disabled={busy||props.disabled} onBusy={setLibraryBusy} onChoose={props.onChooseLibrary} onList={props.onListLibrary} onLoad={props.onLoadLibrary} onJwsReport={setJwsReport} onDocument={loaded=>{setPreview(null);props.onDocument(loaded);}}/>}
    {props.onLoadJws!==undefined && <section aria-label="JWS part import">
      <p>Load supported JWS 351/420/600 symbols. Enter model millimetres per stored coordinate unit (for example, 100 for paper coordinates at 1/100). Inspect replacements and warnings before pasting.</p>
      <div class="drafting-fields clipboard-fields">
        <label>JWS file<input aria-label="JWS part file" value={jwsPath} disabled={disabled} onInput={event=>setJwsPath(event.currentTarget.value)}/></label>
        <label>Coordinate scale<input aria-label="JWS coordinate scale" value={jwsScale} disabled={disabled} onInput={event=>setJwsScale(event.currentTarget.value)}/></label>
      </div>
      <button type="button" disabled={disabled||props.onChooseJws===undefined} onClick={()=>void action(async()=>{const path=await props.onChooseJws?.();if(path)setJwsPath(path);})}>Browse JWS part</button>
      <button type="button" disabled={disabled||!jwsPath.trim()||!jwsScale.trim()} onClick={()=>void action(async()=>{
        setPreview(null);setJwsReport(null);const scale=Number(jwsScale);
        if(!Number.isFinite(scale)||scale<=0)throw new Error("JWS coordinate scale must be finite and positive.");
        const imported=await props.onLoadJws!(jwsPath.trim(),scale);setJwsReport(imported.report);
        if(imported.document===null||imported.report.status!=="converted"){setMessage("JWS import blocked. The existing clipboard was preserved; inspect the compatibility report.");return;}
        props.onDocument(imported.document);setMessage("Checked JWS part loaded. Review the compatibility report, then preview its placement.");
      })}>Load checked JWS part</button>
      {jwsReport!==null && <details class="clipboard-mappings" open><summary>JWS compatibility report: {jwsReport.status}</summary>
        <p>Exact round-trip: {String(jwsReport.exact_round_trip)} · Coordinate scale: {jwsReport.coordinate_scale}</p>
        {jwsReport.blockers.length>0&&<ul>{jwsReport.blockers.map((blocker,index)=><li key={index}>{blocker}</li>)}</ul>}
        {jwsReport.warnings.length>0&&<ul>{jwsReport.warnings.map((warning,index)=><li key={index}>{warning.code}: {warning.message}</li>)}</ul>}
        <textarea aria-label="JWS compatibility report JSON" readOnly value={JSON.stringify(jwsReport,null,2)}/>
      </details>}
    </section>}
    <p role="status">{message}</p>
  </details>;
}
