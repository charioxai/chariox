import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const script = readFileSync(new URL("./docker/start-runtime.sh", import.meta.url), "utf8")
const start = script.indexOf("start_slice_kernel() {")
assert.ok(start >= 0)
const launcher = script.slice(start, script.indexOf("\n}\n", start) + 3)

// `screen` starts its window with SHELL=/bin/sh when SHELL is unset, and the kernel
// hands that to every Room terminal: the user's shell would be sh, with no profile.
test("the slice kernel is started with the slice user's login shell", () => {
  const scratch = mkdtempSync(join(tmpdir(), "chariox-kernel-shell-"))
  try {
    const run = spawnSync("bash", ["-c", `
screen() { printf '%s\\n' "$@"; }
wait_for_screen_session() { :; }
wait_for_kernel_auth_consumption() { :; }
${launcher}
start_slice_kernel
`], { env: { PATH: process.env.PATH, KERNEL_LOCAL_AUTH_FILE: join(scratch, "auth"), LOGS: scratch }, encoding: "utf8", timeout: 5000 })
    assert.equal(run.status, 0, run.stderr)
    assert.ok(run.stdout.split("\n").includes("SHELL=/bin/bash"), run.stdout)
  } finally { rmSync(scratch, { recursive: true, force: true }) }
})
