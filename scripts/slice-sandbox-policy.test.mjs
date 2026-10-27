import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { spawnSync } from "node:child_process"
import test from "node:test"

const source = await readFile(new URL("../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url), "utf8")
const start = source.indexOf('    if [[ "$SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY" == "1" ]]; then')
const end = source.indexOf('    if [[ "$SLICE_DEVELOPMENT_MOUNT_COUNT"', start)
assert.ok(start >= 0 && end > start)
const generate = (selected) => {
  const result = spawnSync("/bin/bash", ["-c", `set -eu\ndocker_create_args=()\n${source.slice(start, end)}\nprintf '%s\\0' "\${docker_create_args[@]}"`], {
    env: { PATH: process.env.PATH, SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY: selected ? "1" : "0", SLICE_ALLOW_UNCONFINED_SECCOMP: "0", SLICE_APPARMOR_PROFILE: "chariox-slice-provider", SCRIPT_DIR: "/signed/runtime" },
    encoding: "utf8",
  })
  assert.equal(result.status, 0, result.stderr)
  return result.stdout.split("\0").filter(Boolean)
}

test("MP-01/MP-08/MP-11 explicit slice options select the shared container security arguments", () => {
  assert.deepEqual(generate(false), ["--security-opt", "seccomp=/signed/runtime/chromium-seccomp.json"])
  assert.deepEqual(generate(true), [
    "--cap-add", "SYS_ADMIN", "--cap-add", "NET_ADMIN", "--cap-add", "SYS_PTRACE",
    "--security-opt", "seccomp=unconfined", "--security-opt", "apparmor=chariox-slice-provider",
    "--security-opt", "systempaths=unconfined",
  ])
})
