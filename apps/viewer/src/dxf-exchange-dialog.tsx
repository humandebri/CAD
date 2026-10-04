import { useEffect, useRef, useState } from "preact/hooks";

export interface DxfFileReport {
  operation: string;
  status: string;
  output_path: string;
  report_path: string;
  exchange: {
    status: string;
    units: string;
    warnings: { code: string; message: string; entity_id?: string | null }[];
    blockers: { code: string; message: string; entity_id?: string | null }[];
  };
}

export function DxfExchangeDialog(props: {
  mode: "import" | "export";
  drawings: string[];
  currentDrawing: string;
  onChooseInput: () => Promise<string | null>;
  onChooseOutput: (input: string) => Promise<string | null>;
  onSubmit: (values: { input: string; output: string; report: string; drawing: string; unitMm: number | null; strict: boolean }) => Promise<DxfFileReport>;
  onClose: () => void;
}) {
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [report, setReport] = useState("");
  const [drawing, setDrawing] = useState(props.currentDrawing || props.drawings[0] || "");
  const [unit, setUnit] = useState("");
  const [strict, setStrict] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<DxfFileReport | null>(null);
  const [message, setMessage] = useState("");
  const dialog = useRef<HTMLElement>(null);
  useEffect(() => {
    const previous = document.activeElement;
    dialog.current?.querySelector<HTMLElement>("input,select,button")?.focus();
    return () => { if (previous instanceof HTMLElement && previous.isConnected) previous.focus(); };
  }, []);
  const importing = props.mode === "import";
  const title = importing ? "Import DXF" : "Export DXF";
  const updateOutput = (path: string) => {
    setOutput(path);
    setReport(`${path}.${importing ? "import-" : ""}report.json`);
    setResult(null);
  };
  async function browseInput() {
    setBusy(true);
    try { const path = await props.onChooseInput(); if (path !== null) { setInput(path); setResult(null); } }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }
  async function browseOutput() {
    setBusy(true);
    try { const path = await props.onChooseOutput(input); if (path !== null) updateOutput(path); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }
  async function submit() {
    const unitMm = unit.trim() === "" ? null : Number(unit);
    if (unitMm !== null && (!Number.isFinite(unitMm) || unitMm <= 0)) {
      setMessage("Millimeters per drawing unit must be a positive number."); return;
    }
    setBusy(true); setResult(null); setMessage("");
    try {
      const result = await props.onSubmit({ input: input.trim(), output: output.trim(), report: report.trim(), drawing, unitMm, strict });
      setResult(result);
      setMessage(result.status === "blocked" ? "Conversion blocked. Report saved; no output published." : importing ? "Checked CAD project saved and opened." : "DXF and compatibility report saved.");
    } catch (error) { setMessage(`Exchange failed: ${String(error)}. Retain any report saved before the failure.`); }
    finally { setBusy(false); }
  }
  return <div class="dialog-backdrop">
    <section ref={dialog} class="dxf-exchange-dialog" role="dialog" aria-modal="true" aria-label={title} onKeyDown={event => {
      if (event.key === "Escape" && !busy) { event.preventDefault(); props.onClose(); }
      if (event.key === "Tab") {
        const controls = [...event.currentTarget.querySelectorAll<HTMLElement>("input:not(:disabled),select:not(:disabled),button:not(:disabled)")];
        const next = event.shiftKey ? controls.at(-1) : controls[0];
        if ((event.shiftKey && document.activeElement === controls[0]) || (!event.shiftKey && document.activeElement === controls.at(-1))) { event.preventDefault(); next?.focus(); }
      }
    }}>
      <h2>{title}</h2>
      <p>{importing ? "Import planar ASCII DXF into a new CAD project. Unsupported records block conversion." : "Export one drawing as R2013 model-space DXF in millimeters. Page layouts and source relationships become approximations."}</p>
      {importing && <label>DXF input<div class="exchange-path"><input aria-label="DXF input" value={input} disabled={busy} onInput={e => { setInput(e.currentTarget.value); setResult(null); }} /><button type="button" disabled={busy} onClick={() => void browseInput()}>Choose DXF</button></div></label>}
      {!importing && <label>Drawing<select aria-label="DXF drawing" disabled={busy} value={drawing} onChange={e => setDrawing(e.currentTarget.value)}>{props.drawings.map(name => <option key={name} value={name}>{name}</option>)}</select></label>}
      <label>{importing ? "New project directory" : "New DXF file"}<div class="exchange-path"><input aria-label="DXF output" value={output} disabled={busy} onInput={e => updateOutput(e.currentTarget.value)} /><button type="button" disabled={busy} onClick={() => void browseOutput()}>Choose destination</button></div></label>
      <label>New compatibility report<input aria-label="DXF report" value={report} disabled={busy} onInput={e => { setReport(e.currentTarget.value); setResult(null); }} /></label>
      {importing ? <label>Millimeters per drawing unit (blank: use file units)<input aria-label="DXF unit override" type="number" min="0" step="any" disabled={busy} value={unit} onInput={e => setUnit(e.currentTarget.value)} /></label> : <label class="exchange-checkbox"><input type="checkbox" checked={strict} disabled={busy} onChange={e => setStrict(e.currentTarget.checked)} />Block every approximation or substitution</label>}
      <p>Existing outputs are protected. Import reports must be outside the new project directory. Reports record conversion readiness; check the saved output after a publication error.</p>
      <p role="status">{message}</p>
      {result !== null && <div class="exchange-result">
        <p>Report: {result.report_path}</p>
        {[...result.exchange.blockers, ...result.exchange.warnings].map((issue, i) => <p key={i}><strong>{issue.code}</strong>{issue.entity_id ? ` (${issue.entity_id})` : ""}: {issue.message}</p>)}
      </div>}
      <div class="exchange-actions"><button type="button" disabled={busy || !output.trim() || !report.trim() || (importing ? !input.trim() : !drawing)} onClick={() => void submit()}>{busy ? "Converting…" : title}</button><button type="button" data-dismiss disabled={busy} onClick={props.onClose}>Close</button></div>
    </section>
  </div>;
}
