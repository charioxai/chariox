import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

for (const suite of ["test_browser_upload_store.py", "test_browser_upload_retirement_markers.py"]) {
test(`upload lifetime safety: ${suite}`, () => {
  const result = spawnSync("python3", [fileURLToPath(new URL(`./${suite}`, import.meta.url))], {
    encoding: "utf8",
    timeout: 10_000,
    maxBuffer: 1024 * 1024,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.match(result.stderr, /\bOK\b/);
});
}
