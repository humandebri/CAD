/** apps/viewer: verifies desktop live review coordination without Tauri. */
import { expect, test } from "@playwright/test";
import { type Artifacts, type DesktopReviewSnapshot, type ProjectState } from "../../src/artifacts";
import { DrawingLoadGuard, LatestProjectPersistenceQueue, LatestReviewQueue, ProjectOpenGuard, mergeProjectStateFromReview, shouldPreserveView, startProjectWatchWithCatchUp } from "../../src/desktop-live-review";

test("ignores startup open failures after a newer project open begins", () => {
  const guard = new ProjectOpenGuard();
  const startup = guard.begin();
  const manual = guard.begin();

  expect(guard.accepts(startup)).toBe(false);
  expect(guard.accepts(manual)).toBe(true);
});

test("persists the newest project after an older save finishes late", async () => {
  const guard = new ProjectOpenGuard();
  const queue = new LatestProjectPersistenceQueue();
  const oldStarted = deferred<void>();
  const oldSave = deferred<void>();
  let stored = "";
  const oldSequence = guard.begin();
  const old = queue.enqueue(oldSequence, (sequence) => guard.accepts(sequence), async () => {
    oldStarted.resolve();
    await oldSave.promise;
    stored = "old";
  });
  await oldStarted.promise;
  const latestSequence = guard.begin();
  const latest = queue.enqueue(latestSequence, (sequence) => guard.accepts(sequence), async () => {
    stored = "latest";
  });

  oldSave.resolve();
  await Promise.all([old, latest]);

  expect(stored).toBe("latest");
});

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
    drawings: ["plan"],
    is_git_project: true,
    editable: true,
    import_warning_count: 3,
  };

  expect(mergeProjectStateFromReview(state, snapshot("latest"))).toEqual({
    project_path: "/tmp/project",
    project_name: "reviewed-latest",
    drawings: ["plan"],
    is_git_project: true,
    editable: true,
    import_warning_count: 3,
  });
});

test("preserves view only in the same context and checks retained selection", () => {
  expect(shouldPreserveView("/project:sheet", "/project:sheet")).toBe(true);
  expect(shouldPreserveView("/project:sheet", "/project:diff")).toBe(false);
  expect(shouldPreserveView("/project:sheet", "/other:sheet")).toBe(false);
});

function artifacts(id: string): Artifacts {
  return {
    sheetSvg: `<svg data-review="${id}" />`,
    diffSvg: "<svg />",
    check: { schema_version: "0.3", status: "ok", diagnostics: [] },
    diff: { schema_version: "0.3", status: "ok", changes: [], warnings: [], configuration_changes: [] },
    comments: [],
    commentsRevision: "comments-revision",
    layers: { revision: "revision", active_layer: null, groups: [], layers: [] },
    drawingNames: ["plan_1f"],
    currentDrawing: "plan_1f",
    editor: { drawing: "plan_1f", revision: id, entities: [], text_styles: [], dimension_styles: [], pens: [], fills: [] },
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
