#!/usr/bin/env node
// Focused protocol-400 drill; no live kernel, provider, credentials or desktop.
// Build kernel-client and CLI first. Rust commands always enter builder slot-run.
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const evidence = process.argv[2] ? path.resolve(process.argv[2]) : mkdtempSync(path.join(tmpdir(), "chariox-app-host-drill-"));
if (evidence === root || evidence.startsWith(`${root}${path.sep}`)) throw new Error("Evidence must be outside the checkout");
mkdirSync(evidence, { recursive: true });
const checks = [
  ["kernel-host", "slot-run", ["cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "app_host", "--", "--nocapture"]],
  ["protocol-shapes", "slot-run", ["cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "local::api::tests::protocol_shapes", "--", "--nocapture"]],
  ["kernel-registry", "slot-run", ["cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "runtime_interaction_owned_state", "--", "--nocapture"]],
  ["kernel-decisions", "slot-run", ["cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "kernel_operation_interactions", "--", "--nocapture"]],
  ["worker-broker", "slot-run", ["cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "app_files_broker::tests", "--", "--nocapture"]],
  ["client-protocol", "slot-run", ["cargo", "+1.88.0", "test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", "client_protocol_conformance", "--", "--nocapture"]],
  ["client-app", "node", ["--test", "packages/kernel-client/dist/ipc-app-requests.test.js"]],
  ["cli-host", "node", ["--test", "apps/cli/dist/app-host-action.test.js", "apps/cli/dist/app-command-handler.test.js", "apps/cli/dist/kernel-approval-controller.test.js", "apps/cli/dist/cli-workflow-action-routing-composition.test.js"]],
  ["tui-render", "bun", ["test", "./apps/cli/dist/kernel-approval-renderer.bun-test.js"]],
  ["view-bridge", "node", ["--test", "apps/kernel/slice-linux-docker/docker/browser-controller-apps.test.mjs"]],
  ["sdk-host", "node", ["--test", "packages/app-sdk/test/sdk.test.js"]],
];
const results = [];
for (const [name, command, args] of checks) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
    env: { ...process.env, RUST_MIN_STACK: process.env.RUST_MIN_STACK ?? "16777216", RUST_TEST_THREADS: "1" } });
  writeFileSync(path.join(evidence, `${name}.log`), `${result.stdout ?? ""}${result.stderr ?? ""}${result.error ?? ""}`);
  results.push({ name, command: [command, ...args], exitCode: result.status });
  writeFileSync(path.join(evidence, "results.json"), JSON.stringify({ protocol: 400, scope: "worker SDK, kernel interaction, shared client, trusted TUI, Room bridge; no live web or OS clipboard proof", results }, null, 2));
  console.log(`${name}: ${result.status === 0 ? "PASS" : "FAIL"}`);
  if (result.status !== 0) process.exit(1);
}
console.log(`Evidence: ${evidence}`);
