/**
 * apps/viewer: verifies JWW import path derivation and Tauri invoke wiring.
 */
import { expect, test } from "@playwright/test";
import { importedProjectPath } from "../../src/desktop-import";

test("derives a new imported project path from selected parent folder", () => {
  expect(importedProjectPath("/tmp/木造平面例.jww", "/Users/me/CAD")).toBe(
    "/Users/me/CAD/jww_import_imported",
  );
  expect(importedProjectPath("/tmp/Test1.jww", "/Users/me/CAD/")).toBe(
    "/Users/me/CAD/test1_imported",
  );
});
