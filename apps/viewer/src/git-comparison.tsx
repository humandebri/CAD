import { useEffect, useState } from "preact/hooks";
import type { GitComparison } from "./desktop-loader";

export function GitComparisonControls(props: {
  value?: GitComparison;
  disabled: boolean;
  onApply: (comparison: GitComparison) => void;
}) {
  const [base, setBase] = useState(props.value?.base ?? "HEAD");
  const [head, setHead] = useState(props.value?.head ?? "worktree");
  useEffect(() => {
    setBase(props.value?.base ?? "HEAD");
    setHead(props.value?.head ?? "worktree");
  }, [props.value?.base, props.value?.head]);
  return <form class="git-comparison" aria-label="Git revision comparison" onSubmit={event => {
    event.preventDefault();
    if (base.trim() && head.trim() && !props.disabled) props.onApply({ base: base.trim(), head: head.trim() });
  }}>
    <label>Compare from<input aria-label="Comparison base revision" list="cad-git-revisions" value={base} disabled={props.disabled} onInput={event => setBase(event.currentTarget.value)} /></label>
    <label>to<input aria-label="Comparison head revision" list="cad-git-revisions" value={head} disabled={props.disabled} onInput={event => setHead(event.currentTarget.value)} /></label>
    <datalist id="cad-git-revisions"><option value="HEAD" /><option value="HEAD~1" /><option value="index" /><option value="worktree" /></datalist>
    <button type="submit" class="tool-button" disabled={props.disabled || !base.trim() || !head.trim()}>Compare revisions</button>
  </form>;
}
