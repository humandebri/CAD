import { useEffect, useRef } from "preact/hooks";

/** One boundary for app dialogs; drafts and native file pickers keep their own state. */
export function useModalFocus(open: boolean, onDismiss: () => void) {
  const dismiss = useRef(onDismiss);
  dismiss.current = onDismiss;
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = document.querySelector<HTMLElement>(".dialog-backdrop [role='dialog']");
    if (dialog === null) return;
    const controls = () => [...dialog.querySelectorAll<HTMLElement>(
      "button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex='0']",
    )].filter(element => element.getClientRects().length > 0);
    const frame = requestAnimationFrame(() => controls()[0]?.focus());
    function handleKey(event: KeyboardEvent) {
      if (event.isComposing) return;
      if (event.key === "Escape") {
        event.preventDefault(); event.stopImmediatePropagation(); dismiss.current();
      } else if (event.key === "Tab") {
        const items = controls();
        if (items.length === 0) { event.preventDefault(); dialog?.focus(); return; }
        const first = items[0], last = items[items.length - 1];
        if (event.shiftKey && (document.activeElement === first || !dialog?.contains(document.activeElement))) {
          event.preventDefault(); last.focus();
        } else if (!event.shiftKey && (document.activeElement === last || !dialog?.contains(document.activeElement))) {
          event.preventDefault(); first.focus();
        }
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKey, true);
      if (previous?.isConnected) {
        if (previous.getClientRects().length > 0) previous.focus();
        else previous.closest("details")?.querySelector("summary")?.focus();
      }
    };
  }, [open]);
}
