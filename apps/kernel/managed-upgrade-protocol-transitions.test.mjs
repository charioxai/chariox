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

test("MP-07 / MP-08 / MP-11: protocol 430 retains admitted predecessor contracts and rejects ambiguous branch protocols", async () => {
  const policy = JSON.parse(await readFile(POLICY_PATH, "utf8"))
  assert.deepEqual(Object.keys(policy).sort(), ["protocol", "rollbackTo", "schemaVersion", "upgradeFrom"])
  assert.equal(policy.schemaVersion, 1)
  const runtimeTypes = await readFile(join(REPOSITORY_ROOT, "apps/kernel/src/local/api/types.rs"), "utf8")
  const runtimeProtocol = Number(runtimeTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+);/)[1])
  assert.equal(policy.protocol, runtimeProtocol)
  assert.equal(policy.protocol, 430)
  for (const list of [policy.upgradeFrom, policy.rollbackTo]) {
    assert.ok(Array.isArray(list) && list.length > 0 && list.length <= 32)
    assert.ok(list.every((version, index) => Number.isSafeInteger(version)
      && version > 0 && (index === 0 || list[index - 1] < version)))
    assert.ok(list.includes(343), "tested predecessor fixture must remain reciprocal")
    assert.ok(list.includes(367), "the release before release F must stay reciprocal for in-place updates")
    assert.ok(list.includes(376), "the previous release (release F) must stay reciprocal for in-place updates")
    assert.ok(list.includes(430), "new protocol must include itself")
  }
  assert.deepEqual(policy.upgradeFrom, [343, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 410, 411, 415, 416, 430])
  assert.deepEqual(policy.rollbackTo, [343, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 410, 411, 415, 416, 430])

  const scratch = await mkdtemp(join(tmpdir(), "chariox-protocol-430-policy-"))
  try {
    const newRoot = join(scratch, "protocol-430")
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
    for (const version of [343, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 410, 411, 415, 416]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.equal(transition(oldRoot, version, newRoot, 430), "")
      assert.equal(transition(newRoot, 430, oldRoot, version), "")
    }
    // Released G2 and Apps predecessors are reciprocal; other branch numbers stay refused.
    for (const version of [312, 325, 333, 339, 342, ...Array.from({ length: 23 }, (_, index) => 344 + index), ...Array.from({ length: 31 }, (_, index) => 379 + index), 412, 413, 414, ...Array.from({ length: 13 }, (_, index) => 417 + index)]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.throws(() => transition(oldRoot, version, newRoot, 430), /not reciprocally authorized/)
      assert.throws(() => transition(newRoot, 430, oldRoot, version), /not reciprocally authorized/)
    }
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})

// MP-07 / MP-08 / MP-11: exercise the same policy validator used by upgrade-image.sh.
test("MP-07 / MP-08 / MP-11: 416 to 430 upgrade and rollback require the shipped reciprocal policy", async () => {
  const policy = JSON.parse(await readFile(POLICY_PATH, "utf8"))
  const scratch = await mkdtemp(join(tmpdir(), "chariox-retire-meta-transition-"))
  try {
    const oldRoot = join(scratch, "protocol-416")
    const newRoot = join(scratch, "protocol-430")
    const fixturePolicy = join(newRoot, POLICY_RELATIVE_PATH)
    await mkdir(dirname(fixturePolicy), { recursive: true })
    const validate = (fromRoot, from, toRoot, to) => execFileSync(process.execPath, [
      UPGRADE_STATE_SCRIPT, "validate-protocol-transition", fromRoot, String(from), toRoot, String(to),
    ], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
    await writeFile(fixturePolicy, JSON.stringify(policy), { mode: 0o600, flag: "wx" })
    assert.equal(validate(oldRoot, 416, newRoot, 430), "")
    assert.equal(validate(newRoot, 430, oldRoot, 416), "")
    assert.equal(validate(newRoot, 430, newRoot, 430), "")
    // A stale declaration or an upgrade-only contract must fail before activation.
    await writeFile(fixturePolicy, JSON.stringify({ ...policy, protocol: 416 }))
    assert.throws(() => validate(oldRoot, 416, newRoot, 430), /policy is invalid/)
    assert.throws(() => validate(newRoot, 430, oldRoot, 416), /policy is invalid/)
    await writeFile(fixturePolicy, JSON.stringify({ ...policy, rollbackTo: policy.rollbackTo.filter((p) => p !== 416) }))
    assert.throws(() => validate(oldRoot, 416, newRoot, 430), /not reciprocally authorized/)
    assert.throws(() => validate(newRoot, 430, oldRoot, 416), /not reciprocally authorized/)
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})
