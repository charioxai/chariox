#!/usr/bin/env node
// MP-08/MP-10: namespace transport for the opt-in headed kernel test. The
// supervisor owns this host process; the real controller owns the container
// PID. Translate only that process identity, preserving every browser RPC.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { appendFileSync } from "node:fs";
import readline from "node:readline";
import path from "node:path";
const realDocker = process.env.CHARIOX_CONCURRENCY_REAL_DOCKER;
assert.ok(realDocker && path.isAbsolute(realDocker));
const child = spawn(realDocker, process.argv.slice(2), { stdio: ["pipe", "pipe", "inherit"] });
process.stdin.pipe(child.stdin);
const lines = readline.createInterface({ input: child.stdout });
lines.on("line", line => {
  const response = JSON.parse(line);
  if (response.result && Object.hasOwn(response.result, "process_id") && response.result.process_id !== null) {
    const controllerPid = response.result.process_id;
    response.result.process_id = process.pid;
    const receipt = process.env.CHARIOX_CONCURRENCY_TRANSPORT_RECEIPT;
    if (receipt) appendFileSync(receipt, JSON.stringify({ mp_items: ["MP-08", "MP-10"], atMs: Date.now(),
      hostTransportPid: process.pid, dockerClientPid: child.pid, controllerContainerPid: controllerPid }) + "\n", { mode: 0o600 });
  }
  process.stdout.write(JSON.stringify(response) + "\n");
});
child.once("error", error => { process.stderr.write(error.message + "\n"); process.exitCode = 1; });
child.once("exit", code => { process.stdin.unpipe(child.stdin); process.stdin.pause(); lines.close(); process.exitCode = code ?? 1; });
