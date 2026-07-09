/**
 * apps/viewer: Vite serves the sample project as static review artifacts.
 * /build/* and /comments/* come from examples/house-small without committing generated files.
 */
import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
  publicDir: "../../examples/house-small",
});
