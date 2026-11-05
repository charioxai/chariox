import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { spawnSync } from "node:child_process"
import test from "node:test"

for (const script of ["verify-slice-publication-access.sh", "verify-rootless-handle-lifecycle.sh"]) {
test(`${script} does not inherit root-private installer scratch`, async () => {
  const source = await readFile(new URL(`../deploy/managed-kernel/${script}`, import.meta.url), "utf8")
  const boundary = source.indexOf("helper=/usr/lib/chariox/")
  assert.ok(boundary > 0)
  const result = spawnSync("sh", ["-c", `id() { printf '0\\n'; }\n${source.slice(0, boundary)}\nsh -c 'printf %s "$TMPDIR"'`], {
    encoding: "utf8", env: { ...process.env, TMPDIR: "/root/private-installer-scratch" }, timeout: 5000,
  })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, "/tmp")
})
}
