import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import readline from "node:readline";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { handleBrowserControllerRequest } from "./browser-controller.mjs";

test("controller health is request-correlated and process-owned", () => {
  assert.deepEqual(handleBrowserControllerRequest({ id: 7, method: "health" }, 41), {
    id: 7,
    ok: true,
    result: {
      state: "ready",
      process_id: 41,
      diagnostic_code: null,
    },
  });
});

test("controller rejects invalid and unknown requests", () => {
  assert.equal(handleBrowserControllerRequest({ id: 1, method: "missing" }).ok, false);
  assert.equal(handleBrowserControllerRequest({ method: "health" }).error.code, "invalid_request");
});

test("stdio controller stays private to its owning process and shuts down cleanly", async (context) => {
  const child = spawn(
    process.execPath,
    [fileURLToPath(new URL("./browser-controller.mjs", import.meta.url)), "stdio"],
    { stdio: ["pipe", "pipe", "pipe"] },
  );
  const responses = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
  const iterator = responses[Symbol.asyncIterator]();
  context.after(() => {
    responses.close();
    child.stdin.destroy();
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
    }
  });

  child.stdin.write(`${JSON.stringify({ id: 1, method: "health" })}\n`);
  const health = JSON.parse((await iterator.next()).value);
  assert.equal(health.id, 1);
  assert.equal(health.ok, true);
  assert.equal(health.result.state, "ready");
  assert.equal(health.result.process_id, child.pid);

  child.stdin.write(`${JSON.stringify({ id: 2, method: "shutdown" })}\n`);
  const shutdown = JSON.parse((await iterator.next()).value);
  assert.deepEqual(shutdown, {
    id: 2,
    ok: true,
    result: {
      state: "stopped",
      process_id: null,
      diagnostic_code: null,
    },
  });
  child.stdin.end();

  const exit = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  assert.deepEqual(exit, { code: 0, signal: null });
});
