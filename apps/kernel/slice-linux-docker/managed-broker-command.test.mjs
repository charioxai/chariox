import assert from "node:assert/strict"
import test from "node:test"
import { dockerControlPolicy } from "./managed-broker-command.mjs"

const plain = ["exec", "-u", "slice", "chariox-slice-owned", "/opt/chariox-slice/slice-screen.sh"]
const configured = ["exec", "-e", "CHARIOX_SLICE_VIEWER_BACKEND=selkies", "-e",
  "CHARIOX_SLICE_NOVNC_PORT=6080", "-e", "CHARIOX_SLICE_DISPLAY_MODE=headed",
  "-u", "slice", "chariox-slice-owned", "/opt/chariox-slice/slice-screen.sh"]

test("approved desktop starts outlive the readiness and cleanup bound", () => {
  for (const command of [plain, configured]) {
    assert.equal(dockerControlPolicy([...command, "start"]).timeout, 120_000)
  }
})

test("other desktop actions and execs keep the ordinary control deadline", () => {
  for (const command of [plain, configured]) {
    for (const action of ["stop", "status", "prepare", "interact"]) {
      assert.equal(dockerControlPolicy([...command, action]).timeout, 30_000)
    }
    assert.equal(dockerControlPolicy([...command, "start", "extra"]).timeout, 30_000)
  }
  assert.equal(dockerControlPolicy(["exec", "-u", "root", ...plain.slice(3), "start"]).timeout, 30_000)
  assert.equal(dockerControlPolicy(["exec", "-u", "slice", "chariox-slice-owned", "test", "-s", "/fixture"]).timeout, 30_000)
})
