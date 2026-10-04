import { useState } from "preact/hooks";

export interface CommitPreview {
  plan_hash: string;
  branch: string;
  parent_oid: string | null;
  tree_oid: string;
  message: string;
  author: { name: string; email: string };
  committer: { name: string; email: string };
  files: { status: string; path: string }[];
  patch: string;
  patch_truncated: boolean;
  cad_check: { status: string; diagnostics: { message: string }[] };
  hooks_run: boolean;
  signed: boolean;
  applied: boolean;
  commit_oid: string | null;
}

export function GitCommitPanel(props: {
  disabled: boolean;
  onPreview: (message: string) => Promise<CommitPreview>;
  onApply: (message: string, hash: string) => Promise<CommitPreview>;
  onCompareIndex: () => void;
}) {
  const [message, setMessage] = useState("");
  const [preview, setPreview] = useState<CommitPreview | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const disabled = busy || props.disabled;
  return <details class="git-stage-panel git-commit-panel"><summary>Commit staged changes</summary>
    <p>This commits the entire repository index, including changes staged outside the selected CAD entities. Working files are preserved. CAD validation covers this project only.</p>
    <p>This operation creates an unsigned commit and runs no Git hooks. Use Git directly when your workflow requires hooks or signing.</p>
    <label>Commit message<textarea aria-label="Commit message" value={message} disabled={disabled} onInput={e => { setMessage(e.currentTarget.value); setPreview(null); setAcknowledged(false); }} /></label>
    {preview === null && <button type="button" disabled={disabled || !message.trim()} onClick={() => {
      setBusy(true); setAcknowledged(false); setStatus("");
      void props.onPreview(message).then(value => { setPreview(value); setStatus("Review all staged files and the patch before committing."); }).catch(error => setStatus(String(error))).finally(() => setBusy(false));
    }}>Preview complete index</button>}
    {preview !== null && <section aria-label="Complete commit candidate">
      <p>Branch: {preview.branch} · Parent: {preview.parent_oid?.slice(0, 12) ?? "initial commit"}</p>
      <p>Author: {preview.author.name} &lt;{preview.author.email}&gt;<br />Committer: {preview.committer.name} &lt;{preview.committer.email}&gt;</p>
      <p>Staged files: {preview.files.length}</p>
      <ul>{preview.files.map(file => <li key={file.path}>{file.status} · {file.path}</li>)}</ul>
      {preview.cad_check.diagnostics.length > 0 && <ul>{preview.cad_check.diagnostics.map((d, i) => <li key={i}>{d.message}</li>)}</ul>}
      {preview.patch_truncated && <p role="alert">Patch cannot be displayed completely (size limit or text encoding). Commit is blocked; review and commit with Git directly.</p>}
      {preview.files.length === 0 && <p>There are no staged changes.</p>}
      <button type="button" disabled={disabled} onClick={props.onCompareIndex}>Compare HEAD and index</button>
      <details class="commit-patch"><summary>Review staged patch</summary><pre>{preview.patch}</pre></details>
      <label class="commit-acknowledgement"><input type="checkbox" checked={acknowledged} disabled={disabled} onChange={e => setAcknowledged(e.currentTarget.checked)} />I reviewed the whole index and accept committing without hooks or signing.</label>
      <button type="button" disabled={disabled || !acknowledged || preview.files.length === 0 || preview.cad_check.status !== "ok" || preview.patch_truncated} onClick={() => {
        setBusy(true);
        void props.onApply(preview.message, preview.plan_hash).then(result => { setPreview(null); setAcknowledged(false); setMessage(""); setStatus(`Commit created: ${result.commit_oid}. Working files and index preserved.`); }).catch(error => { setPreview(null); setAcknowledged(false); setStatus(`${String(error)} Review a new candidate before retrying.`); }).finally(() => setBusy(false));
      }}>Create reviewed commit</button>
      <button type="button" disabled={busy} onClick={() => { setPreview(null); setAcknowledged(false); }}>Discard commit preview</button>
    </section>}
    <p role="status">{status}</p>
  </details>;
}
