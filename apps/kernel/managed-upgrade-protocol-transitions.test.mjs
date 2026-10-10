import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import test from "node:test"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const POLICY_PATH = join(REPOSITORY_ROOT, "apps/kernel/managed-upgrade-protocol-transitions.json")
const POLICY_RELATIVE_PATH = "usr/lib/chariox/slice-build-context/apps/kernel/managed-upgrade-protocol-transitions.json"
const UPGRADE_STATE_SCRIPT = join(REPOSITORY_ROOT, "deploy/managed-kernel/managed-kernel-upgrade-state.mjs")

test("MP-07/MP-08/MP-10/MP-11 protocol 473 retains admitted predecessor contracts and rejects ambiguous branch protocols", async () => {
  const policy = JSON.parse(await readFile(POLICY_PATH, "utf8"))
  assert.deepEqual(Object.keys(policy).sort(), ["protocol", "rollbackTo", "schemaVersion", "upgradeFrom"])
  assert.equal(policy.schemaVersion, 1)
  const runtimeTypes = await readFile(join(REPOSITORY_ROOT, "apps/kernel/src/local/api/types.rs"), "utf8")
  const runtimeProtocol = Number(runtimeTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+);/)[1])
  assert.equal(policy.protocol, runtimeProtocol)
  assert.equal(policy.protocol, 473)
  for (const list of [policy.upgradeFrom, policy.rollbackTo]) {
    assert.ok(Array.isArray(list) && list.length > 0 && list.length <= 32)
    assert.ok(list.every((version, index) => Number.isSafeInteger(version)
      && version > 0 && (index === 0 || list[index - 1] < version)))
    assert.ok(list.includes(343), "tested predecessor fixture must remain reciprocal")
    assert.ok(list.includes(367), "the release before release F must stay reciprocal for in-place updates")
    assert.ok(list.includes(376), "the previous release (release F) must stay reciprocal for in-place updates")
    assert.ok(list.includes(416), "released Apps predecessor remains reciprocal")
    assert.ok(list.includes(435), "released MP-11 predecessor remains reciprocal")
    assert.ok(list.includes(472), "released main predecessor remains reciprocal")
    assert.ok(list.includes(473), "new protocol must include itself")
  }
  assert.deepEqual(policy.upgradeFrom, [343, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 410, 411, 415, 416, 435, 472, 473])
  assert.deepEqual(policy.rollbackTo, [343, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 410, 411, 415, 416, 435, 472, 473])

  const scratch = await mkdtemp(join(tmpdir(), "chariox-protocol-473-policy-"))
  try {
    const newRoot = join(scratch, "protocol-473")
    const fixturePolicy = join(newRoot, POLICY_RELATIVE_PATH)
    await mkdir(dirname(fixturePolicy), { recursive: true })
    await writeFile(fixturePolicy, JSON.stringify(policy), { mode: 0o600, flag: "wx" })
    const transition = (fromRoot, from, toRoot, to) => execFileSync(process.execPath, [
      UPGRADE_STATE_SCRIPT,
      "validate-protocol-transition",
      fromRoot,
      String(from),
      toRoot,
      String(to),
    ], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })

    // Policy fixtures do not prove real-binary persisted-state migration.
    for (const version of [343, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 410, 411, 415, 416, 435, 472]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.equal(transition(oldRoot, version, newRoot, 473), "")
      assert.equal(transition(newRoot, 473, oldRoot, version), "")
    }
    // Released G2, Apps and MP-11 predecessors are reciprocal; other branch numbers,
    // including the never-released local-browser protocol 456, stay refused.
    for (const version of [312, 325, 333, 339, 342, ...Array.from({ length: 23 }, (_, index) => 344 + index), ...Array.from({ length: 31 }, (_, index) => 379 + index), 412, 413, 414, ...Array.from({ length: 9 }, (_, index) => 417 + index), ...Array.from({ length: 36 }, (_, index) => 436 + index)]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.throws(() => transition(oldRoot, version, newRoot, 473), /not reciprocally authorized/)
      assert.throws(() => transition(newRoot, 473, oldRoot, version), /not reciprocally authorized/)
    }
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})
