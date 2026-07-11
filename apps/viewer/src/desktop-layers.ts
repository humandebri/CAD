import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  ExportReport,
  LayerRulesPatch,
  LayerWorkspaceState,
} from "./artifacts";

export type InvokeDesktop = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

const invokeDesktop: InvokeDesktop = (command, args) => tauriInvoke(command, args);

export type LayerRulesUpdateResult =
  | { state: LayerWorkspaceState; error?: never; isLatest: boolean }
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
    ) => Promise<LayerWorkspaceState> = updateLayerRulesFromDesktop,
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
        const state = await this.update(projectPath, { ...patch, expectedRevision });
        const isLatest = generation === this.generation && revision === this.revision;
        if (generation === this.generation) {
          this.persistedRevision = state.revision;
        }
        return { state, isLatest };
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
): Promise<LayerWorkspaceState> {
  return invokeCommand<LayerWorkspaceState>("update_layer_rules", {
    projectPath,
    patch: {
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
