/**
 * apps/viewer: verifies JWW import path derivation and Tauri invoke wiring.
 */
import { expect, test } from "@playwright/test";
import { importedProjectPath, importJwwFromDesktop, type InvokeImport } from "../../src/desktop-import";

test("derives a new imported project path from selected parent folder", () => {
  expect(importedProjectPath("/tmp/木造平面例.jww", "/Users/me/CAD")).toBe(
    "/Users/me/CAD/jww_import_imported",
  );
  expect(importedProjectPath("/tmp/Test1.jww", "/Users/me/CAD/")).toBe(
    "/Users/me/CAD/test1_imported",
  );
});

test("invokes import_jww with derived output directory", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const invoke: InvokeImport = async (command, args) => {
    calls.push({ command, args });
    return {
      project_path: "/Users/me/CAD/test1_imported",
      project_name: "test1",
      is_git_project: false,
      import_warning_count: 3,
    };
  };

  const state = await importJwwFromDesktop("/tmp/Test1.jww", "/Users/me/CAD", invoke);

  expect(calls).toEqual([
    {
      command: "import_jww",
      args: {
        jwwPath: "/tmp/Test1.jww",
        outDir: "/Users/me/CAD/test1_imported",
      },
    },
  ]);
  expect(state.import_warning_count).toBe(3);
});
