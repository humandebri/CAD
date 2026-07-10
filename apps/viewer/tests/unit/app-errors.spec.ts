/**
 * apps/viewer: verifies UI error formatting for Tauri and browser failures.
 */
import { expect, test } from "@playwright/test";
import { formatError } from "../../src/app-errors";

test("keeps string errors returned by Tauri invoke", () => {
  expect(formatError("failed to load project", "fallback")).toBe("failed to load project");
});

test("uses Error messages and fallback for unknown values", () => {
  expect(formatError(new Error("network failed"), "fallback")).toBe("network failed");
  expect(formatError(null, "fallback")).toBe("fallback");
  expect(formatError("", "fallback")).toBe("fallback");
});
