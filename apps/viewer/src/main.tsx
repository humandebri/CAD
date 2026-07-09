/**
 * apps/viewer: mounts the Phase 0 Preact shell for the CAD review UI.
 * Real SVG/check/diff loading starts after the CLI build artifacts exist.
 */
import { render } from "preact";
import { DraftingCompass } from "lucide-preact";
import "./styles.css";

function App() {
  return (
    <main class="app-shell">
      <section class="toolbar" aria-label="Viewer toolbar">
        <DraftingCompass size={20} aria-hidden="true" />
        <span>CAD Viewer</span>
      </section>
      <section class="workspace" aria-label="CAD preview">
        <p>Phase 0 scaffold</p>
      </section>
    </main>
  );
}

const root = document.getElementById("app");

if (root === null) {
  throw new Error("Missing #app root element");
}

render(<App />, root);
