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

test("protocol 349 authorizes only the reviewed 343, 348 and self transitions", async () => {
  const policy = JSON.parse(await readFile(POLICY_PATH, "utf8"))
  assert.deepEqual(Object.keys(policy).sort(), ["protocol", "rollbackTo", "schemaVersion", "upgradeFrom"])
  assert.equal(policy.schemaVersion, 1)
  assert.equal(policy.protocol, 349)
  for (const list of [policy.upgradeFrom, policy.rollbackTo]) {
    assert.ok(Array.isArray(list) && list.length > 0 && list.length <= 16)
    assert.ok(list.every((version, index) => Number.isSafeInteger(version)
      && version > 0 && (index === 0 || list[index - 1] < version)))
    assert.ok(list.includes(343), "current published protocol must remain reciprocal")
    assert.ok(list.includes(348), "additive pairing transition must remain reciprocal")
    assert.ok(list.includes(349), "new protocol must include itself")
  }
  assert.deepEqual(policy.upgradeFrom, [343, 348, 349])
  assert.deepEqual(policy.rollbackTo, [343, 348, 349])

  const scratch = await mkdtemp(join(tmpdir(), "chariox-protocol-349-policy-"))
  try {
    const newRoot = join(scratch, "protocol-349")
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

    for (const version of [343, 348, 349]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.equal(transition(oldRoot, version, newRoot, 349), "")
      assert.equal(transition(newRoot, 349, oldRoot, version), "")
    }
    for (const version of [325, 333, 342, 344, 345, 346, 347]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.throws(() => transition(oldRoot, version, newRoot, 349), /not reciprocally authorized/)
      assert.throws(() => transition(newRoot, 349, oldRoot, version), /not reciprocally authorized/)
    }
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})
