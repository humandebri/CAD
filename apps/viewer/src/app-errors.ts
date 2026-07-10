/**
 * apps/viewer: small UI error helpers for browser and Tauri invoke failures.
 * Tauri command rejections commonly arrive as strings, so the UI must not drop them.
 */

export function formatError(error: unknown, fallback: string): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string" && error.trim().length > 0) {
    return error;
  }
  return fallback;
}
