import { cpSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const viewerRoot = resolve(import.meta.dirname, "..");
const workspaceRoot = resolve(viewerRoot, "../..");
const runRoot = mkdtempSync(resolve(tmpdir(), "cad-desktop-e2e-"));
const projectPath = resolve(runRoot, "project");
const pdfPath = resolve(runRoot, "desktop-smoke.pdf");
cpSync(resolve(workspaceRoot, "examples/house-small"), projectPath, { recursive: true });

const env = {
  ...process.env,
  VITE_CAD_DESKTOP_E2E: "1",
  VITE_CAD_E2E_PROJECT_PATH: projectPath,
  VITE_CAD_E2E_PDF_PATH: pdfPath,
  CAD_E2E_PROJECT_PATH: projectPath,
  CAD_E2E_PDF_PATH: pdfPath,
  CAD_E2E_BINARY_PATH: resolve(workspaceRoot, "target/debug/cad-desktop"),
};

function run(command, args) {
  const result = spawnSync(command, args, { cwd: viewerRoot, env, stdio: "inherit" });
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status ?? 1}`);
}

let succeeded = false;
try {
  run("pnpm", ["exec", "tauri", "build", "--debug", "--no-bundle", "--features", "desktop-e2e", "--config", "src-tauri/tauri.e2e.conf.json"]);
  run("pnpm", ["exec", "wdio", "run", "wdio.desktop.conf.mjs"]);
  succeeded = true;
} finally {
  if (!succeeded) {
    const artifactRoot = resolve(viewerRoot, "test-results/desktop-e2e");
    rmSync(artifactRoot, { recursive: true, force: true });
    mkdirSync(artifactRoot, { recursive: true });
    cpSync(runRoot, artifactRoot, { recursive: true });
  }
  if (process.env.CAD_E2E_KEEP_ARTIFACTS === "1") {
    process.stderr.write(`desktop E2E artifacts: ${runRoot}\n`);
  } else {
    rmSync(runRoot, { recursive: true, force: true });
  }
}
