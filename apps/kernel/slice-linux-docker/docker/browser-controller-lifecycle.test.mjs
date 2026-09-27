import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

test("Linux browser lifecycle owns descendants and crash-safe upload locks", {
  skip: process.platform !== "linux",
  timeout: 95_000,
}, () => {
  const result = spawnSync("python3", [fileURLToPath(new URL("./test_browser_lifecycle.py", import.meta.url))], {
    encoding: "utf8",
    timeout: 90_000,
    maxBuffer: 1024 * 1024,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.match(result.stderr, /Ran 8 tests/);
  assert.match(result.stderr, /\bOK\b/);
});

test("host restart reconciliation requires exact stopped generation and safe receipts", () => {
  const result = spawnSync("python3", ["-B", fileURLToPath(new URL("../test_reconcile_stopped_browser_lifetimes.py", import.meta.url))], {
    encoding: "utf8", timeout: 10_000, maxBuffer: 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.match(result.stderr, /Ran 7 tests/);
});
