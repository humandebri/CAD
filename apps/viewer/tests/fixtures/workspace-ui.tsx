import { render } from "preact";
import { useState } from "preact/hooks";
import { useModalFocus } from "../../src/modal-focus";
import "../../src/styles.css";
import "../../src/workspace.css";

function Fixture() {
  const [open, setOpen] = useState(false);
  useModalFocus(open, () => setOpen(false));
  return <><button type="button" onClick={() => setOpen(true)}>New Project</button>
    <button type="button">Background action</button>
    {open && <div class="dialog-backdrop"><section role="dialog" aria-modal="true" aria-label="New project" class="export-dialog">
      <h2>New project</h2><label>Project name<input aria-label="Project name" /></label>
      <button type="button" onClick={() => setOpen(false)}>Close</button>
    </section></div>}
  </>;
}
render(<Fixture />, document.getElementById("app")!);
