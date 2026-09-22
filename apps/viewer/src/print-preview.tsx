import { useEffect, useRef, useState } from "preact/hooks";
import { invoke } from "@tauri-apps/api/core";
import type { PDFDocumentProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { formatError } from "./app-errors";

export function PrintPreview(props: { projectPath: string; drawing: string; revision: unknown }) {
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [error, setError] = useState("");
  const [width, setWidth] = useState(0);
  const [rendered, setRendered] = useState(false);
  const frame = useRef<HTMLElement>(null);
  const surface = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const observer = new ResizeObserver(entries => setWidth(Math.floor(entries[0].contentRect.width)));
    if (frame.current) {
      setWidth(Math.floor(frame.current.getBoundingClientRect().width));
      observer.observe(frame.current);
    }
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let active = true;
    let loading: ReturnType<typeof import("pdfjs-dist").getDocument> | undefined;
    setPdf(null); setError(""); setRendered(false);
    surface.current?.replaceChildren();
    void Promise.all([
      invoke<number[]>("preview_drawing_pdf", { projectPath: props.projectPath, drawing: props.drawing }),
      import("pdfjs-dist/legacy/build/pdf.mjs"),
    ]).then(async ([bytes, pdfjs]) => {
      if (!active) return;
      pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;
      loading = pdfjs.getDocument({ data: new Uint8Array(bytes), useWasm: false });
      const document = await loading.promise;
      if (active) setPdf(document);
    }).catch(error => { if (active) setError(formatError(error, "Unable to render print preview")); });
    return () => { active = false; if (loading) void loading.destroy(); };
  }, [props.projectPath, props.drawing, props.revision]);

  useEffect(() => {
    if (!pdf || width <= 0) return;
    let active = true;
    let task: import("pdfjs-dist").RenderTask | undefined;
    setRendered(false);
    void pdf.getPage(1).then(async page => {
      if (!active) return;
      const canvas = document.createElement("canvas");
      const viewport = page.getViewport({ scale: width / page.getViewport({ scale: 1 }).width });
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.ceil(viewport.width * ratio);
      canvas.height = Math.ceil(viewport.height * ratio);
      canvas.style.width = `${viewport.width}px`;
      canvas.style.height = `${viewport.height}px`;
      canvas.setAttribute("aria-label", "Printed drawing");
      canvas.setAttribute("role", "img");
      task = page.render({ canvas, viewport, intent: "print", transform: [ratio, 0, 0, ratio, 0, 0] });
      await task.promise;
      if (active) { surface.current?.replaceChildren(canvas); setRendered(true); }
    }).catch(error => { if (active) setError(formatError(error, "Unable to draw PDF page")); });
    return () => { active = false; task?.cancel(); };
  }, [pdf, width]);

  return <section ref={frame} class="print-preview" aria-label="PDF print preview" data-rendered={rendered}>
    {error ? <p role="alert">{error}</p> : !rendered && <p role="status">Rendering PDF…</p>}
    <div ref={surface} />
  </section>;
}
