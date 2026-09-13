import { expect, test } from "@playwright/test";
import { LatestLayerRulesQueue, PdfExportGuard } from "../../src/desktop-layers";
import type { LayerMutationResult, LayerWorkspaceState } from "../../src/artifacts";

const layers: LayerWorkspaceState = {
  revision: "revision-1",
  active_layer: "a",
  groups: [],
  layers: [
    { id: "a", name: "A", order: 0, visible: true, locked: false, printable: true, used_entity_count: 1 },
    { id: "b", name: "B", order: 1, visible: true, locked: false, printable: true, used_entity_count: 0 },
  ],
};

test("PDF export guard rejects results after a context invalidation", () => {
  const guard = new PdfExportGuard();
  const first = guard.begin();
  expect(guard.accepts(first)).toBe(true);
  guard.invalidate();
  expect(guard.accepts(first)).toBe(false);
  const latest = guard.begin();
  expect(guard.accepts(latest)).toBe(true);
});

test.describe("desktop layer transport", () => {
  test("serializes patches and discards the older complete snapshot", async () => {
    const calls: string[] = [];
    const resolvers: Array<(state: LayerMutationResult) => void> = [];
    const queue = new LatestLayerRulesQueue(
      async (_projectPath, patch) => new Promise<LayerMutationResult>((resolve) => {
        calls.push(patch.layers?.[0]?.id ?? "");
        resolvers.push(resolve);
      }),
    );

    const first = queue.enqueue("/project", { expectedRevision: "revision-1", layers: [{ id: "a", visible: false }] });
    const second = queue.enqueue("/project", { expectedRevision: "revision-1", layers: [{ id: "b", visible: false }] });
    await Promise.resolve();
    expect(calls).toEqual(["a"]);
    resolvers[0]({ state: layers, history_id: null, changed_files: [] });
    const firstResult = await first;
    await Promise.resolve();
    expect(firstResult.isLatest).toBe(false);
    expect(calls).toEqual(["a", "b"]);

    const latest = { ...layers, layers: [layers.layers[0], { ...layers.layers[1], visible: false }] };
    resolvers[1]({ state: latest, history_id: null, changed_files: [] });
    await expect(second).resolves.toMatchObject({ state: latest, isLatest: true });
  });

  test("reset cancels queued patches and does not leak the old project revision", async () => {
    const first = deferred<LayerMutationResult>();
    const calls: Array<[string, string]> = [];
    const queue = new LatestLayerRulesQueue(async (project, patch) => {
      calls.push([project, patch.expectedRevision]);
      if (calls.length === 1) return first.promise;
      return { state: { ...layers, revision: "b-next" }, history_id: null, changed_files: [] };
    });
    const running = queue.enqueue("/a", { expectedRevision: "a", layers: [] });
    const queued = queue.enqueue("/a", { expectedRevision: "a", layers: [] });
    await Promise.resolve();
    queue.reset();
    const latest = queue.enqueue("/b", { expectedRevision: "b", layers: [] });
    first.resolve({ state: { ...layers, revision: "a-next" }, history_id: null, changed_files: [] });
    await expect(running).resolves.toMatchObject({ isLatest: false });
    await expect(queued).resolves.toMatchObject({ isLatest: false });
    await expect(latest).resolves.toMatchObject({ isLatest: true, state: { revision: "b-next" } });
    expect(calls).toEqual([["/a", "a"], ["/b", "b"]]);
  });

});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve;
  });
  return { promise, resolve };
}
