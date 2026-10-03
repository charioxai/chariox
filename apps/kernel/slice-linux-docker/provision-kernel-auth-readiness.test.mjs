import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const script = readFileSync(new URL("./docker/start-runtime.sh", import.meta.url), "utf8")
const start = script.indexOf("wait_for_kernel_auth_consumption() {")
assert.ok(start >= 0)
const helper = script.slice(start, script.indexOf("\n}\n", start) + 3)
function run({ alive, consume }) {
  const scratch = mkdtempSync(join(tmpdir(), "chariox-kernel-readiness-"))
  try {
    const marker = join(scratch, "readiness.marker")
    writeFileSync(marker, "synthetic marker")
    return spawnSync("bash", ["-c", `
set -uo pipefail
polls=0
screen() { ${alive ? 'printf "1.chariox-slice-kernel (Detached)\\n"' : 'return 1'}; }
sleep() { polls=$((polls+1)); if [[ "$polls" == "${consume}" ]]; then mv "$KERNEL_LOCAL_AUTH_FILE" "$KERNEL_LOCAL_AUTH_FILE.consumed"; fi; }
${helper}
wait_for_kernel_auth_consumption
result=$?
printf 'polls=%s\\n' "$polls"
exit "$result"
`], { env: { ...process.env, KERNEL_LOCAL_AUTH_FILE: marker }, encoding: "utf8", timeout: 5000 })
  } finally { rmSync(scratch, { recursive: true, force: true }) }
}
test("kernel readiness waits for delayed token consumption", () => {
  const r = run({ alive: true, consume: 2 })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /polls=2/)
})
test("kernel readiness refuses an exited process before consumption", () => {
  const r = run({ alive: false, consume: -1 })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /exited before consuming/)
  assert.match(r.stdout, /polls=0/)
})
test("kernel readiness bounds a live process that never consumes its token", () => {
  const r = run({ alive: true, consume: -1 })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /did not consume/)
  assert.match(r.stdout, /polls=60/)
})
