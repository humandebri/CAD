export function closestEntityId(target: EventTarget | null): string | null {
  return target instanceof Element
    ? target.closest("[data-entity-id]")?.getAttribute("data-entity-id") ?? null
    : null;
}
