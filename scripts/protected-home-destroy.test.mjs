// MP-03/MP-08/MP-10/MP-11: execute the broker Delete path with retained homes.
import assert from "node:assert/strict"
import { chmodSync, mkdtempSync, readFileSync, rmSync } from "node:fs"
import { join } from "node:path"
import test from "node:test"
import { runInNewContext } from "node:vm"
import { createHomeGenerationStore } from "../apps/kernel/slice-linux-docker/protected-home-generation.mjs"
import { retireProtectedQuotaHomes } from "../apps/kernel/slice-linux-docker/protected-home-retirement.mjs"
import { sliceDiskQuotaIdentityFromEnvironment } from "../apps/kernel/slice-linux-docker/slice-disk-quota-client.mjs"

const brokerUrl = new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url)
const broker = readFileSync(brokerUrl, "utf8")
const executeSource = broker.slice(broker.indexOf("async function execute(request)"), broker.indexOf("function errorResponse"))
  .replaceAll("import.meta.url", JSON.stringify(brokerUrl.href))
const layout = readFileSync(new URL("../apps/kernel/slice-linux-docker/protected-managed-layout.mjs", import.meta.url), "utf8")
const retainedSource = layout.slice(layout.indexOf("    retainedHomeVolumes(container) {"), layout.indexOf("    recordLegacyImage("))
const container = "chariox-slice-delete-generations"
const labels = {
  "io.chariox.slice.id": "slice-1",
  "io.chariox.slice.owner-kernel-id": "kernel-1",
  "io.chariox.slice.owner-machine-id": "machine-1",
}

function fixture(t, {local = true, bounded = false, destroyStatus = 0, quotaUnavailable = false} = {}) {
  const root = mkdtempSync(join(process.env.CHARIOX_HOME ?? process.env.HOME, ".chariox-delete-generations-test-"))
  t.after(() => rmSync(root, {recursive: true, force: true}))
  chmodSync(root, 0o711)
  const store = createHomeGenerationStore(root)
  let current = `${container}-home`
  // Real journal transitions preserve multiple earlier generations. No private
  // runtime identity is created; identity admission and Docker are fixture seams.
  for (let i = 0; i < 3; i++) {
    const generation = store.begin({container, oldHomeVolume: current,
      archiveDigest: "a".repeat(64), imageId: `sha256:${"b".repeat(64)}`})
    const parameters = {container, token: generation.token, volume: generation.newHomeVolume, digest: generation.archiveDigest}
    store.complete(parameters); store.publish(parameters); store.resolve(container, generation.newHomeVolume)
    current = generation.newHomeVolume
  }
  const homes = [...new Set([current, store.read(container).oldHomeVolume, ...store.read(container).retainedPreviousHomes])]
  assert.equal(homes.length, 4)
  const foreign = "chariox-slice-foreign-home"
  const unrecorded = `${container}-home-g${"f".repeat(32)}`
  const volumes = new Map([...homes, foreign, unrecorded].map(name => [name, {Name: name, Driver: "local", Labels: {...labels}}]))
  const calls = []
  const retainedHomeVolumes = runInNewContext(`({${retainedSource}}).retainedHomeVolumes`, {
    receipt: name => { assert.equal(name, container); return {homeVolume: current} },
    retained: () => {}, generations: store,
  })
  const environment = {CHARIOX_SLICE_NAME: container, CHARIOX_SLICE_ID: "slice-1",
    CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-1", CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-1"}
  const docker = args => {
    const [kind, action, name] = args
    assert.equal(kind, "volume")
    calls.push([action, name])
    if (action === "inspect") return volumes.has(name)
      ? {status: 0, stdout: JSON.stringify([volumes.get(name)])}
      : {status: 1, stderr: `Error: No such volume: ${name}\n`}
    assert.equal(action, "rm")
    if (volumes.get(name)?.inUse) return {status: 1}
    volumes.delete(name)
    return {status: 0}
  }
  const quotaRequest = env => ({identity: sliceDiskQuotaIdentityFromEnvironment(env)})
  const execute = runInNewContext(`${executeSource}; execute`, {
    Buffer, process,
    LOCAL_AUTHORITY: local ? {enrollment: {ownerUid: process.getuid()}} : null,
    validateRequest: request => { assert.equal(request.kind, "provisioner"); assert.equal(request.action, "destroy") },
    verifiedProtectedAuthority: () => {}, localDevRuntimeEnvironment: () => ({}),
    protectedLayouts: {homeVolume: () => current, retainedHomeVolumes},
    provisionerQuotaRequest: quotaRequest, sliceDiskQuotaIdentityFromEnvironment,
    diskQuotaMarkerPresent: () => bounded,
    requestSliceDiskQuota: async request => {
      calls.push(["quota", request.operation])
      if (quotaUnavailable && request.operation === "status") throw Object.assign(new Error("offline"), {code: "ENOENT"})
      if (request.operation === "release") assert.ok(homes.every(name => !volumes.has(name)), "release waits for every home")
      return {bounded}
    },
    sliceDiskQuotaCoordinator: {withContainerLock: async (_name, run) => run({}), revokeUnboundedProof: () => calls.push(["revoke"])},
    prepareProvisioner: async request => ({environment: request.environment}),
    PROVISIONER: "/fixture/provisioner", DOCKER_HOST: "unix:///fixture/docker.sock",
    VERIFIED_BUILD_CONTEXT_DIGEST: "", MAX_OUTPUT_BYTES: 64 * 1024,
    spawnBounded: async (_command, args) => {
      assert.deepEqual(Array.from(args), ["destroy"])
      calls.push(["destroy"])
      if (destroyStatus === 0) volumes.delete(current) // Normal provisioner removes the active home.
      return {status: destroyStatus}
    },
    releasePersistentHandles: () => calls.push(["handles"]),
    retireProtectedQuotaHomes, spawnControl: (_command, args) => docker(args), dockerEnvironment: () => ({}),
    cleanupPrepared: () => calls.push(["cleanup"]),
  })
  return {homes, current, foreign, unrecorded, volumes, calls,
    destroy: () => execute({kind: "provisioner", action: "destroy", environment})}
}

for (const placement of [
  {name: "local DEV", local: true},
  {name: "managed unbounded", local: false},
  {name: "managed bounded", local: false, bounded: true},
  {name: "managed unbounded offline allocator", local: false, quotaUnavailable: true},
]) {
  test(`MP-03/MP-08/MP-10: Delete retires every recorded home in ${placement.name}`, async t => {
    const f = fixture(t, placement)
    assert.equal((await f.destroy()).status, 0)
    assert.deepEqual(f.homes.filter(name => f.volumes.has(name)), [], "prior generated homes must not survive Delete")
    assert.ok(f.volumes.has(f.foreign)); assert.ok(f.volumes.has(f.unrecorded))
    assert.equal(f.calls.filter(call => call[0] === "quota" && call[1] === "release").length, placement.bounded ? 1 : 0)
    assert.equal(f.calls.at(-1)[0], "cleanup")
    assert.equal((await f.destroy()).status, 0, "retry accepts exact missing-volume results")
  })
}

test("MP-03/MP-10: failed provisioner Delete preserves prior homes and quota reservation", async t => {
  const f = fixture(t, {local: false, bounded: true, destroyStatus: 1})
  assert.equal((await f.destroy()).status, 1)
  assert.ok(f.homes.every(name => f.volumes.has(name)))
  assert.equal(f.calls.some(call => call[0] === "inspect" || call[0] === "rm" || call[1] === "release"), false)
})

for (const label of Object.keys(labels)) {
  test(`MP-03/MP-10/MP-11: Delete refuses a retained home with foreign ${label}`, async t => {
    const f = fixture(t)
    const prior = f.homes[1]
    f.volumes.get(prior).Labels[label] = "foreign"
    await assert.rejects(f.destroy(), /ownership is unverified/)
    assert.ok(f.volumes.has(prior)); assert.ok(f.volumes.has(f.foreign)); assert.ok(f.volumes.has(f.unrecorded))
    assert.equal(f.calls.some(call => call[0] === "rm" && call[1] === prior), false)
    assert.equal(f.calls.at(-1)[0], "cleanup")
  })
}

test("MP-03/MP-10: a still-mounted prior home fails Delete before quota release", async t => {
  const f = fixture(t, {local: false, bounded: true})
  const prior = f.homes[1]
  f.volumes.get(prior).inUse = true
  await assert.rejects(f.destroy(), /still in use/)
  assert.ok(f.volumes.has(prior))
  assert.equal(f.calls.some(call => call[1] === "release"), false)
  assert.equal(f.calls.at(-1)[0], "cleanup")
})
