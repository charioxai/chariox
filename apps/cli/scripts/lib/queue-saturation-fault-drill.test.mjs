import assert from "node:assert/strict"
import test from "node:test"

import {
  QUEUE_SATURATION_FAULT_CASE_IDS,
  QUEUE_SATURATION_FAULT_TEST_NAME,
  buildQueueSaturationFaultCargoArgs,
  parseQueueSaturationFaultProbe,
} from "./queue-saturation-fault-drill.mjs"

test("queue saturation drill runs only the exact relay library probe", () => {
  assert.deepEqual(buildQueueSaturationFaultCargoArgs(), [
    "test",
    "-p",
    "chariox-relay",
    "--lib",
    QUEUE_SATURATION_FAULT_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
  assert.deepEqual(QUEUE_SATURATION_FAULT_CASE_IDS, [
    "fault.queue-saturation",
    "cleanup.resources",
  ])
})

test("queue saturation drill requires bounded backpressure and healthy readers", () => {
  const probe = {
    schema: "chariox.queue_saturation_fault_probe.v1",
    queueLimitReachedDeterministically: true,
    clientRequestRejectedRetryably: true,
    peerRequestRejectedRetryably: true,
    slowSubscriberIsolated: true,
    healthyReaderPreserved: true,
    readerLaneRemainedLive: true,
    backpressureMetricsRecorded: true,
  }
  assert.deepEqual(parseQueueSaturationFaultProbe(`noise\n${JSON.stringify(probe)}\n`), probe)
  assert.throws(
    () => parseQueueSaturationFaultProbe(JSON.stringify({ ...probe, readerLaneRemainedLive: false })),
    /readerLaneRemainedLive must be true/,
  )
})
