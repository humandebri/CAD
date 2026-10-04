import { useEffect, useState } from "preact/hooks";
import { parseCoordinateInput } from "./command-registry";

export type ToolkitMeasurement = {
  total_length_mm: number;
  total_area_mm2: number;
  unsupported_count: number;
  supported_leaf_count: number;
  area_mode: "sum" | "union";
  additive_area_mm2: number;
  curve_tolerance_mm: number | null;
  area_approximation_error_estimate_mm2: number;
  measurements: Array<{ entity_id: string; length_mm: number | null; area_mm2: number | null; warnings: string[] }>;
};
const tools = ["double_line", "centerline", "door", "sliding_door", "window", "ellipse", "cubic_bezier", "circle_tangents", "tangent_circle", "calculated_text", "massing_projection", "sun_shadow", "sky_view", "divide"] as const;
type Tool = typeof tools[number];
export type ToolkitValues = { tool: Tool; p1: string; p2: string; p3: string; p4: string; width: string; height: string; angle: string; swing: string; count: string; tolerance: string; pen: string; expression: string; precision: string; prefix: string; suffix: string; style: string; massingHeight:string;observerZ:string;skySamples:string };
const initial: ToolkitValues = { tool: "double_line", p1: "0,0", p2: "1000,0", p3: "1000,1000", p4: "2000,1000", width: "100", height: "100", angle: "0", swing: "90", count: "2", tolerance: "0.1", pen: "", expression:"1000/3", precision:"2", prefix:"", suffix:" mm", style:"",massingHeight:"3000",observerZ:"0",skySamples:"180" };

export function buildToolkitGeometry(values: ToolkitValues, selected: string[], textStyles: string[] = []): Record<string, unknown> {
  const point = (text: string) => {
    const result = parseCoordinateInput(text, null);
    if (result === null) throw new Error("Coordinates must be x,y in model millimetres.");
    return result;
  };
  const number = (text: string, positive = false) => {
    const value = Number(text);
    if (!text.trim() || !Number.isFinite(value) || (positive && value <= 0)) throw new Error("Enter valid sizes and angles.");
    return value;
  };
  const type = values.tool;
  if (["massing_projection","sun_shadow","sky_view"].includes(type)) {
    if(!selected.length)throw new Error("Select closed polylines, solid polygons or hatch boundaries as building footprints.");
    const common={type,footprint_ids:selected,height_mm:number(values.massingHeight,true)};
    if(type==="sun_shadow")return {...common,sun_azimuth_deg:number(values.angle),sun_altitude_deg:number(values.swing)};
    if(type==="massing_projection")return {...common,origin:point(values.p1),at:point(values.p2),yaw_deg:number(values.angle),elevation_deg:number(values.swing)};
    const observer=point(values.p1),samples=number(values.skySamples);
    if(!Number.isInteger(samples)||samples<36||samples>2048)throw new Error("Sky azimuth samples must be an integer from 36 to 2048.");
    return {...common,observer:[...observer,number(values.observerZ)],azimuth_samples:samples,at:point(values.p2),radius_mm:number(values.width,true)};
  }
  if (type === "divide") {
    if (selected.length !== 1) throw new Error("Select one line or circular arc to divide.");
    return { type, entity_id: selected[0], segments: number(values.count, true) };
  }
  const p1 = point(values.p1);
  if (type === "calculated_text") {
    const precision = number(values.precision), style = values.style || textStyles[0];
    if (!Number.isInteger(precision) || precision < 0 || precision > 12) throw new Error("Decimal places must be an integer from 0 to 12.");
    if (!style || !values.expression.trim()) throw new Error("Choose a text style and enter an arithmetic expression.");
    return {type,at:p1,expression:values.expression,precision,prefix:values.prefix,suffix:values.suffix,style,rotation_deg:number(values.angle)};
  }
  if (type === "tangent_circle") {
    if (selected.length !== 2 || selected[0] === selected[1]) throw new Error("Select two distinct straight lines for a tangent circle.");
    return { type, line_ids: selected, radius: number(values.width, true), near: p1 };
  }
  if (type === "door") return { type, hinge: p1, width: number(values.width, true), rotation_deg: number(values.angle), swing_deg: number(values.swing) };
  if (type === "ellipse") return { type, center: p1, radius_x: number(values.width, true), radius_y: number(values.height, true), rotation_deg: number(values.angle), start_deg: 0, end_deg: 360 };
  const p2 = point(values.p2);
  if (type === "double_line") return { type, p1, p2, width: number(values.width, true), centerline: false };
  if (type === "centerline") return { type, p1, p2, extension: number(values.width) };
  if (type === "window") return { type, p1, p2, depth: number(values.height, true), panels: number(values.count, true) };
  if (type === "sliding_door") return { type, p1, p2, depth: number(values.height, true) };
  if (type === "circle_tangents") return { type, center: p1, radius: number(values.width, true), from: p2 };
  return { type, points: [p1, p2, point(values.p3), point(values.p4)], tolerance_mm: number(values.tolerance, true) };
}

export function ToolkitPanel(props: {
  selected: string[]; pens: string[]; disabled: boolean; previewReady: boolean;
  onGenerate: (geometry: Record<string, unknown>, pen: string | null) => Promise<string[]>;
  sourceRevision?: string;
  textStyles?: string[];
  initialPen?: string;
  initialTextStyle?: string;
  analysisReport?:unknown|null;reportSaveDisabled?:boolean;onSaveReport?:(path:string,report:unknown)=>Promise<boolean>;
  onMeasure: (options: { areaMode: "sum" | "union"; curveToleranceMm: number }) => Promise<ToolkitMeasurement>;
  onApply: () => void; onCancel: () => void;
}) {
  const [values, setValues] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [measurement, setMeasurement] = useState<ToolkitMeasurement | null>(null);
  const [areaMode, setAreaMode] = useState<"sum" | "union">("sum");
  const [curveTolerance, setCurveTolerance] = useState("0.1");
  const [reportPath,setReportPath]=useState(""),[reportMessage,setReportMessage]=useState("");
  useEffect(() => setValues(current=>({...current,pen:props.initialPen || "",style:props.initialTextStyle || ""})), [props.initialPen,props.initialTextStyle]);
  const selection = props.selected.join(",");
  useEffect(() => setMeasurement(null), [selection, props.sourceRevision, areaMode, curveTolerance]);
  const set = (key: keyof ToolkitValues, value: string) => {
    if (props.previewReady) props.onCancel();
    setMessage("");
    setValues(current => ({ ...current, [key]: value }));
  };
  const field = (key: keyof ToolkitValues, label: string, numeric = false) => <label>{label}<input aria-label={label} type={numeric ? "number" : "text"} step={numeric ? "any" : undefined} value={values[key]} onInput={event => set(key, event.currentTarget.value)} /></label>;
  const disabled = props.disabled || busy;
  return <details class="toolkit-panel">
    <summary>Building tools and measurements</summary>
    <form aria-label="Building tools" onSubmit={event => {
      event.preventDefault();
      if (disabled) return;
      if (props.previewReady) props.onCancel();
      void (async () => {
        setBusy(true);
        try {
          const warnings = await props.onGenerate(buildToolkitGeometry(values, props.selected, props.textStyles), values.pen || null);
          setMessage([...warnings, "Review the drawing preview, then apply."].join(" "));
        } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); }
        finally { setBusy(false); }
      })();
    }}>
      <fieldset disabled={disabled}><legend>Dimensions are model millimetres</legend><div class="drafting-fields">
        <label>Tool<select aria-label="Building tool" value={values.tool} onChange={event => set("tool", event.currentTarget.value)}>{tools.map(tool => <option key={tool} value={tool}>{tool.replaceAll("_", " ")}</option>)}</select></label>
        {!["divide","sun_shadow"].includes(values.tool) && field("p1", values.tool === "sky_view"?"Observer east,north":values.tool==="massing_projection"?"Projection origin east,north":values.tool === "tangent_circle" ? "Near center x,y" : ["door", "ellipse", "circle_tangents", "calculated_text"].includes(values.tool) ? "Origin x,y" : "First point x,y")}
        {!["door", "ellipse", "divide", "tangent_circle", "calculated_text","sun_shadow"].includes(values.tool) && field("p2", ["sky_view","massing_projection"].includes(values.tool)?"Diagram origin x,y":values.tool === "circle_tangents" ? "Tangent origin x,y" : "Second point x,y")}
        {["double_line", "centerline", "door", "ellipse", "circle_tangents", "tangent_circle"].includes(values.tool) && field("width", values.tool === "ellipse" ? "Radius X" : ["circle_tangents", "tangent_circle"].includes(values.tool) ? "Radius" : values.tool === "centerline" ? "Extension" : "Width", true)}
        {values.tool === "tangent_circle" && <small>Select two straight lines. Their infinite extensions define the circle; the solution nearest the specified point is previewed. Parallel lines are unsupported.</small>}
        {["window", "sliding_door", "ellipse"].includes(values.tool) && field("height", values.tool === "ellipse" ? "Radius Y" : "Depth", true)}
        {["door", "ellipse", "calculated_text"].includes(values.tool) && field("angle", "Rotation degrees", true)}
        {["massing_projection","sun_shadow","sky_view"].includes(values.tool)&&<>
          {field("massingHeight","Building height mm",true)}
          <small>Selected closed footprints become vertical prisms with base z=0 and the same height. Flat ground and horizontal roofs. Keep the conditions report for regeneration; output is ordinary 2D geometry.</small>
        </>}
        {values.tool==="massing_projection"&&<>{field("angle","Projection yaw degrees",true)}{field("swing","View elevation degrees",true)}<small>Yaw rotates +x toward +y. Elevation: 0° side view, 90° plan. All edges are shown.</small></>}
        {values.tool==="sun_shadow"&&<>{field("angle","Solar azimuth degrees",true)}{field("swing","Solar altitude degrees",true)}<small>Solar north=0°, east=90°; altitude in (0°,90°]. Ground z=0. Supply angles for the intended conditions; no date/location is inferred.</small></>}
        {values.tool==="sky_view"&&<>{field("observerZ","Observer height mm",true)}{field("skySamples","Sky azimuth samples",true)}{field("width","Sky diagram radius mm",true)}<small>Horizontal sky-view factor and orthographic sky diagram. North is +y. Sampling refinement is reported; the calculation does not assess statutory compliance.</small></>}
        {values.tool === "calculated_text" && <>
          {field("expression","Arithmetic expression")}{field("precision","Decimal places",true)}
          {field("prefix","Text prefix")}{field("suffix","Text suffix")}
          <label>Text style<select aria-label="Calculated text style" value={values.style || props.textStyles?.[0] || ""} onChange={e => set("style",e.currentTarget.value)}>{props.textStyles?.map(style=><option key={style}>{style}</option>)}</select></label>
          <small>Scalar arithmetic: + - * / parentheses and scientific notation. Decimal places: 0–12. Units come from your prefix/suffix. The saved text does not recalculate automatically.</small>
        </>}
        {values.tool === "door" && field("swing", "Swing degrees", true)}
        {["window", "divide"].includes(values.tool) && field("count", values.tool === "window" ? "Panels" : "Segments", true)}
        {values.tool === "cubic_bezier" && <>{field("p3", "Third point x,y")}{field("p4", "Fourth point x,y")}{field("tolerance", "Curve tolerance mm", true)}</>}
        <label>Pen<select aria-label="Building pen" value={values.pen} onChange={event => set("pen", event.currentTarget.value)}><option value="">Layer pen</option>{props.pens.map(pen => <option key={pen}>{pen}</option>)}</select></label>
      </div><button type="submit">Preview building tool</button></fieldset>
    </form>
    {props.analysisReport!==undefined&&props.analysisReport!==null&&<details class="massing-report" open><summary>Last massing calculation and conditions</summary>
      <textarea aria-label="Massing calculation report JSON" readOnly value={JSON.stringify(props.analysisReport,null,2)}/>
      {props.onSaveReport!==undefined&&<><label>New calculation report file<input aria-label="Massing report file" value={reportPath} disabled={busy||props.reportSaveDisabled} onInput={event=>setReportPath(event.currentTarget.value)}/></label>
        <button type="button" disabled={busy||props.reportSaveDisabled} onClick={()=>void (async()=>{setBusy(true);setReportMessage("");try{const saved=await props.onSaveReport!(reportPath.trim(),props.analysisReport);setReportMessage(saved?"Calculation report saved. Existing files are never overwritten.":"Report save cancelled.");}catch(error){setReportMessage(String(error));}finally{setBusy(false);}})()}>Save new massing report</button>
      </>}
      <p role="status">{reportMessage}</p>
    </details>}
    <div class="toolkit-actions"><button type="button" disabled={disabled || !props.previewReady} onClick={props.onApply}>Apply building preview</button><button type="button" disabled={disabled || !props.previewReady} onClick={props.onCancel}>Cancel preview</button>
      <label>Area calculation<select aria-label="Area calculation" disabled={disabled} value={areaMode} onChange={event => setAreaMode(event.currentTarget.value as "sum" | "union")}><option value="sum">Sum of entity areas</option><option value="union">Exclude overlapping areas</option></select></label>
      {areaMode === "union" && <label>Curve tolerance mm<input aria-label="Area curve tolerance mm" type="number" min="0" step="any" disabled={disabled} value={curveTolerance} onInput={event => setCurveTolerance(event.currentTarget.value)} /></label>}
      <button type="button" disabled={disabled} onClick={() => {
        const tolerance = Number(curveTolerance);
        if (!curveTolerance.trim() || !Number.isFinite(tolerance) || tolerance <= 0) { setMessage("Area curve tolerance must be positive millimetres."); return; }
        setBusy(true);
        setMessage("");
        void props.onMeasure({ areaMode, curveToleranceMm: tolerance }).then(result => { setMeasurement(result); setMessage(measurement === null ? "Measurements calculated." : "Measurements are current."); }).catch(error => { setMeasurement(null); setMessage(String(error)); }).finally(() => setBusy(false));
      }}>{props.selected.length ? "Measure selection" : "Measure drawing"}</button></div>
    <p role="status">{message}</p>
    {measurement !== null && <div aria-label="Measured geometry"><p>Length: {(measurement.total_length_mm / 1000).toFixed(4)} m · Area: {(measurement.total_area_mm2 / 1e6).toFixed(4)} m²</p><p>Unsupported entities excluded: {measurement.unsupported_count} (expanded leaves). Supported leaves: {measurement.supported_leaf_count}. Lengths sum each entity independently.</p>{measurement.area_mode === "union" ? <><p>Overlapping areas are excluded within each drawing. Sum before overlap removal: {(measurement.additive_area_mm2 / 1e6).toFixed(4)} m².</p><p>Curve tolerance: {measurement.curve_tolerance_mm} mm · Area approximation error estimate: {measurement.area_approximation_error_estimate_mm2.toPrecision(4)} mm². This estimate excludes floating point errors.</p></> : <p>Areas sum each entity independently; overlapping areas are counted separately.</p>}</div>}
  </details>;
}
