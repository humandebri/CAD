/**
 * apps/viewer: verifies selected-entity AI context handoff uses the desktop
 * invoke contract without requiring a Tauri window.
 */
import { expect, test } from "@playwright/test";
import { type AiContextState } from "../../src/artifacts";
import {
  LatestAiContextWriteQueue,
  type InvokeAiContext,
  writeAiContextFromDesktop,
} from "../../src/desktop-ai-context";

test("writes AI context through desktop invoke", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const ready: AiContextState = {
    status: "ready",
    json_path: "/tmp/project/build/ai-context.json",
    markdown_path: "/tmp/project/build/ai-context.md",
    message: "AI context ready",
  };
  const invoke: InvokeAiContext = async (command, args) => {
    calls.push({ command, args });
    return ready;
  };

  const state = await writeAiContextFromDesktop(
    "/tmp/project",
    "diff",
    "ent_01JZ0000000000000000000000",
    invoke,
  );

  expect(calls).toEqual([
    {
      command: "write_ai_context",
      args: {
        projectPath: "/tmp/project",
        viewMode: "diff",
        selectedEntityId: "ent_01JZ0000000000000000000000",
      },
    },
  ]);
  expect(state).toEqual(ready);
});

test("serializes writes and keeps only the latest pending selection", async () => {
  const calls: string[] = [];
  const first = deferred<AiContextState>();
  const latest = deferred<AiContextState>();
  const invoke: InvokeAiContext = async (_command, args) => {
    const selectedEntityId = args.selectedEntityId;
    if (typeof selectedEntityId !== "string") {
      throw new Error("selectedEntityId must be a string");
    }
    calls.push(selectedEntityId);
    return selectedEntityId === "A" ? first.promise : latest.promise;
  };
  const queue = new LatestAiContextWriteQueue(invoke);
  const completed: string[] = [];
  const failed: unknown[] = [];

  queue.enqueue(request("A"), () => completed.push("A"), (error) => failed.push(error));
  queue.enqueue(request("B"), () => completed.push("B"), (error) => failed.push(error));
  queue.enqueue(request("C"), () => completed.push("C"), (error) => failed.push(error));

  expect(calls).toEqual(["A"]);
  first.resolve(readyState("A"));
  await expect.poll(() => calls).toEqual(["A", "C"]);
  latest.resolve(readyState("C"));
  await expect.poll(() => completed).toEqual(["C"]);
  expect(failed).toEqual([]);
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
