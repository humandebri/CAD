export const config = {
  runner: "local",
  specs: ["./tests/desktop/**/*.e2e.mjs"],
  maxInstances: 1,
  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { timeout: 120_000 },
  capabilities: [{ browserName: "tauri" }],
  services: [["tauri", {
    appBinaryPath: process.env.CAD_E2E_BINARY_PATH,
    driverProvider: "embedded",
  }]],
};
