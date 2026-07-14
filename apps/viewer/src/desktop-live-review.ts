/**
 * apps/viewer: desktop source watching and latest-only review coordination.
 * Filesystem access stays in Rust; this module only handles Tauri events and
 * serializes the existing run_review transport.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import {
  type DesktopReviewSnapshot,
  type ProjectState,
  type ProjectWatchEvent,
  type ProjectWatchState,
} from "./artifacts";

export const PROJECT_WATCH_EVENT = "cad-project-watch";

/** Prevents late project-open success or failure results from changing state. */
export class ProjectOpenGuard {
  private sequence = 0;

  begin(): number {
    return ++this.sequence;
  }

  accepts(sequence: number): boolean {
    return sequence === this.sequence;
  }
}

/** Serializes persisted side effects and drops work whose generation is stale. */
export class LatestProjectPersistenceQueue {
  private tail: Promise<void> = Promise.resolve();

  enqueue(
    sequence: number,
    accepts: (sequence: number) => boolean,
    persist: () => Promise<void>,
  ): Promise<void> {
    const run = this.tail.catch(() => undefined).then(async () => {
      if (!accepts(sequence)) return;
      await persist();
    });
    this.tail = run.catch(() => undefined);
    return run;
  }
}

export type DrawingLoadToken = {
  sequence: number;
  drawing: string;
  previousDrawing: string | null;
};

/** Coordinates direct drawing switches with asynchronous live-review results. */
export class DrawingLoadGuard {
  private sequence = 0;
  private drawing: string | null = null;

  reset(drawing: string | null): void {
    this.sequence += 1;
    this.drawing = drawing;
  }

  begin(drawing: string): DrawingLoadToken {
    const token = {
      sequence: ++this.sequence,
      drawing,
      previousDrawing: this.drawing,
    };
    this.drawing = drawing;
    return token;
  }

  accepts(token: DrawingLoadToken, loadedDrawing: string): boolean {
    return (
      token.sequence === this.sequence &&
      token.drawing === this.drawing &&
      loadedDrawing === this.drawing
    );
  }

  acceptsCurrent(loadedDrawing: string): boolean {
    return loadedDrawing === this.drawing;
  }

  rollback(token: DrawingLoadToken): string | null | undefined {
    if (token.sequence !== this.sequence || token.drawing !== this.drawing) {
      return undefined;
    }
    this.sequence += 1;
    this.drawing = token.previousDrawing;
    return this.drawing;
  }
}

export type InvokeProjectWatch = (
  command: string,
  args?: Record<string, unknown>,
) => Promise<unknown>;

export type ListenProjectWatch = (
  handler: (event: ProjectWatchEvent) => void,
) => Promise<() => void>;

export type LoadDesktopReview = (projectPath: string) => Promise<DesktopReviewSnapshot>;

const invokeProjectWatch: InvokeProjectWatch = (command, args) => tauriInvoke(command, args);

const listenProjectWatch: ListenProjectWatch = (handler) =>
  tauriListen<ProjectWatchEvent>(PROJECT_WATCH_EVENT, (event) => handler(event.payload));

export async function startProjectWatch(
  projectPath: string,
  invokeCommand: InvokeProjectWatch = invokeProjectWatch,
): Promise<ProjectWatchState> {
  return invokeCommand("start_project_watch", { projectPath }) as Promise<ProjectWatchState>;
}

export async function startProjectWatchWithCatchUp(
  projectPath: string,
  enqueueCatchUp: (projectPath: string) => void,
  invokeCommand: InvokeProjectWatch = invokeProjectWatch,
): Promise<ProjectWatchState> {
  const state = await startProjectWatch(projectPath, invokeCommand);
  enqueueCatchUp(projectPath);
  return state;
}

export async function stopProjectWatch(
  invokeCommand: InvokeProjectWatch = invokeProjectWatch,
): Promise<void> {
  await invokeCommand("stop_project_watch");
}

export function subscribeProjectWatch(
  handler: (event: ProjectWatchEvent) => void,
  listenCommand: ListenProjectWatch = listenProjectWatch,
): Promise<() => void> {
  return listenCommand(handler);
}

type QueuedReview = {
  sequence: number;
  projectPath: string;
  onSuccess: (snapshot: DesktopReviewSnapshot) => void;
  onError: (error: unknown) => void;
};

/** Serializes run_review calls and retains only the newest pending request. */
export class LatestReviewQueue {
  private sequence = 0;
  private running = false;
  private pending: QueuedReview | null = null;

  constructor(private readonly loadReview: LoadDesktopReview) {}

  enqueue(
    projectPath: string,
    onSuccess: (snapshot: DesktopReviewSnapshot) => void,
    onError: (error: unknown) => void,
  ): void {
    this.sequence += 1;
    this.pending = {
      sequence: this.sequence,
      projectPath,
      onSuccess,
      onError,
    };
    this.start();
  }

  reset(): void {
    this.sequence += 1;
    this.pending = null;
  }

  private start(): void {
    if (this.running) {
      return;
    }
    this.running = true;
    void this.drain();
  }

  private async drain(): Promise<void> {
    while (this.pending !== null) {
      const current = this.pending;
      this.pending = null;
      try {
        const snapshot = await this.loadReview(current.projectPath);
        if (current.sequence === this.sequence) {
          current.onSuccess(snapshot);
        }
      } catch (error: unknown) {
        if (current.sequence === this.sequence) {
          current.onError(error);
        }
      }
    }
    this.running = false;
    if (this.pending !== null) {
      this.start();
    }
  }
}

export function mergeProjectStateFromReview(
  current: ProjectState,
  snapshot: DesktopReviewSnapshot,
): ProjectState {
  return {
    ...current,
    project_name: snapshot.projectName,
  };
}


export function sheetSvgContainsEntity(sheetSvg: string, entityId: string): boolean {
  if (entityId === "") {
    return false;
  }
  return (
    sheetSvg.includes(`data-entity-id="${entityId}"`) ||
    sheetSvg.includes(`data-entity-id='${entityId}'`)
  );
}

export function shouldPreserveView(previousContext: string | null, nextContext: string): boolean {
  return previousContext === nextContext;
}
