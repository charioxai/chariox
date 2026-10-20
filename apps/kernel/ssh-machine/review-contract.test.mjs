// MP-08 / MP-11: fail-first review regressions for target absence and actual Cloud wire admission.
import assert from "node:assert/strict"
import test from "node:test"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { runMachine } from "./remote.mjs"
test("MP-08/MP-11 reconciliation can prove an unpublished target absent without changing it", async t => {
  const dir = await mkdtemp(join(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir(), "byom-review-")); t.after(() => rm(dir, { recursive: true, force: true }))
  const home = join(dir, "home"); await mkdir(home)
  const result = await runMachine({ action: "inspect", installId: "byom-test", port: 55129, releaseDigest: `sha256:${"a".repeat(64)}` }, { home, serviceManager: async () => "LoadState=not-found\nFragmentPath=\nDropInPaths=\n" })
  assert.equal(result.status, "absent")
})

test("MP-11 reconciliation cannot declare a foreign unit absent", async t => {
  const dir = await mkdtemp(join(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir(), "byom-review-foreign-")); t.after(() => rm(dir, { recursive: true, force: true }))
  const home = join(dir, "home"); await mkdir(home)
  await assert.rejects(runMachine({ action: "inspect", installId: "byom-test", port: 55129, releaseDigest: `sha256:${"a".repeat(64)}` }, { home, serviceManager: async () => "LoadState=loaded\nFragmentPath=/foreign/unit.service\nDropInPaths=\n" }), /unit exists/)
})
