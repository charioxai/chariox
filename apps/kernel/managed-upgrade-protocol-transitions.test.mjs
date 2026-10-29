import { localProtocolVersion, admittedPredecessorProtocols } from "../../scripts/lib/protocol-versions.mjs"
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

test("MP-08/MP-09/MP-10/MP-11 managed upgrades admit only the target and the current and previous releases", async () => {
  const policy = JSON.parse(await readFile(POLICY_PATH, "utf8"))
  assert.deepEqual(Object.keys(policy).sort(), ["protocol", "rollbackTo", "schemaVersion", "upgradeFrom"])
  assert.equal(policy.schemaVersion, 1)
  const runtimeTypes = await readFile(join(REPOSITORY_ROOT, "apps/kernel/src/local/api/types.rs"), "utf8")
  const runtimeProtocol = Number(runtimeTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+);/)[1])
  assert.equal(policy.protocol, runtimeProtocol)
  assert.equal(policy.protocol, localProtocolVersion)
  assert.deepEqual(admittedPredecessorProtocols, [629, 636])
  const publication = await readFile(join(REPOSITORY_ROOT, "docker/publication/Dockerfile"), "utf8")
  const publicationPins = [...publication.matchAll(/^ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=(\d+)$/gm)].map(match => Number(match[1]))
  assert.deepEqual(publicationPins, [runtimeProtocol, runtimeProtocol], "builder and runtime publication pins must match the binary")
  for (const list of [policy.upgradeFrom, policy.rollbackTo]) {
    assert.ok(Array.isArray(list) && list.length > 0 && list.length <= 32, "older managed validators refuse a policy over 32 entries")
    assert.ok(list.every((version, index) => Number.isSafeInteger(version)
      && version > 0 && (index === 0 || list[index - 1] < version)))
    assert.deepEqual(list, [629, 636, localProtocolVersion])
  }
  assert.deepEqual(policy.upgradeFrom, [...admittedPredecessorProtocols, localProtocolVersion])
  assert.deepEqual(policy.rollbackTo, [...admittedPredecessorProtocols, localProtocolVersion])

  const scratch = await mkdtemp(join(tmpdir(), "chariox-protocol-current-policy-"))
  try {
    const newRoot = join(scratch, `protocol-${localProtocolVersion}`)
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
    for (const version of [629, 636, localProtocolVersion]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.equal(transition(oldRoot, version, newRoot, localProtocolVersion), "")
      assert.equal(transition(newRoot, localProtocolVersion, oldRoot, version), "")
    }
    // Retired releases and branch protocols are refused in both directions.
    for (const version of [
      343, 367, 376, 377, 410, 411, 415, 416, 435, 472, 487, 490, 491, 493,
      506, 511, 513, 517, 519, 520, 528, 552, 556, 567, 589, 592, 594, 596, 599, 614,
      312, 325, 333, 339, 342, 372, 412, 413, 414, 488, 489, 492, 495, 496, 499,
      ...Array.from({ length: 23 }, (_, index) => 344 + index),
      ...Array.from({ length: 32 }, (_, index) => 378 + index),
      ...Array.from({ length: 18 }, (_, index) => 417 + index),
      ...Array.from({ length: 26 }, (_, index) => 436 + index),
      ...Array.from({ length: 14 }, (_, index) => 473 + index),
    ]) {
      const oldRoot = join(scratch, `protocol-${version}`)
      assert.throws(() => transition(oldRoot, version, newRoot, localProtocolVersion), /not reciprocally authorized/)
      assert.throws(() => transition(newRoot, localProtocolVersion, oldRoot, version), /not reciprocally authorized/)
    }
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})
