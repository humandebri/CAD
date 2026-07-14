import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  ExportReport,
  PdfExportRequest,
  LayerRulesPatch,
  LayerMutationResult,
  LayerWorkspaceState,
} from "./artifacts";

export type InvokeDesktop = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

const invokeDesktop: InvokeDesktop = (command, args) => tauriInvoke(command, args);

/** Invalidates an in-flight PDF request when its drawing/source context changes. */
export class PdfExportGuard {
  private sequence = 0;

  begin(): number {
    return ++this.sequence;
  }

  invalidate(): void {
    this.sequence += 1;
  }

  accepts(sequence: number): boolean {
    return sequence === this.sequence;
  }
}

export type LayerRulesUpdateResult =
  | {
      state: LayerWorkspaceState;
      history_id?: string | null;
      changed_files?: string[];
      error?: never;
      isLatest: boolean;
    }
  | { state?: never; error: unknown; isLatest: boolean };

/** Serializes file-backed patches while allowing the UI to ignore superseded responses. */
export class LatestLayerRulesQueue {
  private tail: Promise<void> = Promise.resolve();
  private revision = 0;
  private generation = 0;
  private persistedRevision: string | null = null;

  constructor(
    private readonly update: (
      projectPath: string,
      patch: LayerRulesPatch,
    ) => Promise<LayerMutationResult | LayerWorkspaceState> = updateLayerRulesFromDesktop,
  ) {}

  enqueue(projectPath: string, patch: LayerRulesPatch): Promise<LayerRulesUpdateResult> {
    const revision = ++this.revision;
    const generation = this.generation;
    const operation = this.tail.then(async (): Promise<LayerRulesUpdateResult> => {
      if (generation !== this.generation) {
        return { error: new Error("layer update was cancelled by reset"), isLatest: false };
      }
      try {
        const expectedRevision = this.persistedRevision ?? patch.expectedRevision;
        if (expectedRevision === undefined) {
          throw new Error("layer rules revision is required");
        }
        const mutation = await this.update(projectPath, { ...patch, expectedRevision });
        const state = "state" in mutation ? mutation.state : mutation;
        const isLatest = generation === this.generation && revision === this.revision;
        if (generation === this.generation) {
          this.persistedRevision = state.revision;
        }
        return "state" in mutation
          ? { state, history_id: mutation.history_id, changed_files: mutation.changed_files, isLatest }
          : { state, isLatest };
      } catch (error: unknown) {
        return {
          error,
          isLatest: generation === this.generation && revision === this.revision,
        };
      }
    });
    this.tail = operation.then(() => undefined);
    return operation;
  }

  reset(): void {
    this.generation += 1;
    this.revision += 1;
    this.persistedRevision = null;
  }
}

export function updateLayerRulesFromDesktop(
  projectPath: string,
  patch: LayerRulesPatch,
  invokeCommand: InvokeDesktop = invokeDesktop,
): Promise<LayerMutationResult> {
  return invokeCommand<LayerMutationResult>("update_layer_rules", {
    projectPath,
    patch: {
      drawing: patch.drawing,
      layers: patch.layers ?? [],
      groups: patch.groups ?? [],
      active_layer: patch.activeLayer,
      expected_revision: patch.expectedRevision,
    },
  });
}

export function exportJwwFromDesktop(
  projectPath: string,
  drawing: string,
  outputPath: string,
  allowLossy: boolean,
  overwrite: boolean,
  invokeCommand: InvokeDesktop = invokeDesktop,
): Promise<ExportReport> {
  return invokeCommand<ExportReport>("export_jww", {
    projectPath,
    drawing,
    outputPath,
    allowLossy,
    overwrite,
  });
}

export function exportDrawingPdfFromDesktop(
  request: PdfExportRequest,
  invokeCommand: InvokeDesktop = invokeDesktop,
): Promise<void> {
  return invokeCommand<void>("export_drawing_pdf", {
    projectPath: request.projectPath,
    drawing: request.drawing,
    layout: request.layout ?? null,
    outputPath: request.outputPath,
    overwrite: request.overwrite,
    expectedFiles: request.expectedFiles ?? [],
  });
}

export function withLayerVisibility(
  state: LayerWorkspaceState,
  updates: ReadonlyMap<string, boolean>,
): LayerWorkspaceState {
  return {
    ...state,
    layers: state.layers.map((layer) =>
      updates.has(layer.id) ? { ...layer, visible: updates.get(layer.id) ?? layer.visible } : layer,
    ),
  };
}
