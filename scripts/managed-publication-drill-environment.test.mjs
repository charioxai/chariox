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
test("image preparation BuildKit client does not inherit private installer scratch", async () => {
  const source = await readFile(new URL("../deploy/managed-kernel/prepare-hetzner-image.sh", import.meta.url), "utf8")
  const start = source.indexOf("runuser -u chariox-docker -- env \\\n  DOCKER_BUILDKIT=1")
  assert.ok(start > 0)
  const end = source.indexOf("\nrunuser", start + 1)
  assert.ok(end > start)
  const command = source.slice(start, end)
  const result = spawnSync("sh", ["-c", `runuser() { shift 4; while [ "$#" -gt 0 ]; do case "$1" in *=*) export "$1"; shift;; *) break;; esac; done; printf 'TMPDIR=%s\\n' "$TMPDIR"; }\nbroker_namespace=/unused-broker\n${command.replace(" >/dev/null", "")}`], {
    encoding: "utf8", env: { ...process.env, TMPDIR: "/root/private-installer-scratch" }, timeout: 5000,
  })
  // The runuser fixture applies the actual command's env assignments without launching Docker.
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /^TMPDIR=\/tmp$/m)
})
