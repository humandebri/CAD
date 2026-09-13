import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// ci-local builds cadc before browser tests. Poppler is required for PDF image checks.
test("acceptance drawing SVG and PDF retain their reviewed appearance", async ({ page }, testInfo) => {
  test.skip(process.platform !== "darwin", "The reviewed raster/font baseline is macOS-specific; structural output tests run on every platform.");
  test.setTimeout(120_000);
  const root = resolve(import.meta.dirname, "../../../..");
  const cadc = resolve(root, "target/debug/cadc");
  const project = resolve(root, "examples/cad-acceptance");
  const svgPath = testInfo.outputPath("acceptance.svg");
  const pdfPath = testInfo.outputPath("acceptance.pdf");
  execFileSync(cadc, ["check", project, "--target", "cad", "--format", "json", "--out", "-"], { stdio: "pipe" });
  execFileSync(cadc, ["render", project, "--format", "svg", "--out", svgPath], { stdio: "pipe" });
  execFileSync(cadc, ["export-pdf", project, "--drawing", "acceptance", "--out", pdfPath], { stdio: "pipe" });
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.setContent(`<style>body{margin:0;background:white}svg{width:1200px;height:850px;display:block}</style>${readFileSync(svgPath, "utf8")}`);
  await page.evaluate(() => document.fonts.ready);
  await expect(page.locator("svg")).toHaveScreenshot("acceptance-svg.png", { maxDiffPixelRatio: 0.001 });
  const imagePrefix = testInfo.outputPath("acceptance-pdf");
  execFileSync("pdftoppm", ["-f", "1", "-singlefile", "-scale-to", "1200", "-png", pdfPath, imagePrefix], { stdio: "pipe" });
  expect(readFileSync(`${imagePrefix}.png`)).toMatchSnapshot("acceptance-pdf.png", { maxDiffPixelRatio: 0.001 });
});
