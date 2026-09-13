/**
 * apps/viewer: verifies selected-entity AI context handoff uses the desktop
 * invoke contract without requiring a Tauri window.
 */
import { expect, test } from "@playwright/test";
import { type AiContextState } from "../../src/artifacts";
import { LatestAiContextWriteQueue } from "../../src/desktop-ai-context";

test("serializes writes, drops superseded work, and publishes selection clearing", async () => {
  const calls: string[] = [];
  const first = deferred<AiContextState>();
  const cleared = deferred<AiContextState>();
  const started = deferred<void>();
  const completed = deferred<AiContextState>();
  const applied: string[] = [];
  const queue = new LatestAiContextWriteQueue(async (_command, args) => {
    const id = String(args.selectedEntityId);
    calls.push(id);
    if (id === "A") return first.promise;
    started.resolve();
    return cleared.promise;
  });
  queue.enqueue(request("A"), () => applied.push("A"), error => { throw error; });
  queue.enqueue(request("B"), () => applied.push("B"), error => { throw error; });
  queue.enqueue(request(""), state => { applied.push("cleared"); completed.resolve(state); }, error => { throw error; });
  expect(calls).toEqual(["A"]);
  first.resolve(readyState("A"));
  await started.promise;
  expect(calls).toEqual(["A", ""]);
  expect(applied).toEqual([]);
  cleared.resolve({ status: "no_entity_selected", message: "no entity selected" });
  await completed.promise;
  expect(applied).toEqual(["cleared"]);
});

function request(selectedEntityId: string) {
  return {
    projectPath: "/tmp/project",
    viewMode: "diff",
    selectedEntityId,
  };
}

function readyState(selectedEntityId: string): AiContextState {
  return {
    status: "ready",
    json_path: `/tmp/project/build/${selectedEntityId}.json`,
    markdown_path: `/tmp/project/build/${selectedEntityId}.md`,
    message: "AI context ready",
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
