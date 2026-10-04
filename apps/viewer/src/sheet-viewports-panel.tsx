import { useMemo, useState } from "preact/hooks";
import type { LayoutWorkspaceState } from "./artifacts";
import { parseCoordinateInput } from "./command-registry";
import { sanitizeSvg } from "./svg-sanitizer";

export type SheetViewport=NonNullable<LayoutWorkspaceState["viewports"]>[number];
export type SheetPreview={report:{plan_hash:string;warnings:string[];[key:string]:unknown};svg:string};
type Draft={name:string;drawing:string;origin:string;at:string;size:string;scale:string;layers:string[]};
const draft=(view:SheetViewport):Draft=>({name:view.name,drawing:view.drawing,origin:view.origin.join(","),at:view.at_mm.join(","),size:view.size_mm.join(","),scale:view.scale,layers:view.layers??[]});
const parse=(view:Draft):SheetViewport=>{
  const point=(text:string)=>{const value=parseCoordinateInput(text,null);if(value===null)throw new Error("Coordinates must be two finite numbers separated by a comma.");return value;};
  const at=point(view.at),size=point(view.size);
  if(at.some(v=>v<0)||size.some(v=>v<=0))throw new Error("Paper position must be nonnegative and viewport size must be positive.");
  return {name:view.name.trim(),drawing:view.drawing,origin:point(view.origin),at_mm:at,size_mm:size,scale:view.scale.trim(),layers:view.layers};
};
export function SheetViewportsPanel(props:{layout:LayoutWorkspaceState;drawing:string;drawings:string[];layers:string[];disabled:boolean;onPreview:(views:SheetViewport[])=>Promise<SheetPreview>;onApply:(views:SheetViewport[],hash:string)=>Promise<unknown>}) {
  const [views,setViews]=useState<Draft[]>(()=> (props.layout.viewports??[]).map(draft));
  const [busy,setBusy]=useState(false),[message,setMessage]=useState("");
  const [candidate,setCandidate]=useState<{views:SheetViewport[];preview:SheetPreview}|null>(null);
  const svg=useMemo(()=>candidate===null?"":sanitizeSvg(candidate.preview.svg),[candidate]);
  const disabled=props.disabled||busy;
  const change=(next:Draft[])=>{setViews(next);setCandidate(null);setMessage("");};
  const update=(index:number,key:keyof Draft,value:string|string[])=>change(views.map((view,i)=>i===index?{...view,[key]:value}:view));
  return <details class="sheet-viewports-panel"><summary>Scaled views on one sheet</summary>
    <p>Layout {props.layout.id}: {props.layout.paper} {props.layout.orientation}. Paper positions and sizes are millimetres from the lower left. Model origins use model millimetres.</p>
    <p>Each view clips the source drawing. Source geometry and measurements stay unchanged. Text sizes follow each scale; pen widths stay paper sizes. Empty views restore the normal model display. Use SVG/PDF for this sheet; JWW export is blocked.</p>
    <fieldset disabled={disabled}><legend>Sheet views</legend>
      {views.map((view,index)=><fieldset key={index}><legend>View {index+1}</legend>
        <div class="drafting-fields">
          <label>Name<input aria-label={`View ${index+1} name`} value={view.name} onInput={e=>update(index,"name",e.currentTarget.value)}/></label>
          <label>Source drawing<select aria-label={`View ${index+1} drawing`} value={view.drawing} onChange={e=>update(index,"drawing",e.currentTarget.value)}>{props.drawings.map(drawing=><option key={drawing}>{drawing}</option>)}</select></label>
          <label>Model origin east,north<input aria-label={`View ${index+1} model origin`} value={view.origin} onInput={e=>update(index,"origin",e.currentTarget.value)}/></label>
          <label>Paper position x,y mm<input aria-label={`View ${index+1} paper position`} value={view.at} onInput={e=>update(index,"at",e.currentTarget.value)}/></label>
          <label>Paper width,height mm<input aria-label={`View ${index+1} paper size`} value={view.size} onInput={e=>update(index,"size",e.currentTarget.value)}/></label>
          <label>Scale<input aria-label={`View ${index+1} scale`} value={view.scale} onInput={e=>update(index,"scale",e.currentTarget.value)}/></label>
          <label>Layers (none selected means all)<select multiple aria-label={`View ${index+1} layers`} onChange={e=>update(index,"layers",Array.from(e.currentTarget.selectedOptions).map(o=>o.value))}>{props.layers.map(layer=><option key={layer} selected={view.layers.includes(layer)}>{layer}</option>)}</select></label>
        </div>
        <button type="button" onClick={()=>change(views.filter((_,i)=>i!==index))}>Remove view {index+1}</button>
      </fieldset>)}
      <button type="button" disabled={views.length>=32} onClick={()=>{let number=1;while(views.some(v=>v.name===`View ${number}`))number++;change([...views,{name:`View ${number}`,drawing:props.drawing,origin:"0,0",at:"10,10",size:"100,100",scale:"1/100",layers:[]}]);}}>Add sheet view</button>
      <button type="button" onClick={()=>void(async()=>{setBusy(true);setCandidate(null);setMessage("");try {const parsed=views.map(parse);const preview=await props.onPreview(parsed);setCandidate({views:parsed,preview});setMessage("Review the clipped sheet preview, then apply.");}catch(error){setMessage(String(error));}finally{setBusy(false);}})()}>Preview scaled sheet</button>
    </fieldset>
    {candidate!==null&&<section aria-label="Scaled sheet preview"><div class="sheet-preview-svg" dangerouslySetInnerHTML={{__html:svg}}/>{candidate.preview.report.warnings.map((warning,i)=><p key={i}>{warning}</p>)}<details><summary>Sheet calculation and source report</summary><textarea aria-label="Scaled sheet report JSON" readOnly value={JSON.stringify(candidate.preview.report,null,2)}/></details></section>}
    <button type="button" disabled={disabled||candidate===null} onClick={()=>void(async()=>{if(candidate===null)return;setBusy(true);setMessage("");try{await props.onApply(candidate.views,candidate.preview.report.plan_hash);setCandidate(null);setMessage("Sheet views saved. Geometry is unchanged; project Undo restores the layout.");}catch(error){setCandidate(null);setMessage(String(error));}finally{setBusy(false);}})()}>Apply reviewed sheet views</button>
    <p role="status">{message}</p>
  </details>;
}
