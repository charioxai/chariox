import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  DISPLAY_FAULT_CASE_IDS,
  buildDisplayFaultDockerArgs,
  validateDisplayFaultProbe,
} from "./slice-display-fault-drill.mjs"

test("display fault container is bounded, offline, unprivileged, and current-source", () => {
  const args = buildDisplayFaultDockerArgs({
    containerName: "chariox-slice-display-fault-test",
    image: "chariox-slice-linux:test",
    sourceRoot: "/work/chariox/apps/kernel/slice-linux-docker",
  })

  assert.deepEqual(args.slice(0, 4), ["run", "--rm", "--name", "chariox-slice-display-fault-test"])
  assert.ok(args.includes("--network") && args.includes("none"))
  assert.ok(args.includes("--memory") && args.includes("1g"))
  assert.ok(args.includes("--memory-swap") && args.includes("1g"))
  assert.ok(args.includes("--cpus") && args.includes("1"))
  assert.ok(args.includes("--pids-limit") && args.includes("256"))
  assert.ok(args.includes("--read-only"))
  assert.ok(args.includes("no-new-privileges"))
  assert.ok(args.includes("type=bind,src=/work/chariox/apps/kernel/slice-linux-docker,dst=/drill,readonly"))
  assert.deepEqual(args.slice(-3), [
    "chariox-slice-linux:test",
    "/drill/validate-slice-viewer.py",
    "--json",
  ])
})

test("display fault probe requires streamer and browser recovery plus cleanup", () => {
  assert.deepEqual(DISPLAY_FAULT_CASE_IDS, [
    "fault.streamer-crash",
    "fault.browser-crash",
    "cleanup.resources",
  ])
  const probe = {
    schema: "chariox.slice_display_fault_probe.v1",
    streamer: {
      crashDetected: true,
      browserRemainedAvailable: true,
      recoveredOnce: true,
    },
    browser: {
      crashDetected: true,
      streamerRemainedAvailable: true,
      recoveredOnce: true,
      profileStatePreserved: true,
    },
    cleanup: {
      displaySocketRemoved: true,
      portsReleased: true,
    },
  }
  assert.equal(validateDisplayFaultProbe(probe), probe)

  for (const field of ["browserRemainedAvailable", "recoveredOnce"]) {
    const broken = structuredClone(probe)
    broken.streamer[field] = false
    assert.throws(() => validateDisplayFaultProbe(broken), new RegExp(field))
  }
  const staleProfile = structuredClone(probe)
  staleProfile.browser.profileStatePreserved = false
  assert.throws(() => validateDisplayFaultProbe(staleProfile), /profileStatePreserved/)
  const leakedPort = structuredClone(probe)
  leakedPort.cleanup.portsReleased = false
  assert.throws(() => validateDisplayFaultProbe(leakedPort), /portsReleased/)
  assert.throws(
    () => validateDisplayFaultProbe({ ...probe, extra: path.join("/tmp", "unexpected") }),
    /unexpected keys/,
  )
})
