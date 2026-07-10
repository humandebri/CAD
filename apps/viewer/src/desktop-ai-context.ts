/**
 * apps/viewer: desktop AI context transport writes the selected CAD entity
 * handoff files through Tauri without adding an HTTP API.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { type AiContextState } from "./artifacts";

export type InvokeAiContext = (
  command: string,
  args: Record<string, unknown>,
) => Promise<AiContextState>;

const invokeAiContext: InvokeAiContext = (command, args) =>
  tauriInvoke<AiContextState>(command, args);

export async function writeAiContextFromDesktop(
  projectPath: string,
  viewMode: string,
  selectedEntityId: string,
  invokeCommand: InvokeAiContext = invokeAiContext,
): Promise<AiContextState> {
  return invokeCommand("write_ai_context", {
    projectPath,
    viewMode,
    selectedEntityId,
  });
}

export type AiContextWriteRequest = {
  projectPath: string;
  viewMode: string;
  selectedEntityId: string;
};

type QueuedWrite = {
  sequence: number;
  request: AiContextWriteRequest;
  onSuccess: (state: AiContextState) => void;
  onError: (error: unknown) => void;
};

/**
 * Serializes desktop writes and retains only the newest pending selection.
 * This guarantees that an older invoke cannot finish after a newer invoke and
 * leave the shared ai-context files pointing at the wrong entity.
 */
export class LatestAiContextWriteQueue {
  private sequence = 0;
  private running = false;
  private pending: QueuedWrite | null = null;

  constructor(private readonly invokeCommand: InvokeAiContext = invokeAiContext) {}

  enqueue(
    request: AiContextWriteRequest,
    onSuccess: (state: AiContextState) => void,
    onError: (error: unknown) => void,
  ): () => void {
    this.sequence += 1;
    const sequence = this.sequence;
    this.pending = { sequence, request, onSuccess, onError };
    this.start();
    return () => {
      if (this.sequence === sequence) {
        this.sequence += 1;
        if (this.pending?.sequence === sequence) {
          this.pending = null;
        }
      }
    };
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
        const state = await writeAiContextFromDesktop(
          current.request.projectPath,
          current.request.viewMode,
          current.request.selectedEntityId,
          this.invokeCommand,
        );
        if (current.sequence === this.sequence) {
          current.onSuccess(state);
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
