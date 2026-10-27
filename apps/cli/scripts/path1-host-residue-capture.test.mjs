import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { copyFile, mkdir, mkdtemp, readFile, realpath, rm, stat as statFile, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import test from "node:test"
import { fileURLToPath, pathToFileURL } from "node:url"
import { offlineCorrelationScript } from "./path1-offline-correlation-test-fixture.mjs"
import {
  capturePath1HostResidue,
  parsePath1HostResidueArguments,
  runPath1HostResidueCaptureCli,
} from "./path1-host-residue-capture.mjs"

const bootId = "11111111-2222-3333-4444-555555555555"
const machineId = "a".repeat(32)
const invocationId = "b".repeat(32)
const processPid = 321
const processStart = "1000"
const releaseDigest = `sha256:${"c".repeat(64)}`
const releasePath = `/usr/lib/chariox/releases/${"c".repeat(64)}`
const statePaths = ["/var/lib/chariox", "/home/chariox"]
const unit = "chariox-path1-managed-bootstrap.service"
const baseBefore = {
  schema: "chariox.path1-host-identity-capture/v1",
  identity: {
    bootId,
    machineId,
    release: { digest: releaseDigest, instanceId: "stat:1:50" },
    serviceInstances: [{ unit, invocationId }],
    processes: [`${bootId}:${processPid}:${processStart}`],
    stateInstances: [
      { path: statePaths[0], instanceId: "stat:2:20" },
      { path: statePaths[1], instanceId: "stat:3:30" },
    ],
  },
}

function stat(kind, dev, ino) {
  return {
    dev: BigInt(dev), ino: BigInt(ino), mode: BigInt(kind === "symlink" ? 0o120777 : kind === "directory" ? 0o40755 : 0o100644),
    uid: 0n, gid: 0n,
    isSymbolicLink: () => kind === "symlink",
    isDirectory: () => kind === "directory",
    isFile: () => kind === "file",
  }
}

function procStat(pid = processPid, startTimeTicks = processStart) {
  const fields = Array(20).fill("0")
  fields[0] = "S"
  fields[1] = "1"
  fields[19] = startTimeTicks
  return `${pid} (chariox-kernel) ${fields.join(" ")}\n`
}

function systemdOutput({ id = unit, invocation = invocationId, pid = processPid, active = "active" } = {}) {
  return `Id=${id}\nInvocationID=${invocation}\nMainPID=${pid}\nActiveState=${active}\n`
}

function fixture({ bootReads, processReads, stateOverrides = {}, releaseOverride, readFailures = {}, serviceReads } = {}) {
  const calls = { reads: {}, lstats: {}, realpaths: {}, commands: 0 }
  const sequenceValue = (values, index, fallback) => Array.isArray(values) ? (values[Math.min(index, values.length - 1)] ?? fallback) : values ?? fallback
  const fileSystem = {
    async readFile(path) {
      const index = calls.reads[path] ?? 0
      calls.reads[path] = index + 1
      if (readFailures[path]) throw Object.assign(new Error("fixture read error"), { code: readFailures[path] })
      if (path === "/proc/sys/kernel/random/boot_id") return sequenceValue(bootReads, index, bootId)
      if (path === "/etc/machine-id") return machineId
      if (path === `/proc/${processPid}/stat`) {
        const value = sequenceValue(processReads, index, procStat())
        if (value instanceof Error) throw value
        return value
      }
      throw Object.assign(new Error("unexpected path"), { code: "ENOENT" })
    },
    async lstat(path) {
      const index = calls.lstats[path] ?? 0
      calls.lstats[path] = index + 1
      if (stateOverrides[path]) {
        const value = stateOverrides[path](index)
        if (value instanceof Error) throw value
        return value
      }
      if (path === releasePath && releaseOverride) {
        const value = releaseOverride(index)
        if (value instanceof Error) throw value
        return value
      }
      if (path === statePaths[0]) return stat("directory", 2, 20)
      if (path === statePaths[1]) return stat("directory", 3, 30)
      if (path === releasePath) return stat("directory", 1, 50)
      throw Object.assign(new Error("fixture missing path"), { code: "ENOENT" })
    },
    async realpath(path) {
      calls.realpaths[path] = (calls.realpaths[path] ?? 0) + 1
      return path
    },
  }
  const runCommand = async (executable, args) => {
    assert.equal(executable, "/usr/bin/systemctl")
    assert.deepEqual(args, ["show", "--no-pager", "--property=Id", "--property=InvocationID", "--property=MainPID", "--property=ActiveState", unit])
    const index = calls.commands++
    return { stdout: sequenceValue(serviceReads, index, systemdOutput()), stderr: "", exitCode: 0 }
  }
  const capture = () => capturePath1HostResidue({
    beforeCapture: baseBefore,
    platform: "linux",
    fileSystem,
    runCommand,
    now: () => "2026-09-26T00:00:00.000Z",
  })
  return { calls, capture }
}

test("CLI arguments require one retained input and absolute output", () => {
  assert.deepEqual(parsePath1HostResidueArguments(["--before", "/tmp/before.json", "--output", "/tmp/after.json"]), {
    before: "/tmp/before.json", output: "/tmp/after.json",
  })
  assert.deepEqual(parsePath1HostResidueArguments(["--output", "/tmp/after.json", "--before", "/tmp/before.json"]), {
    before: "/tmp/before.json", output: "/tmp/after.json",
  })
  assert.throws(() => parsePath1HostResidueArguments(["--before", "/tmp/before.json", "--output", "relative.json"]), /absolute external/)
  assert.throws(() => parsePath1HostResidueArguments(["--before", "/tmp/a.json", "--before", "/tmp/b.json"]), /invalid Path-1/)
})

test("CLI runner uses the injected probe seam and binds input evidence without a verdict", async () => {
  const inputPath = "/tmp/path1-before.json"
  const outputPath = "/tmp/path1-residue.json"
  const inputEvidence = { path: inputPath, sha256: `sha256:${"e".repeat(64)}` }
  let written
  const output = await runPath1HostResidueCaptureCli(["--before", inputPath, "--output", outputPath], {
    readInput: async (file) => {
      assert.equal(file, inputPath)
      return { capture: baseBefore, evidence: inputEvidence }
    },
    validateOutput: async (file) => file,
    capture: async ({ beforeCapture }) => {
      assert.deepEqual(beforeCapture, baseBefore)
      return { schema: "chariox.path1-host-residue-capture/v1", observations: {} }
    },
    writeOutput: async (file, capture) => {
      assert.equal(file, outputPath)
      written = capture
    },
  })
  assert.deepEqual(output.retainedBeforeEvidence, inputEvidence)
  assert.deepEqual(written, output)
  assert.equal("mpVerdict" in output, false)
})

test("input and output path overlap is rejected before writing", async () => {
  const samePath = "/tmp/path1-same.json"
  await assert.rejects(runPath1HostResidueCaptureCli(["--before", samePath, "--output", samePath], {
    readInput: async () => ({ capture: baseBefore, evidence: { path: samePath, sha256: `sha256:${"e".repeat(64)}` } }),
    validateOutput: async (file) => file,
    capture: async () => assert.fail("probe must not run for overlapping paths"),
    writeOutput: async () => assert.fail("must not write overlapping paths"),
  }), (error) => error.code === "input_output_overlap")
})

test("unchanged exact host identities produce bounded raw observations", async () => {
  const { calls, capture } = fixture()
  const result = await capture()
  assert.equal(result.schema, "chariox.path1-host-residue-capture/v1")
  assert.equal(result.observations.host.bootId.status, "present")
  assert.equal(result.observations.host.machineId.status, "present")
  assert.equal(result.observations.services[0].status, "present")
  assert.equal(result.observations.processes[0].status, "present")
  assert.deepEqual(result.observations.stateDirectories.map(({ status }) => status), ["present", "present"])
  assert.equal(result.observations.releaseRoot.status, "present")
  assert.equal(calls.commands, 2)
  assert.match(result.limitations[0], /cannot prove there are no copies/)
  assert.equal("mpVerdict" in result, false)
})

test("replaced state and release inodes are observed as changed", async () => {
  const { capture: stateCapture } = fixture({ stateOverrides: { [statePaths[0]]: () => stat("directory", 2, 99) } })
  const stateResult = await stateCapture()
  assert.equal(stateResult.observations.stateDirectories[0].status, "changed")
  const { capture: releaseCapture } = fixture({ releaseOverride: () => stat("directory", 1, 99) })
  const releaseResult = await releaseCapture()
  assert.equal(releaseResult.observations.releaseRoot.status, "changed")
})

test("missing exact state and process paths are derived from ENOENT reads", async () => {
  const missing = Object.assign(new Error("missing"), { code: "ENOENT" })
  const { capture } = fixture({
    stateOverrides: { [statePaths[0]]: () => missing },
    processReads: [missing, missing],
  })
  const result = await capture()
  assert.equal(result.observations.stateDirectories[0].status, "missing")
  assert.equal(result.observations.processes[0].status, "missing")
  assert.equal(result.observations.processes[0].observed, null)
})

test("permission errors fail as unknown instead of becoming missing", async () => {
  const { capture } = fixture({ stateOverrides: { [statePaths[0]]: () => Object.assign(new Error("denied"), { code: "EACCES" }) } })
  await assert.rejects(capture(), (error) => error.code === "probe_read_failed")
})

test("a symlink at an exact state path is changed and is never followed", async () => {
  const { calls, capture } = fixture({ stateOverrides: { [statePaths[0]]: () => stat("symlink", 2, 99) } })
  const result = await capture()
  assert.equal(result.observations.stateDirectories[0].status, "changed")
  assert.equal(result.observations.stateDirectories[0].observed.kind, "symlink")
  assert.equal(calls.realpaths[statePaths[0]] ?? 0, 0)
})

test("a reused PID with a different start time is changed, never old-process presence", async () => {
  const reused = procStat(processPid, "2000")
  const { capture } = fixture({ processReads: [reused, reused] })
  const result = await capture()
  assert.equal(result.observations.processes[0].status, "changed")
  assert.equal(result.observations.processes[0].observed.startTimeTicks, "2000")
  assert.match(result.observations.processes[0].limitation, /reused PID/)
})

test("a boot identity change during collection rejects the mixed capture", async () => {
  const nextBootId = "99999999-8888-7777-6666-555555555555"
  const { capture } = fixture({ bootReads: [bootId, nextBootId] })
  await assert.rejects(capture(), (error) => error.code === "capture_generation_changed")
})

test("a stable replacement systemd invocation is observed as changed", async () => {
  const replacement = systemdOutput({ invocation: "d".repeat(32), pid: processPid + 1 })
  const { capture } = fixture({ serviceReads: [replacement, replacement] })
  const result = await capture()
  assert.equal(result.observations.services[0].status, "changed")
  assert.equal(result.observations.services[0].observed.invocationId, "d".repeat(32))
})

test("a systemd MainPID identity change during collection rejects the mixed capture", async () => {
  const { capture } = fixture({ serviceReads: [systemdOutput(), systemdOutput({ pid: processPid + 1 })] })
  await assert.rejects(capture(), (error) => error.code === "capture_generation_changed")
})

test("a process start-time change during collection rejects the mixed capture", async () => {
  const { capture } = fixture({ processReads: [procStat(), procStat(processPid, "2000")] })
  await assert.rejects(capture(), (error) => error.code === "capture_generation_changed")
})

test("Linux platform is an explicit injectable precondition", async () => {
  await assert.rejects(capturePath1HostResidue({ beforeCapture: baseBefore, platform: "darwin" }), (error) => error.code === "unsupported_platform")
})

test("default CLI reader and output helpers work in the actual offline module graph", async (t) => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "path1-residue-offline-")))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const correlationScript = await offlineCorrelationScript(directory, "path1-host-cloud-correlation.mjs")
  const scriptsDirectory = dirname(correlationScript)
  const repository = resolve(scriptsDirectory, "../../..")
  const residueScript = join(scriptsDirectory, "path1-host-residue-capture.mjs")
  await copyFile(fileURLToPath(new URL("./path1-host-residue-capture.mjs", import.meta.url)), residueScript)
  const residueCli = await import(pathToFileURL(residueScript).href)

  const beforePath = join(directory, "before.json")
  const beforeBytes = Buffer.from(`${JSON.stringify(baseBefore, null, 2)}\n`)
  await writeFile(beforePath, beforeBytes)
  const outputPath = join(directory, "residue.json")
  const fakeObservation = {
    schema: "chariox.path1-host-residue-capture/v1",
    capturedAt: "2026-09-26T00:00:00.000Z",
    observations: { fixtureOnly: { status: "present" } },
    limitations: ["fixture observation; no host probe executed"],
  }
  let probeCalls = 0
  const fakeProbe = async ({ beforeCapture }) => {
    probeCalls += 1
    assert.deepEqual(beforeCapture, baseBefore)
    return fakeObservation
  }
  const args = (before, output) => ["--before", before, "--output", output]
  const run = (before, output) => residueCli.runPath1HostResidueCaptureCli(args(before, output), { capture: fakeProbe })

  const result = await run(beforePath, outputPath)
  const expectedEvidence = {
    path: beforePath,
    sha256: `sha256:${createHash("sha256").update(beforeBytes).digest("hex")}`,
  }
  assert.deepEqual(result.retainedBeforeEvidence, expectedEvidence)
  assert.deepEqual(JSON.parse(await readFile(outputPath, "utf8")), result)
  assert.equal((await statFile(outputPath)).mode & 0o777, 0o600)
  assert.equal(probeCalls, 1)
  assert.equal(Object.hasOwn(result, "mpVerdict"), false)
  assert.equal(Object.hasOwn(result, "passed"), false)

  const assertRejectedBeforeProbe = async (before, output, pattern) => {
    const count = probeCalls
    await assert.rejects(run(before, output), pattern)
    assert.equal(probeCalls, count)
  }
  const malformedPath = join(directory, "malformed.json")
  await writeFile(malformedPath, "{")
  await assertRejectedBeforeProbe(malformedPath, join(directory, "malformed-output.json"), SyntaxError)

  const oversizedPath = join(directory, "oversized.json")
  await writeFile(oversizedPath, "x".repeat(1024 * 1024 + 1))
  await assertRejectedBeforeProbe(oversizedPath, join(directory, "oversized-output.json"), /bounded regular file/)

  const inputLink = join(directory, "before-alias.json")
  await symlink(beforePath, inputLink)
  await assertRejectedBeforeProbe(inputLink, join(directory, "symlink-output.json"), /no symlinks/)
  const parentLink = join(directory, "input-parent-alias")
  await symlink(directory, parentLink)
  await assertRejectedBeforeProbe(join(parentLink, "before.json"), join(directory, "parent-output.json"), /no symlinks/)

  await assertRejectedBeforeProbe(beforePath, join(repository, "residue-inside-repository.json"), /outside the repository/)
  const outputAlias = join(directory, "output-alias")
  await symlink(directory, outputAlias)
  await assertRejectedBeforeProbe(beforePath, join(outputAlias, "aliased-output.json"), /symlink/)
  await assertRejectedBeforeProbe(beforePath, outputPath, /new file/)
  await assertRejectedBeforeProbe(beforePath, beforePath, /new file/)
})
