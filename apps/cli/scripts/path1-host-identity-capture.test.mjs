import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"

import {
  capturePath1HostIdentity,
  Path1HostIdentityCaptureError,
} from "./path1-host-identity-capture.mjs"

const BOOT_PATH = "/proc/sys/kernel/random/boot_id"
const MACHINE_PATH = "/etc/machine-id"
const BOOT_ID = "01234567-89ab-cdef-0123-456789abcdef"
const CHANGED_BOOT_ID = "fedcba98-7654-3210-fedc-ba9876543210"
const MACHINE_ID = "a".repeat(32)
const UNIT = "chariox-path1-managed-bootstrap.service"
const SECOND_UNIT = "chariox-managed-bootstrap.service"
const ACTIVATION = "/usr/local/bin/chariox-kernel"
const CURRENT = "/usr/lib/chariox/current"
const RELEASES = "/usr/lib/chariox/releases"
const VAR_STATE = "/var/lib/chariox"
const HOME_STATE = "/home/chariox"

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function makeFixture() {
  const nodes = new Map()
  const systemdCalls = []
  const unitProperties = new Map()
  let nextInode = 100
  const sourceCommit = "1".repeat(40)
  const sourceTree = "2".repeat(40)
  const manifestBytes = Buffer.from(JSON.stringify({ schemaVersion: 2, sourceCommit, sourceTree, artifacts: [] }))
  const digest = sha256(manifestBytes)
  const releaseHex = digest.slice("sha256:".length)
  const releaseRoot = `${RELEASES}/${releaseHex}`
  const kernelPath = `${releaseRoot}/usr/local/bin/chariox-kernel`
  const manifestPath = `${releaseRoot}/usr/lib/chariox/release-manifest.json`

  function add(filePath, kind, bytes = Buffer.alloc(0), { uid = 0, gid = 0, mode = null } = {}) {
    nodes.set(filePath, {
      kind,
      bytes: Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes),
      uid,
      gid,
      mode: mode ?? (kind === "directory" ? 0o755 : kind === "symlink" ? 0o777 : 0o644),
      inode: nextInode++,
    })
  }

  for (const directory of [
    "/proc",
    "/proc/sys",
    "/proc/sys/kernel",
    "/proc/sys/kernel/random",
    "/etc",
    "/usr",
    "/usr/local",
    "/usr/local/bin",
    "/usr/lib",
    "/usr/lib/chariox",
    RELEASES,
    VAR_STATE,
    HOME_STATE,
    releaseRoot,
    `${releaseRoot}/usr`,
    `${releaseRoot}/usr/local`,
    `${releaseRoot}/usr/local/bin`,
    `${releaseRoot}/usr/lib`,
    `${releaseRoot}/usr/lib/chariox`,
  ]) add(directory, "directory")

  add(BOOT_PATH, "file", `${BOOT_ID}\n`, { mode: 0o444 })
  add(MACHINE_PATH, "file", `${MACHINE_ID}\n`, { mode: 0o444 })
  add("/proc/321/stat", "file", processStat(321))
  add(ACTIVATION, "symlink")
  add(CURRENT, "symlink")
  add(kernelPath, "file", "kernel", { mode: 0o755 })
  add(manifestPath, "file", manifestBytes)

  const fileSystem = {
    async lstat(filePath) {
      const entry = nodes.get(filePath)
      if (!entry) throw fsError("ENOENT", `missing ${filePath}`)
      return {
        dev: 2049n,
        ino: BigInt(entry.inode),
        uid: BigInt(entry.uid),
        gid: BigInt(entry.gid),
        mode: BigInt(entry.mode),
        size: BigInt(entry.bytes.length),
        isSymbolicLink: () => entry.kind === "symlink",
        isDirectory: () => entry.kind === "directory",
        isFile: () => entry.kind === "file",
      }
    },
    async readFile(filePath) {
      const entry = nodes.get(filePath)
      if (!entry || entry.kind !== "file") throw fsError("ENOENT", `missing ${filePath}`)
      return entry.bytes
    },
    async readlink(filePath) {
      if (filePath === ACTIVATION) return "../../../usr/lib/chariox/current/usr/local/bin/chariox-kernel"
      if (filePath === CURRENT) return `releases/${releaseHex}`
      throw fsError("EINVAL", `not a symlink: ${filePath}`)
    },
    async realpath(filePath) {
      if (filePath === CURRENT || filePath === releaseRoot) return releaseRoot
      if (filePath === ACTIVATION || filePath === kernelPath) return kernelPath
      if (nodes.has(filePath) && nodes.get(filePath).kind !== "symlink") return filePath
      throw fsError("ENOENT", `missing ${filePath}`)
    },
  }

  return {
    nodes,
    fileSystem,
    systemdCalls,
    unitProperties,
    digest,
    releaseRoot,
    kernelPath,
    manifestPath,
    async runCommand(command, args) {
      assert.equal(command, "/usr/bin/systemctl")
      assert.deepEqual(args.slice(0, -1), [
        "show", "--no-pager", "--property=Id", "--property=InvocationID",
        "--property=MainPID", "--property=ActiveState",
      ])
      const unit = args.at(-1)
      systemdCalls.push(unit)
      const values = unitProperties.get(unit) ?? {
        invocationId: "b".repeat(32),
        mainPid: "321",
        activeState: "active",
      }
      return {
        stdout: `Id=${unit}\nInvocationID=${values.invocationId}\nMainPID=${values.mainPid}\nActiveState=${values.activeState}\n`,
        stderr: "",
        exitCode: 0,
      }
    },
  }
}

function fsError(code, message) {
  const error = new Error(message)
  error.code = code
  return error
}

function processStat(pid, startTime = "98765", comm = "kernel worker") {
  const fields = ["S", "1", "1", "1", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", startTime]
  return `${pid} (${comm}) ${fields.join(" ")}`
}

function capture(fixture, options = {}) {
  return capturePath1HostIdentity({
    units: [UNIT],
    platform: "linux",
    fileSystem: fixture.fileSystem,
    runCommand: fixture.runCommand,
    now: () => new Date("2026-09-26T12:00:00.000Z"),
    ...options,
  })
}

test("captures raw host identities and rechecks one coherent identity generation", async () => {
  const fixture = makeFixture()
  const captureResult = await capture(fixture)

  assert.equal(captureResult.schema, "chariox.path1-host-identity-capture/v1")
  assert.equal(captureResult.identity.bootId, BOOT_ID)
  assert.equal(captureResult.identity.machineId, MACHINE_ID)
  assert.deepEqual(captureResult.identity.release, {
    digest: fixture.digest,
    sourceCommit: "1".repeat(40),
    sourceTree: "2".repeat(40),
    instanceId: captureResult.observations.release.releaseRoot.instanceId,
  })
  assert.deepEqual(captureResult.identity.serviceInstances, [{ unit: UNIT, invocationId: "b".repeat(32) }])
  assert.deepEqual(captureResult.identity.processes, [`${BOOT_ID}:321:98765`])
  assert.deepEqual(captureResult.identity.stateInstances.map(({ path: value }) => value), [VAR_STATE, HOME_STATE])
  assert.equal(captureResult.observations.systemdUnits[0].mainPid, "321")
  assert.equal(captureResult.observations.systemdUnits[0].activeState, "active")
  assert.equal(captureResult.observations.processIdentities[0].startTimeTicks, "98765")
  assert.equal(captureResult.observations.release.activation.resolvedPath, `${fixture.releaseRoot}/usr/local/bin/chariox-kernel`)
  assert.equal(captureResult.observations.release.releaseRoot.uid, "0")
  assert.equal(captureResult.observations.release.manifestDigest, fixture.digest)
  assert.equal(captureResult.observations.release.manifest.sha256, fixture.digest)
  assert.equal(captureResult.observations.identityRecheck.bootId, BOOT_ID)
  assert.equal(captureResult.observations.identityRecheck.machineId, MACHINE_ID)
  assert.deepEqual(fixture.systemdCalls, [UNIT, UNIT])
  assert.equal("signatureVerified" in captureResult.identity.release, false)
  assert.equal("trustedBuilderPublicKey" in captureResult.observations.release, false)
})

test("fails closed when a requested service is inactive", async () => {
  const fixture = makeFixture()
  fixture.unitProperties.set(UNIT, { invocationId: "", mainPid: "0", activeState: "inactive" })
  await assert.rejects(
    capture(fixture),
    (error) => error instanceof Path1HostIdentityCaptureError && error.code === "systemd_unit_inactive",
  )
})

test("rejects unsupported platforms and units before probing", async () => {
  const fixture = makeFixture()
  await assert.rejects(capture(fixture, { platform: "darwin" }), (error) => error.code === "platform_unsupported")
  await assert.rejects(capture(fixture, { units: ["arbitrary.service"] }), (error) => error.code === "unit_not_allowed")
  assert.deepEqual(fixture.systemdCalls, [])
})

test("rejects activation redirected outside the selected release", async () => {
  const fixture = makeFixture()
  fixture.fileSystem.readlink = async (filePath) => {
    if (filePath === ACTIVATION) return "../../../tmp/chariox-kernel"
    if (filePath === CURRENT) return `releases/${fixture.digest.slice("sha256:".length)}`
    throw fsError("EINVAL", `not a symlink: ${filePath}`)
  }
  await assert.rejects(capture(fixture), (error) => error.code === "release_activation_unsafe")
})

test("rejects a manifest whose digest does not select the active release", async () => {
  const fixture = makeFixture()
  fixture.nodes.get(fixture.manifestPath).bytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    sourceCommit: "3".repeat(40),
    sourceTree: "4".repeat(40),
    artifacts: [],
  }))
  await assert.rejects(capture(fixture), (error) => error.code === "release_digest_mismatch")
})

test("requires root-owned non-writable release identity paths", async (t) => {
  for (const unsafe of [
    { uid: 1000, mode: 0o755, code: "unsafe_release_owner" },
    { uid: 0, mode: 0o775, code: "unsafe_release_mode" },
  ]) {
    await t.test(unsafe.code, async () => {
      const fixture = makeFixture()
      const originalLstat = fixture.fileSystem.lstat
      fixture.fileSystem.lstat = async (filePath, options) => {
        const stat = await originalLstat(filePath, options)
        if (filePath === fixture.releaseRoot) return { ...stat, uid: BigInt(unsafe.uid), mode: BigInt(unsafe.mode) }
        return stat
      }
      await assert.rejects(capture(fixture), (error) => error.code === unsafe.code)
    })
  }
})

test("rejects a boot ID change during capture", async () => {
  const fixture = makeFixture()
  const originalReadFile = fixture.fileSystem.readFile
  let bootReads = 0
  fixture.fileSystem.readFile = async (filePath) => {
    if (filePath === BOOT_PATH && ++bootReads === 2) return `${CHANGED_BOOT_ID}\n`
    return originalReadFile(filePath)
  }
  await assert.rejects(capture(fixture), (error) => error.code === "host_identity_changed")
})

test("rejects a machine ID change during capture", async () => {
  const fixture = makeFixture()
  const originalReadFile = fixture.fileSystem.readFile
  let machineReads = 0
  fixture.fileSystem.readFile = async (filePath) => {
    if (filePath === MACHINE_PATH && ++machineReads === 2) return `${"c".repeat(32)}\n`
    return originalReadFile(filePath)
  }
  await assert.rejects(capture(fixture), (error) => error.code === "host_identity_changed")
})

test("rejects a requested service identity change during capture", async () => {
  const fixture = makeFixture()
  const originalRunCommand = fixture.runCommand
  let calls = 0
  fixture.runCommand = async (...args) => {
    const result = await originalRunCommand(...args)
    if (++calls === 2) result.stdout = result.stdout.replace(`InvocationID=${"b".repeat(32)}`, `InvocationID=${"c".repeat(32)}`)
    return result
  }
  await assert.rejects(capture(fixture), (error) => error.code === "systemd_identity_changed")
})

test("rejects a MainPID start-time change during capture", async () => {
  const fixture = makeFixture()
  const originalReadFile = fixture.fileSystem.readFile
  let processReads = 0
  fixture.fileSystem.readFile = async (filePath) => {
    if (filePath === "/proc/321/stat" && ++processReads === 2) return processStat(321, "98766")
    return originalReadFile(filePath)
  }
  await assert.rejects(capture(fixture), (error) => error.code === "process_identity_changed")
})

test("rejects duplicate invocation identities across distinct requested units", async () => {
  const fixture = makeFixture()
  await assert.rejects(
    capture(fixture, { units: [UNIT, SECOND_UNIT] }),
    (error) => error.code === "systemd_identity_duplicate",
  )
})

test("rejects a state-directory replacement during capture", async () => {
  const fixture = makeFixture()
  const originalLstat = fixture.fileSystem.lstat
  let stateReads = 0
  fixture.fileSystem.lstat = async (filePath, options) => {
    const metadata = await originalLstat(filePath, options)
    if (filePath === VAR_STATE && ++stateReads === 2) return { ...metadata, ino: metadata.ino + 1n }
    return metadata
  }
  await assert.rejects(capture(fixture), (error) => error.code === "state_identity_changed")
})

test("rejects an active-release replacement during capture", async () => {
  const fixture = makeFixture()
  const originalLstat = fixture.fileSystem.lstat
  let releaseReads = 0
  fixture.fileSystem.lstat = async (filePath, options) => {
    const metadata = await originalLstat(filePath, options)
    if (filePath === fixture.releaseRoot && ++releaseReads === 2) return { ...metadata, ino: metadata.ino + 1n }
    return metadata
  }
  await assert.rejects(capture(fixture), (error) => error.code === "release_identity_changed")
})
