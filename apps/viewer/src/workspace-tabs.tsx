import type { ComponentChildren } from "preact";

export type WorkspaceTab = { id: string; label: string; icon?: ComponentChildren };

/** Arrow keys move focus and activate; hidden panels remain mounted to retain drafts. */
export function WorkspaceTabs(props: {
  id: string;
  label: string;
  tabs: WorkspaceTab[];
  active: string;
  onChange: (id: string) => void;
}) {
  return <div class="workspace-tabs" role="tablist" aria-label={props.label}>
    {props.tabs.map((tab, index) => <button
      key={tab.id} type="button" role="tab" id={`${props.id}-tab-${tab.id}`}
      aria-controls={`${props.id}-panel-${tab.id}`} aria-selected={props.active === tab.id}
      tabIndex={props.active === tab.id ? 0 : -1}
      onClick={() => props.onChange(tab.id)}
      onKeyDown={event => {
        let next: number;
        if (event.key === "ArrowRight") next = (index + 1) % props.tabs.length;
        else if (event.key === "ArrowLeft") next = (index - 1 + props.tabs.length) % props.tabs.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = props.tabs.length - 1;
        else return;
        event.preventDefault();
        props.onChange(props.tabs[next].id);
        document.getElementById(`${props.id}-tab-${props.tabs[next].id}`)?.focus();
      }}
    >{tab.icon}<span>{tab.label}</span></button>)}
  </div>;
}
