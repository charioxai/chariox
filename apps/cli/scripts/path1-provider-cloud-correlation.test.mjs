import assert from "node:assert/strict"
import test from "node:test"
import { correlatePath1ProviderCloud } from "./path1-provider-cloud-correlation.mjs"

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
