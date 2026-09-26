import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { link, lstat, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import test from "node:test"
import { correlatePath1ProviderCloud } from "./path1-provider-cloud-correlation.mjs"
import { offlineCorrelationScript } from "./path1-offline-correlation-test-fixture.mjs"

const SCRIPT = resolve(import.meta.dirname, "path1-provider-cloud-correlation.mjs")
const REPOSITORY_ROOT = resolve(import.meta.dirname, "../../..")
const SUCCESS_MESSAGE = "Provider and Cloud evidence correlated; no acceptance verdict.\n"
const FAILURE_MESSAGE = "Path-1 provider and Cloud correlation failed; no acceptance verdict.\n"

function runCli(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8" })
}

function cliArguments(paths) {
  return ["--before", paths.before, "--after", paths.after, "--cloud", paths.cloud, "--output", paths.output]
}

function assertCliFailure(result) {
  assert.equal(result.status, 1)
  assert.equal(result.stdout, "")
  assert.equal(result.stderr, FAILURE_MESSAGE)
}

async function temporaryCaptures(t) {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "path1-provider-cloud-correlation-")))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const values = fixtures()
  values.before.providerResponseBody = "DO-NOT-COPY-PROVIDER-BODY"
  values.after.action.untrustedErrorBody = "DO-NOT-COPY-ERROR"
  values.cloud.projectId = "DO-NOT-COPY-PROJECT"
  values.cloud.passed = true
  values.cloud.providerResponse = { authorization: "DO-NOT-COPY-HEADER" }
  const paths = { output: join(directory, "correlation.json") }
  const bytes = {}
  for (const kind of ["before", "after", "cloud"]) {
    paths[kind] = join(directory, `${kind}.json`)
    bytes[kind] = Buffer.from(`\n${JSON.stringify(values[kind], null, 2)}\n`)
    await writeFile(paths[kind], bytes[kind], { mode: 0o600 })
  }
  return { directory, paths, bytes, values }
}

const CAPTURED_BEFORE = "2026-09-26T04:59:30.000Z"
const REQUESTED_AT = "2026-09-26T05:00:00.000Z"
const COMPLETED_AT = "2026-09-26T05:01:00.000Z"
const CLOUD_CAPTURED_AT = "2026-09-26T05:01:02.000Z"
const CAPTURED_AFTER = "2026-09-26T05:01:05.000Z"

function providerServer(overrides = {}) {
  return {
    serverId: "1234",
    imageId: "400",
    serverTypeId: "70",
    locationId: "80",
    datacenterId: "90",
    ...overrides,
  }
}

function fixtures() {
  return {
    before: {
      schema: "chariox.path1-provider-rebuild-capture/v1",
      provider: "hetzner-cloud",
      kind: "before",
      capturedAt: CAPTURED_BEFORE,
      server: providerServer(),
    },
    after: {
      schema: "chariox.path1-provider-rebuild-capture/v1",
      provider: "hetzner-cloud",
      kind: "after",
      capturedAt: CAPTURED_AFTER,
      requestedAt: REQUESTED_AT,
      serverBefore: providerServer({ imageId: "500" }),
      action: {
        actionId: "88",
        command: "rebuild",
        status: "success",
        error: null,
        started: "2026-09-26T05:00:01Z",
        finished: "2026-09-26T05:00:40Z",
      },
      serverAfter: providerServer({ imageId: "500" }),
    },
    cloud: {
      schema: "chariox.path1-cloud-reimage-capture/v1",
      minimumProtocolVersion: 345,
      environmentId: "environment-1",
      operationId: "operation-1",
      receiptId: "receipt-1",
      receiptDigest: `sha256:${"a".repeat(64)}`,
      generation: 2,
      previousGeneration: 1,
      providerServerId: "1234",
      providerImageId: "500",
      rebuildActionId: "88",
      capturedAt: CLOUD_CAPTURED_AT,
      before: {
        bootId: "11111111-1111-1111-1111-111111111111",
        machineId: "1".repeat(32),
        observedAt: "2026-09-26T04:58:00.000Z",
      },
      after: {
        bootId: "22222222-2222-2222-2222-222222222222",
        machineId: "2".repeat(32),
      },
      requestedAt: REQUESTED_AT,
      completedAt: COMPLETED_AT,
    },
  }
}

test("correlates matching provider allocation, rebuild, and finalized Cloud receipt", () => {
  const input = fixtures()
  input.before.server.providerResponseBody = "DO-NOT-COPY-PROVIDER-BODY"
  input.after.action.untrustedErrorBody = "DO-NOT-COPY-ERROR"
  input.cloud.projectId = "caller-supplied-project"
  input.cloud.passed = true
  input.cloud.providerResponse = { authorization: "DO-NOT-COPY-HEADER" }

  const result = correlatePath1ProviderCloud(input)
  assert.deepEqual(result, {
    schema: "chariox.path1-provider-cloud-correlation/v1",
    cloud: {
      environmentId: "environment-1",
      operationId: "operation-1",
      receiptId: "receipt-1",
      receiptDigest: `sha256:${"a".repeat(64)}`,
      generation: 2,
      previousGeneration: 1,
    },
    provider: {
      serverId: "1234",
      sourceImageId: "400",
      targetImageId: "500",
      serverTypeId: "70",
      locationId: "80",
      datacenterId: "90",
      action: {
        actionId: "88",
        command: "rebuild",
        status: "success",
        error: null,
        started: "2026-09-26T05:00:01Z",
        finished: "2026-09-26T05:00:40Z",
      },
    },
    times: {
      providerPreparedAt: CAPTURED_BEFORE,
      requestedAt: REQUESTED_AT,
      actionStartedAt: "2026-09-26T05:00:01Z",
      actionFinishedAt: "2026-09-26T05:00:40Z",
      completedAt: COMPLETED_AT,
      cloudCapturedAt: CLOUD_CAPTURED_AT,
      providerAfterCapturedAt: CAPTURED_AFTER,
    },
    providerProjectProvenance: {
      status: "unexecuted",
      reason: "hetzner_captures_do_not_expose_project_identity",
    },
  })
  assert.ok(!JSON.stringify(result).includes("DO-NOT-COPY"))
  assert.ok(!JSON.stringify(result).includes("caller-supplied-project"))
  assert.equal(Object.hasOwn(result, "passed"), false)
  assert.equal(Object.hasOwn(result.provider, "projectId"), false)
})

for (const [name, mutate, message] of [
  ["unsupported provider schema", (x) => { x.before.schema = "future" }, /wrong schema or kind/],
  ["wrong provider kind", (x) => { x.after.kind = "before" }, /wrong schema or kind/],
  ["unsupported Cloud schema", (x) => { x.cloud.schema = "future" }, /unsupported Cloud capture schema/],
  ["missing owner-bound receipt identity", (x) => { delete x.cloud.receiptId }, /owner-bound finalized receipt identity/],
  ["bad receipt digest", (x) => { x.cloud.receiptDigest = "not-a-digest" }, /owner-bound finalized receipt identity/],
  ["wrong generation binding", (x) => { x.cloud.previousGeneration = 0 }, /owner-bound finalized receipt identity/],
  ["unrotated Cloud host identity", (x) => { x.cloud.after.machineId = x.cloud.before.machineId }, /rotated host identities/],
  ["Cloud baseline after request", (x) => { x.cloud.before.observedAt = "2026-09-26T05:00:01.000Z" }, /Cloud baseline, requested, and completed times/],
  ["Cloud completion before request", (x) => { x.cloud.completedAt = "2026-09-26T04:59:59.000Z" }, /Cloud baseline, requested, and completed times/],
  ["Cloud capture before completion", (x) => { x.cloud.capturedAt = "2026-09-26T05:00:59.000Z" }, /Cloud baseline, requested, and completed times/],
  ["wrong Cloud-bound server", (x) => { x.cloud.providerServerId = "9999" }, /same Cloud-bound server/],
  ["wrong before server", (x) => { x.before.server.serverId = "9999" }, /same Cloud-bound server/],
  ["wrong after pre-history server", (x) => { x.after.serverBefore.serverId = "9999" }, /same Cloud-bound server/],
  ["wrong after post-history server", (x) => { x.after.serverAfter.serverId = "9999" }, /same Cloud-bound server/],
  ["source image equals target image", (x) => { x.before.server.imageId = "500" }, /did not change/],
  ["wrong after target image", (x) => { x.after.serverBefore.imageId = "501" }, /Cloud target image/],
  ["wrong post-history target image", (x) => { x.after.serverAfter.imageId = "501" }, /Cloud target image/],
  ["wrong Cloud action id", (x) => { x.cloud.rebuildActionId = "89" }, /action id does not match/],
  ["wrong action command", (x) => { x.after.action.command = "reboot" }, /not a rebuild/],
  ["running action", (x) => { x.after.action.status = "running" }, /not successful/],
  ["non-null action error", (x) => { x.after.action.error = { message: "private" } }, /error is not null/],
  ["missing action error field", (x) => { delete x.after.action.error }, /error is not null/],
  ["provider requestedAt mismatch", (x) => { x.after.requestedAt = "2026-09-26T04:59:00.000Z" }, /requestedAt values do not match/],
  ["provider preparation after request", (x) => { x.before.capturedAt = "2026-09-26T05:00:00.001Z" }, /times are out of order/],
  ["action start before request", (x) => { x.after.action.started = "2026-09-26T04:59:59Z" }, /times are out of order/],
  ["action finishes before it starts", (x) => { x.after.action.finished = "2026-09-26T05:00:00Z" }, /times are out of order/],
  ["action finishes after Cloud completion", (x) => { x.after.action.finished = "2026-09-26T05:01:01Z" }, /times are out of order/],
  ["sub-millisecond action finish after Cloud completion", (x) => { x.after.action.finished = "2026-09-26T05:01:00.000001Z" }, /times are out of order/],
  ["provider after capture predates completion", (x) => { x.after.capturedAt = "2026-09-26T05:00:59.000Z" }, /times are out of order/],
  ["changed server type", (x) => { x.after.serverAfter.serverTypeId = "71" }, /server type, location, or datacenter changed/],
  ["changed location", (x) => { x.after.serverBefore.locationId = "81" }, /server type, location, or datacenter changed/],
  ["changed datacenter", (x) => { x.after.serverAfter.datacenterId = "91" }, /server type, location, or datacenter changed/],
  ["missing allocation identity", (x) => { delete x.before.server.locationId }, /location id is missing or invalid/],
  ["noncanonical provider capture time", (x) => { x.after.capturedAt = "2026-09-26T05:01:05Z" }, /canonical UTC timestamp/],
  ["missing action", (x) => { delete x.after.action }, /provider action is invalid/],
]) {
  test(`rejects ${name}`, () => {
    const input = fixtures()
    mutate(input)
    assert.throws(() => correlatePath1ProviderCloud(input), message)
  })
}

test("CLI binds exact input bytes and writes allowlisted private evidence", async (t) => {
  const input = await temporaryCaptures(t)
  const result = runCli(cliArguments(input.paths))
  assert.equal(result.status, 0, result.stderr || result.stdout)
  assert.equal(result.stdout, SUCCESS_MESSAGE)
  assert.equal(result.stderr, "")

  const output = JSON.parse(await readFile(input.paths.output, "utf8"))
  const { evidence, ...correlation } = output
  assert.deepEqual(correlation, correlatePath1ProviderCloud(input.values))
  for (const kind of ["before", "after", "cloud"]) {
    assert.deepEqual(evidence[kind], {
      path: input.paths[kind],
      sha256: `sha256:${createHash("sha256").update(input.bytes[kind]).digest("hex")}`,
    })
  }
  assert.equal((await stat(input.paths.output)).mode & 0o777, 0o600)
  const encoded = JSON.stringify(output)
  for (const secret of ["DO-NOT-COPY-PROVIDER-BODY", "DO-NOT-COPY-ERROR", "DO-NOT-COPY-PROJECT", "DO-NOT-COPY-HEADER"]) {
    assert.equal(encoded.includes(secret), false)
  }
  assert.equal(Object.hasOwn(output.provider, "projectId"), false)
  assert.equal(Object.hasOwn(output, "passed"), false)
})

test("CLI rejects missing, duplicate, unknown, empty, and aliased arguments", async (t) => {
  const input = await temporaryCaptures(t)
  const valid = cliArguments(input.paths)
  const cases = [
    ["missing", valid.slice(0, -2)],
    ["unknown", valid.map(value => value === "--cloud" ? "--unknown" : value)],
    ["duplicate", ["--before", input.paths.before, "--before", input.paths.after,
      "--cloud", input.paths.cloud, "--output", input.paths.output]],
    ["empty", valid.map((value, index) => index === 1 ? "" : value)],
    ["aliased input", ["--before", input.paths.before, "--after", input.paths.before,
      "--cloud", input.paths.cloud, "--output", input.paths.output]],
    ["input used as output", ["--before", input.paths.before, "--after", input.paths.after,
      "--cloud", input.paths.cloud, "--output", input.paths.before]],
    ["normalized input/output alias", ["--before", input.paths.before, "--after", input.paths.after,
      "--cloud", input.paths.cloud, "--output", join(input.directory, "nested", "..", "before.json")]],
  ]
  for (const [name, args] of cases) {
    assertCliFailure(runCli(args), name)
  }
})

test("CLI rejects aliased hard-link inputs and symlink inputs", async (t) => {
  const input = await temporaryCaptures(t)
  const hardLink = join(input.directory, "after-hard-link.json")
  await link(input.paths.before, hardLink)
  const hardLinked = { ...input.paths, after: hardLink, output: join(input.directory, "hardlink.json") }
  assertCliFailure(runCli(cliArguments(hardLinked)))

  const inputLink = join(input.directory, "before-link.json")
  await symlink(input.paths.before, inputLink)
  const linkedInput = { ...input.paths, before: inputLink, output: join(input.directory, "symlink-input.json") }
  assertCliFailure(runCli(cliArguments(linkedInput)))

  const parentLink = join(input.directory, "linked-directory")
  await symlink(input.directory, parentLink)
  const linkedParent = { ...input.paths, before: join(parentLink, "before.json"), output: join(input.directory, "symlink-parent.json") }
  assertCliFailure(runCli(cliArguments(linkedParent)))
})

test("CLI rejects oversized evidence inputs", async (t) => {
  const input = await temporaryCaptures(t)
  const oversized = join(input.directory, "oversized.json")
  await writeFile(oversized, Buffer.alloc(1024 * 1024 + 1, 0x20), { mode: 0o600 })
  const paths = { ...input.paths, before: oversized }
  assertCliFailure(runCli(cliArguments(paths)))
  await assert.rejects(lstat(paths.output))
})

test("CLI rejects repository, symlink, and overwrite outputs", async (t) => {
  const input = await temporaryCaptures(t)
  const repositoryOutput = join(REPOSITORY_ROOT, ".path1-provider-cloud-correlation-forbidden.json")
  const insideRepo = { ...input.paths, output: repositoryOutput }
  assertCliFailure(runCli(cliArguments(insideRepo)))
  await assert.rejects(lstat(repositoryOutput))

  const target = join(input.directory, "symlink-target.json")
  await writeFile(target, "leave-existing-target\n", { mode: 0o600 })
  const linkedOutput = join(input.directory, "symlink-output.json")
  await symlink(target, linkedOutput)
  const viaLink = { ...input.paths, output: linkedOutput }
  assertCliFailure(runCli(cliArguments(viaLink)))

  const linkedDirectory = join(input.directory, "output-parent-link")
  const realDirectory = join(input.directory, "real-output-parent")
  await mkdir(realDirectory)
  await symlink(realDirectory, linkedDirectory)
  const viaParentLink = { ...input.paths, output: join(linkedDirectory, "evidence.json") }
  assertCliFailure(runCli(cliArguments(viaParentLink)))
  assert.equal(await readFile(target, "utf8"), "leave-existing-target\n")
})

test("CLI output is exclusive and a rejected overwrite preserves the original", async (t) => {
  const input = await temporaryCaptures(t)
  assert.equal(runCli(cliArguments(input.paths)).status, 0)
  const original = await readFile(input.paths.output)
  assertCliFailure(runCli(cliArguments(input.paths)))
  assert.deepEqual(await readFile(input.paths.output), original)
  assert.equal((await stat(input.paths.output)).mode & 0o777, 0o600)
})

test("CLI runs the real offline module graph without generated client files", async (t) => {
  const input = await temporaryCaptures(t)
  const script = await offlineCorrelationScript(input.directory, "path1-provider-cloud-correlation.mjs")
  const result = spawnSync(process.execPath, [script, ...cliArguments(input.paths)], { encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, SUCCESS_MESSAGE)
  const { evidence, ...correlation } = JSON.parse(await readFile(input.paths.output, "utf8"))
  assert.deepEqual(correlation, correlatePath1ProviderCloud(input.values))
  assert.equal(Object.keys(evidence).length, 3)
})
