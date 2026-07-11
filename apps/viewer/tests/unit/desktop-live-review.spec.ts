/** apps/viewer: verifies desktop live review coordination without Tauri. */
import { expect, test } from "@playwright/test";
import {
  type Artifacts,
  type DesktopReviewSnapshot,
  type ProjectState,
  type ProjectWatchEvent,
} from "../../src/artifacts";
import {
  DrawingLoadGuard,
  LatestReviewQueue,
  isRelevantProjectSourcePath,
  mergeProjectStateFromReview,
  sheetSvgContainsEntity,
  shouldPreserveView,
  startProjectWatch,
  startProjectWatchWithCatchUp,
  stopProjectWatch,
  subscribeProjectWatch,
} from "../../src/desktop-live-review";

test("rejects an old drawing result after a newer drawing switch", () => {
  const guard = new DrawingLoadGuard();
  guard.reset("A");
  const loadingB = guard.begin("B");
  const loadingC = guard.begin("C");

  expect(guard.accepts(loadingB, "B")).toBe(false);
  expect(guard.acceptsCurrent("B")).toBe(false);
  expect(guard.accepts(loadingC, "C")).toBe(true);
});

test("watcher results follow the selected drawing and a current failure rolls back", () => {
  const guard = new DrawingLoadGuard();
  guard.reset("A");
  const loadingB = guard.begin("B");

  expect(guard.acceptsCurrent("A")).toBe(false);
  expect(guard.acceptsCurrent("B")).toBe(true);
  expect(guard.rollback(loadingB)).toBe("A");
  expect(guard.acceptsCurrent("A")).toBe(true);
});

test("serializes review requests and applies only the latest result", async () => {
  const calls: string[] = [];
  const first = deferred<DesktopReviewSnapshot>();
  const latest = deferred<DesktopReviewSnapshot>();
  const queue = new LatestReviewQueue(async (projectPath) => {
    calls.push(projectPath);
    return projectPath === "A" ? first.promise : latest.promise;
  });
  const completed: string[] = [];
  const failed: unknown[] = [];

  queue.enqueue("A", (value) => completed.push(value.projectName), (error) => failed.push(error));
  queue.enqueue("B", (value) => completed.push(value.projectName), (error) => failed.push(error));
  queue.enqueue("C", (value) => completed.push(value.projectName), (error) => failed.push(error));

  expect(calls).toEqual(["A"]);
  first.resolve(snapshot("A"));
  await expect.poll(() => calls).toEqual(["A", "C"]);
  latest.resolve(snapshot("C"));
  await expect.poll(() => completed).toEqual(["reviewed-C"]);
  expect(failed).toEqual([]);
});

test("reset invalidates an in-flight result", async () => {
  const pending = deferred<DesktopReviewSnapshot>();
  const queue = new LatestReviewQueue(() => pending.promise);
  const completed: DesktopReviewSnapshot[] = [];

  queue.enqueue("project", (value) => completed.push(value), () => undefined);
  queue.reset();
  pending.resolve(snapshot("stale"));

  await expect.poll(() => completed).toEqual([]);
});

test("starts watching before enqueuing the catch-up review", async () => {
  const calls: string[] = [];
  const watchStarted = deferred<unknown>();
  const invoke = async (command: string) => {
    calls.push(command);
    await watchStarted.promise;
    return { project_path: "/tmp/project", status: "watching", debounce_ms: 250 };
  };
  const catchUp = startProjectWatchWithCatchUp(
    "/tmp/project",
    (projectPath) => calls.push(`catch-up:${projectPath}`),
    invoke,
  );

  await expect.poll(() => calls).toEqual(["start_project_watch"]);
  watchStarted.resolve(undefined);
  await catchUp;

  expect(calls).toEqual(["start_project_watch", "catch-up:/tmp/project"]);
});

test("does not enqueue catch-up when starting the watcher fails", async () => {
  const catchUps: string[] = [];
  const invoke = async () => {
    throw new Error("watch failed");
  };

  await expect(
    startProjectWatchWithCatchUp("/tmp/project", (path) => catchUps.push(path), invoke),
  ).rejects.toThrow("watch failed");
  expect(catchUps).toEqual([]);
});

test("updates reviewed project name while preserving session metadata", () => {
  const state: ProjectState = {
    project_path: "/tmp/project",
    project_name: "old-name",
    is_git_project: true,
    import_warning_count: 3,
  };

  expect(mergeProjectStateFromReview(state, snapshot("latest"))).toEqual({
    project_path: "/tmp/project",
    project_name: "reviewed-latest",
    is_git_project: true,
    import_warning_count: 3,
  });
});

test("watch transport uses the desktop command and event contracts", async () => {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const invoke = async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    return { project_path: "/tmp/project", status: "watching", debounce_ms: 250 };
  };
  const events: ProjectWatchEvent[] = [];
  const listener: { current?: (event: ProjectWatchEvent) => void } = {};
  const unlisten = () => undefined;

  await startProjectWatch("/tmp/project", invoke);
  await subscribeProjectWatch((event) => events.push(event), async (handler) => {
    listener.current = handler;
    return unlisten;
  });
  listener.current?.({
    kind: "changed",
    project_path: "/tmp/project",
    paths: ["drawings/plan/entities.ndjson"],
  });
  await stopProjectWatch(invoke);

  expect(calls).toEqual([
    { command: "start_project_watch", args: { projectPath: "/tmp/project" } },
    { command: "stop_project_watch", args: undefined },
  ]);
  expect(events).toHaveLength(1);
});

test("filters project source paths and ignores generated files", () => {
  expect(isRelevantProjectSourcePath("cad.project.toml")).toBe(true);
  expect(isRelevantProjectSourcePath("rules/styles.toml")).toBe(true);
  expect(isRelevantProjectSourcePath("drawings/plan/entities.ndjson")).toBe(true);
  expect(isRelevantProjectSourcePath("drawings\\plan\\sheet.toml")).toBe(true);
  expect(isRelevantProjectSourcePath("comments/plan.ndjson")).toBe(true);
  expect(isRelevantProjectSourcePath("build/ai-context.json")).toBe(false);
  expect(isRelevantProjectSourcePath(".git/index")).toBe(false);
  expect(isRelevantProjectSourcePath("drawings/plan/entities.ndjson.swp")).toBe(false);
});

test("preserves view only in the same context and checks retained selection", () => {
  const svg = '<svg><g data-entity-id="ent_A"><line /></g></svg>';
  expect(shouldPreserveView("/project:sheet", "/project:sheet")).toBe(true);
  expect(shouldPreserveView("/project:sheet", "/project:diff")).toBe(false);
  expect(shouldPreserveView("/project:sheet", "/other:sheet")).toBe(false);
  expect(sheetSvgContainsEntity(svg, "ent_A")).toBe(true);
  expect(sheetSvgContainsEntity(svg, "ent_B")).toBe(false);
});

function artifacts(id: string): Artifacts {
  return {
    sheetSvg: `<svg data-review="${id}" />`,
    diffSvg: "<svg />",
    check: { schema_version: "0.1", status: "ok", diagnostics: [] },
    diff: { schema_version: "0.2", status: "ok", changes: [], warnings: [], configuration_changes: [] },
    comments: [],
    layers: { revision: "revision", active_layer: null, groups: [], layers: [] },
    drawingNames: ["plan_1f"],
    currentDrawing: "plan_1f",
    editor: { drawing: "plan_1f", revision: id, entities: [], text_styles: [], dimension_styles: [], pens: [] },
  };
}

function snapshot(id: string): DesktopReviewSnapshot {
  return {
    artifacts: artifacts(id),
    projectName: `reviewed-${id}`,
  };
}

function deferred<T>() {
  let resolver: ((value: T) => void) | null = null;
  const promise = new Promise<T>((resolve) => {
    resolver = resolve;
  });
  return {
    promise,
    resolve(value: T) {
      if (resolver === null) {
        throw new Error("deferred resolver is not initialized");
      }
      resolver(value);
    },
  };
}
