// MP-04/MP-07/MP-08/MP-10/MP-11: offline native-release transaction qualification.
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { chmod, chown, mkdir, readFile, readdir, readlink, rm, stat } from "node:fs/promises"
import { join } from "node:path"
import { test as nodeTest } from "node:test"
// These tests qualify actual root ownership; other hosts report explicit skips.
const test = (name, run) => nodeTest(name, {
  skip: process.platform !== "linux" || process.getuid() !== 0,
}, run)
import { makeHarness, put } from "./lib/managed-kernel-upgrade-fixture.mjs"

const updateId = "managed_release_update_0123abcd-0000-4000-8000-0123456789ab"

async function path1Harness(context) {
  const targetTransitionPolicy = JSON.parse(await readFile(
    new URL("../apps/kernel/managed-upgrade-protocol-transitions.json", import.meta.url), "utf8"))
  const harness = await makeHarness(context, { path1Release: true, rotateBuilder: true,
    currentProtocol: 370, targetProtocol: targetTransitionPolicy.protocol, targetTransitionPolicy,
    currentAppArtifacts: false,
  })
  const home = join(harness.installRoot, "home/chariox")
  const state = join(home, ".chariox")
  await chown(home, harness.charioxIdentity.uid, harness.charioxIdentity.gid)
  await chown(state, harness.charioxIdentity.uid, harness.charioxIdentity.gid)
  await chmod(state, 0o700)
  for (const [name, relativePath] of [
    ["history", ".chariox/sessions/history-fixture"],
    ["profile", ".local/share/chariox-profile-fixture/marker"],
  ]) {
    const path = join(home, relativePath)
    await put(path, `synthetic-${name}-continuity\n`, 0o600)
    await chown(path, harness.charioxIdentity.uid, harness.charioxIdentity.gid)
    harness.persistent[name] = path
  }
  const runtimePin = join(harness.installRoot, "etc/chariox/trusted-builder-public-key")
  await put(runtimePin, await readFile(harness.trustedBuilderKey), 0o644)
  await put(join(harness.state, "check-builder-pin-on-start"), "check\n")
  return { ...harness, runtimePin, env: {
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: harness.nextTrustedBuilderKey,
    CHARIOX_MANAGED_RELEASE_UPDATE_ID: updateId,
  } }
}

async function serviceMutations(harness) {
  return (await readFile(join(harness.state, "systemctl.log"), "utf8"))
    .split("\n").filter(line => line && !line.startsWith("show "))
}

async function snapshot(harness) {
  const state = join(harness.installRoot, "home/chariox/.chariox")
  const paths = [state, ...Object.values(harness.persistent)]
  return Promise.all(paths.map(async path => {
    const info = await stat(path)
    return { path, dev: info.dev, ino: info.ino, uid: info.uid, gid: info.gid,
      mode: info.mode, digest: info.isFile()
        ? createHash("sha256").update(await readFile(path)).digest("hex") : null }
  }))
}

async function recoverWithoutImage(harness, env = {}) {
  await rm(harness.target.rootfs, { recursive: true })
  return harness.run({ ...harness.env, CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY: "1", ...env })
}

async function assertSettled(harness, role, phase, before) {
  const authority = join(harness.installRoot, "usr/lib/chariox")
  const selected = role === "previous" ? harness.current : harness.target
  assert.equal(await readlink(join(authority, "current")), `releases/${selected.digest.slice(7)}`)
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, selected.digest)
  const expectedPin = role === "previous" ? harness.trustedBuilderKey : harness.nextTrustedBuilderKey
  assert.equal(await readFile(harness.runtimePin, "utf8"), await readFile(expectedPin, "utf8"))
  assert.deepEqual(await snapshot(harness), before)
  for (const name of [".managed-kernel-upgrade", ".managed-kernel-upgrade.terminal"]) {
    await assert.rejects(stat(join(authority, name)), { code: "ENOENT" })
  }
  const result = await readFile(join(authority, ".managed-kernel-upgrade-result"), "utf8")
  assert.equal(result, ["1", updateId, harness.current.digest, harness.target.digest,
    "environment-1", "machine-1", "kernel-1", phase, ""].join("\n"))
}

test("MP-07/MP-10 unexpected exit after stopping restores the previous release before cleanup", async context => {
  const harness = await path1Harness(context)
  const before = await snapshot(harness)
  await put(join(harness.state, "fail-after-stopped"), "fail once\n")
  const result = harness.run(harness.env)
  assert.equal(result.status, 23, result.stderr)
  await assertSettled(harness, "previous", "rolled_back", before)
  assert.ok((await serviceMutations(harness)).includes("start chariox-path1-managed-bootstrap.service"))
})

test("MP-07/MP-10 activation diagnostics identify steps and unexpected exits without payloads", async context => {
  const steps = ["builder_pin", "home_migration", "receipt", "release_override", "app_prepare",
    "current_link", "data_volume_links", "app_storage", "slice_facade", "slice_facade_check"]
  for (const fail of [false, true]) {
    const harness = await path1Harness(context)
    const directory = join(harness.state, "public-diagnostics")
    await mkdir(directory, { mode: 0o700 })
    if (fail) await put(join(harness.state, "fail-after-stopped"), "fail once\n")
    const result = harness.run({ ...harness.env, CHARIOX_RUNTIME_DIAGNOSTICS_DIR: directory })
    assert.equal(result.status, fail ? 23 : 0, result.stderr)
    const records = (await Promise.all((await readdir(directory)).map(async name =>
      (await readFile(join(directory, name), "utf8")).trim().split("\n").map(line => JSON.parse(line))))).flat()
    for (const record of records) assert.deepEqual(Object.keys(record).sort(), ["atMs", "event", "pid", "schema"])
    const events = records.sort((a, b) => a.atMs - b.atMs).map(record => record.event)
    const pinSteps = ["builder_pin_journal_start", "builder_pin_journal_returned",
      "builder_pin_runtime_start", "builder_pin_runtime_returned",
      "builder_pin_compare_start", "builder_pin_compare_returned",
      "builder_pin_atomic_start", "builder_pin_atomic_returned"]
    assert.deepEqual(events, fail ? ["prepared", "update_unexpected_exit", ...pinSteps, "rolled_back"]
      : ["prepared", "stopped", "activation_builder_pin_start", ...pinSteps,
        ...steps.slice(1).map(step => `activation_${step}_start`), "activated", "committed"])
  }
})

for (const [boundary, selected, phase] of [
  ["phase-prepared", "previous", "rolled_back"],
  ["phase-stopped", "previous", "rolled_back"],
  ["builder-pin", "previous", "rolled_back"],
  ["phase-activated", "previous", "rolled_back"],
  ["supervisor-start", "previous", "rolled_back"],
  ["phase-committed", "target", "committed"],
  ["committed-tombstone", "target", "committed"],
  ["phase-rolled_back", "previous", "rolled_back"],
  ["rolled_back-tombstone", "previous", "rolled_back"],
]) test(`MP-04/MP-07/MP-10 Path-1 recovery without image at ${boundary}`, async context => {
  const harness = await path1Harness(context)
  const before = await snapshot(harness)
  if (boundary.includes("rolled_back") || boundary.includes("rolled-back")) {
    await put(join(harness.state, "fail-health-once"), "fail\n")
  }
  await put(join(harness.state, `crash-after-${boundary}`), "crash\n")
  const interrupted = harness.run(harness.env)
  if (boundary === "builder-pin") {
    // MP-07: the pin validator is isolated; its death now reaches the caller's
    // rollback branch instead of killing the updater with a stopped kernel.
    assert.equal(interrupted.status, 1, interrupted.stderr)
    await assertSettled(harness, "previous", "rolled_back", before)
  } else {
    assert.equal(interrupted.signal, "SIGKILL", interrupted.stderr)
  }
  const recovered = await recoverWithoutImage(harness)
  assert.equal(recovered.status, 0, recovered.stderr)
  await assertSettled(harness, selected, phase, before)
  // Replay never dispatches a new target after settling the original update.
  const calls = await serviceMutations(harness)
  const replay = harness.run({ ...harness.env, CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY: "1" })
  assert.equal(replay.status, 0, replay.stderr)
  assert.deepEqual(await serviceMutations(harness), calls)
})

for (const boundary of ["phase-activated", "phase-committed", "committed-tombstone", "phase-rolled_back", "rolled_back-tombstone"]) {
  test(`MP-07/MP-11 recovery rejects tampered selected release at ${boundary}`, async context => {
    const harness = await path1Harness(context)
    if (boundary.includes("rolled_back")) await put(join(harness.state, "fail-health-once"), "fail\n")
    await put(join(harness.state, `crash-after-${boundary}`), "crash\n")
    assert.equal(harness.run(harness.env).signal, "SIGKILL")
    const selected = boundary === "phase-activated" || boundary.includes("rolled_back") ? harness.current : harness.target
    const binary = join(harness.installRoot, "usr/lib/chariox/releases",
      selected.digest.slice(7), "usr/local/bin/chariox-kernel")
    await put(binary, "#!/bin/sh\necho 370\n# corrupted signed executable\n", 0o755)
    const calls = await serviceMutations(harness)
    const current = await readlink(join(harness.installRoot, "usr/lib/chariox/current"))
    const result = await recoverWithoutImage(harness)
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /release artifact chariox-kernel is corrupted/)
    assert.deepEqual(await serviceMutations(harness), calls)
    assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), current)
  })
}

test("MP-07/MP-11 recovery rejects a foreign Cloud command without service mutation", async context => {
  const harness = await path1Harness(context)
  await put(join(harness.state, "crash-after-phase-activated"), "crash\n")
  assert.equal(harness.run(harness.env).signal, "SIGKILL")
  const before = await snapshot(harness)
  const calls = await serviceMutations(harness)
  const result = await recoverWithoutImage(harness, {
    CHARIOX_MANAGED_RELEASE_UPDATE_ID: updateId.replace(/b$/, "c"),
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /does not match the Cloud command/)
  assert.deepEqual(await serviceMutations(harness), calls)
  assert.deepEqual(await snapshot(harness), before)
})


test("MP-04/MP-07/MP-10 publication interruption leaves previous release and state intact", async context => {
  const harness = await path1Harness(context)
  const before = await snapshot(harness)
  await put(join(harness.state, "crash-after-release-publication"), "crash\n")
  assert.equal(harness.run(harness.env).signal, "SIGKILL")
  const authority = join(harness.installRoot, "usr/lib/chariox")
  assert.equal(await readlink(join(authority, "current")), `releases/${harness.current.digest.slice(7)}`)
  await assert.rejects(stat(join(authority, ".managed-kernel-upgrade")), { code: "ENOENT" })
  assert.deepEqual(await serviceMutations(harness), [])
  assert.deepEqual(await snapshot(harness), before)
  // Before journal publication, normal retry still requires the signed incoming
  // image. Recovery-only is an idempotent no-op; it must not invent activation.
  assert.equal(harness.run({ ...harness.env, CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY: "1" }).status, 0)
  assert.deepEqual(await serviceMutations(harness), [])
  const retried = harness.run(harness.env)
  assert.equal(retried.status, 0, retried.stderr)
  await assertSettled(harness, "target", "committed", before)
})

test("MP-04/MP-07/MP-08/MP-10 signed B-to-new-to-B-to-new fixture preserves state", async context => {
  const harness = await path1Harness(context)
  const before = await snapshot(harness)
  for (const [index, from, target, currentPin, nextPin] of [
    [1, harness.current, harness.target, harness.trustedBuilderKey, harness.nextTrustedBuilderKey],
    [2, harness.target, harness.current, harness.nextTrustedBuilderKey, harness.trustedBuilderKey],
    [3, harness.current, harness.target, harness.trustedBuilderKey, harness.nextTrustedBuilderKey],
  ]) {
    const id = updateId.replace(/b$/, String(index))
    const result = harness.run({ ...harness.env,
      CHARIOX_MANAGED_RELEASE_UPDATE_ID: id,
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: currentPin,
      CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: nextPin,
    }, [target.rootfs, from.digest, target.digest, harness.trustedKey])
    assert.equal(result.status, 0, result.stderr)
    const authority = join(harness.installRoot, "usr/lib/chariox")
    assert.equal(await readlink(join(authority, "current")), `releases/${target.digest.slice(7)}`)
    assert.equal(await readFile(harness.runtimePin, "utf8"), await readFile(nextPin, "utf8"))
    assert.deepEqual(await snapshot(harness), before)
    const receipt = JSON.parse(await readFile(harness.receiptPath, "utf8"))
    assert.deepEqual(receipt, { ...harness.receipt, runtimeReleaseDigest: target.digest })
    const evidence = (await readFile(join(authority, ".managed-kernel-upgrade-result"), "utf8")).trimEnd().split("\n")
    assert.deepEqual(evidence, ["1", id, from.digest, target.digest, "environment-1", "machine-1", "kernel-1", "committed"])
  }
})
