import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

// Extend #868's script guard boundary to the relay lane's incidentally found adapters.
const unsafeSignal = /\b(?:process\s*\.\s*kill|\w+\s*\.\s*kill|libc\s*::\s*kill|os\s*\.\s*(?:kill|killpg))\s*\(|\b(?:pkill|killall)\b/
const paths = [
  "apps/kernel/src/managed_context/kernel/import.rs",
  "apps/kernel/src/managed_bootstrap/worker.rs",
  "apps/kernel/src/slice/local_docker/state.rs",
  "apps/kernel/slice-linux-docker/slice-command-guard.py",
  "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
  "apps/cli/scripts/live-relay-identity-security-drill.mjs",
  "apps/cli/scripts/live-room-environment-pointer-click-drill.mjs",
  "apps/cli/scripts/live-docker-slice-browser-state-drill.mjs",
]

test("MP11 adapter scan catches negative group probes, child timers and name signals", () => {
  for (const source of ["libc::kill(-(pid as i32), SIGKILL)", "process.kill(-pid, 0)",
    "setTimeout(() => child.kill('SIGKILL'), 100)", "os.killpg(group, signal.SIGTERM)", "pkill -f provider"]) {
    assert.match(source, unsafeSignal)
  }
  assert.doesNotMatch("signalOwnedProcess(child, 'SIGTERM')", unsafeSignal)
})
for (const path of paths) {
  test(`MP11 ${path} uses the shared signal guard`, async () => {
    const source = await readFile(new URL(`../${path}`, import.meta.url), "utf8")
    assert.doesNotMatch(source, unsafeSignal, "raw process adapter escaped the shared guard")
  })
}
