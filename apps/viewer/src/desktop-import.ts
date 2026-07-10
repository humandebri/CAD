/**
 * apps/viewer: desktop JWW import transport and output path derivation.
 * The UI chooses a parent folder; the importer creates a new project directory inside it.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { type ProjectState } from "./artifacts";

export type InvokeImport = (
  command: string,
  args: Record<string, unknown>,
) => Promise<ProjectState>;

const invokeImport: InvokeImport = (command, args) => tauriInvoke<ProjectState>(command, args);

export async function importJwwFromDesktop(
  jwwPath: string,
  parentDir: string,
  invokeCommand: InvokeImport = invokeImport,
): Promise<ProjectState> {
  return invokeCommand("import_jww", {
    jwwPath,
    outDir: importedProjectPath(jwwPath, parentDir),
  });
}

export function importedProjectPath(jwwPath: string, parentDir: string): string {
  const stem = sanitizeProjectStem(fileStem(jwwPath));
  const separator = parentDir.includes("\\") ? "\\" : "/";
  const trimmedParent = parentDir.replace(/[\\/]+$/, "");
  return `${trimmedParent}${separator}${stem}_imported`;
}

function fileStem(path: string): string {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? "jww_import";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) {
    return name;
  }
  return name.slice(0, dot);
}

function sanitizeProjectStem(value: string): string {
  const replaced = value
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return replaced.length === 0 ? "jww_import" : replaced;
}
