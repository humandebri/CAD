import { expect, test } from "@playwright/test";
import {
  exportDrawingPdfFromDesktop,
  exportJwwFromDesktop,
  LatestLayerRulesQueue,
  PdfExportGuard,
  updateLayerRulesFromDesktop,
  withLayerVisibility,
} from "../../src/desktop-layers";
import type { LayerWorkspaceState } from "../../src/artifacts";

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
  test("uses the Tauri layer command contract", async () => {
    const calls: unknown[][] = [];
    const invoke = async <T>(command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      return layers as T;
    };
    await updateLayerRulesFromDesktop("/project", { expectedRevision: "revision-1", layers: [{ id: "a", visible: false }] }, invoke);
    expect(calls).toEqual([["update_layer_rules", {
      projectPath: "/project",
      patch: { layers: [{ id: "a", visible: false }], groups: [], active_layer: undefined, expected_revision: "revision-1" },
    }]]);
  });

  test("updates visibility without mutating the source state", () => {
    const updated = withLayerVisibility(layers, new Map([["a", false]]));
    expect(updated.layers[0].visible).toBe(false);
    expect(layers.layers[0].visible).toBe(true);
  });

  test("serializes patches and discards the older complete snapshot", async () => {
    const calls: string[] = [];
    const resolvers: Array<(state: LayerWorkspaceState) => void> = [];
    const queue = new LatestLayerRulesQueue(
      async (_projectPath, patch) => new Promise<LayerWorkspaceState>((resolve) => {
        calls.push(patch.layers?.[0]?.id ?? "");
        resolvers.push(resolve);
      }),
    );

    const first = queue.enqueue("/project", { expectedRevision: "revision-1", layers: [{ id: "a", visible: false }] });
    const second = queue.enqueue("/project", { expectedRevision: "revision-1", layers: [{ id: "b", visible: false }] });
    await Promise.resolve();
    expect(calls).toEqual(["a"]);
    resolvers[0](layers);
    const firstResult = await first;
    await Promise.resolve();
    expect(firstResult.isLatest).toBe(false);
    expect(calls).toEqual(["a", "b"]);

    const latest = withLayerVisibility(layers, new Map([["b", false]]));
    resolvers[1](latest);
    await expect(second).resolves.toEqual({ state: latest, isLatest: true });
  });

  test("reset prevents queued updates from invoking the backend", async () => {
    const calls: string[] = [];
    const first = deferred<LayerWorkspaceState>();
    const queue = new LatestLayerRulesQueue(async (projectPath) => {
      calls.push(projectPath);
      return first.promise;
    });

    const running = queue.enqueue("/project-a", { expectedRevision: "revision-1", layers: [] });
    const queued = queue.enqueue("/project-a", { expectedRevision: "revision-1", layers: [] });
    await Promise.resolve();
    queue.reset();
    first.resolve({ ...layers, revision: "revision-2" });

    await expect(running).resolves.toMatchObject({ isLatest: false });
    await expect(queued).resolves.toMatchObject({ isLatest: false });
    expect(calls).toEqual(["/project-a"]);
  });

  test("a reset in flight does not seed the next project revision", async () => {
    const first = deferred<LayerWorkspaceState>();
    const revisions: string[] = [];
    const queue = new LatestLayerRulesQueue(async (_projectPath, patch) => {
      revisions.push(patch.expectedRevision);
      if (revisions.length === 1) return first.promise;
      return { ...layers, revision: "project-b-revision-2" };
    });

    const old = queue.enqueue("/project-a", { expectedRevision: "project-a-revision", layers: [] });
    await Promise.resolve();
    queue.reset();
    first.resolve({ ...layers, revision: "project-a-revision-2" });
    await old;
    await queue.enqueue("/project-b", { expectedRevision: "project-b-revision", layers: [] });

    expect(revisions).toEqual(["project-a-revision", "project-b-revision"]);
  });

  test("uses the experimental export command contract", async () => {
    const calls: unknown[][] = [];
    const invoke = async <T>(command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      return { status: "exported" } as T;
    };
    await exportJwwFromDesktop("/project", "plan", "/tmp/plan.jww", false, true, invoke);
    expect(calls).toEqual([["export_jww", {
      projectPath: "/project",
      drawing: "plan",
      outputPath: "/tmp/plan.jww",
      allowLossy: false,
      overwrite: true,
    }]]);
  });

  test("uses the layout-aware PDF export command contract", async () => {
    const calls: unknown[][] = [];
    const invoke = async <T>(command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      return undefined as T;
    };
    await exportDrawingPdfFromDesktop({
      projectPath: "/project",
      drawing: "plan",
      layout: "default",
      outputPath: "/tmp/plan.pdf",
      overwrite: false,
    }, invoke);
    expect(calls).toEqual([["export_drawing_pdf", {
      projectPath: "/project",
      drawing: "plan",
        layout: "default",
        outputPath: "/tmp/plan.pdf",
        overwrite: false,
        expectedFiles: [],
      }]]);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve;
  });
  return { promise, resolve };
}
