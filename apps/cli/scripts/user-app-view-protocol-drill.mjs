#!/usr/bin/env node
// Protocol 443: user-domain view lifecycle/channel, detached approval, and
// unchanged Room views. Synthetic fixtures only; no live accounts or desktop.
import { spawnSync } from "node:child_process";
import { closeSync, mkdirSync, mkdtempSync, openSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const evidence = process.argv[2] ? path.resolve(process.argv[2]) : mkdtempSync(path.join(tmpdir(), "chariox-user-app-drill-"));
if (evidence === root || evidence.startsWith(`${root}${path.sep}`)) throw new Error("Evidence must be outside the checkout");
mkdirSync(evidence, { recursive: true });
const checks = [
  ["display-types", "pnpm", ["--filter", "@chariox/tool-display", "build"]],
  ["client-build", "pnpm", ["--filter", "@chariox/kernel-client", "build"]],
  ["app-views", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "app_view", "--", "--test-threads=1"]],
  ["detached-decisions", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "runtime_interaction_owned_state", "--", "--test-threads=1"]],
  ["kernel-decisions", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "kernel_operation_interaction", "--", "--test-threads=1"]],
  ["critical-passkey", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "critical_approval_passkey", "--", "--test-threads=1"]],
  ["client-conformance", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "client_protocol_conformance", "--", "--test-threads=1"]],
  ["room-open", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "app_open", "--", "--test-threads=1"]],
  ["protocol", "slot-run", ["env", "RUSTC_WRAPPER=", "cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "local::api::tests::protocol_shapes", "--", "--test-threads=1"]],
  ["client-requests", "node", ["--test", "packages/kernel-client/dist/ipc-app-requests.test.js"]],
  ["room-bridge", "node", ["--test", "apps/kernel/slice-linux-docker/docker/browser-controller-apps.test.mjs"]],
  ["sdk-protocol", "node", ["--test", "packages/app-sdk/test/protocol.test.js"]],
];
const results = [];
for (const [name, command, args] of checks) {
  const log = openSync(path.join(evidence, `${name}.log`), "w");
  let result;
  try {
    result = spawnSync(command, args, { cwd: root, stdio: ["ignore", log, log],
      env: { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? path.join(tmpdir(), "chariox-user-app-cargo-target"),
        RUST_TEST_THREADS: "1" } });
    if (result.error) writeFileSync(log, String(result.error));
  } finally {
    closeSync(log);
  }
  results.push({ name, command: [command, ...args], exitCode: result.status });
  writeFileSync(path.join(evidence, "results.json"), JSON.stringify({ protocol: 443,
    scope: "kernel dispatcher, real fixed App ABI worker, durable approval boundary, synthetic passkey; no native browser/frontend rendering claim", results }, null, 2));
  console.log(`MD-APP ${name}: ${result.status === 0 ? "PASS" : "FAIL"}`);
  if (result.status !== 0) process.exit(1);
}
console.log(`Evidence: ${evidence}`);
