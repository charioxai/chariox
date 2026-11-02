import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_DEPLOYMENT_PRESETS,
  appendRemoteKernelPassthrough,
  drillDeploymentPresetMetadata,
  remoteKernelPassthroughMetadata,
  isKnownDrillDeploymentPreset,
  parseRemoteKernelPassthroughArg,
  validateDrillDeploymentPreset,
  validateDrillDeploymentPresets,
} from "./drill-environment-presets.mjs"

test("parses remote kernel passthrough flags", () => {
  assert.deepEqual(parseRemoteKernelPassthroughArg(["--remote-kernel-host", "root@example"], 0), {
    args: ["--remote-kernel-host", "root@example"],
    nextIndex: 1,
  })
  assert.deepEqual(parseRemoteKernelPassthroughArg(["--remote-kernel-repo=/tmp/chariox"], 0), {
    args: ["--remote-kernel-repo", "/tmp/chariox"],
    nextIndex: 0,
  })
  assert.equal(parseRemoteKernelPassthroughArg(["--include-remote-kernel"], 0), null)
  assert.throws(() => parseRemoteKernelPassthroughArg(["--remote-kernel-key"], 0), /requires a value/)
})

test("appends remote kernel passthrough only for remote kernel scenarios", () => {
  const passthrough = ["--remote-kernel-host", "root@example"]

  assert.deepEqual(
    appendRemoteKernelPassthrough(["drill.mjs"], { requires: ["remote-kernel"] }, passthrough),
    ["drill.mjs", "--remote-kernel-host", "root@example"],
  )
  assert.deepEqual(
    appendRemoteKernelPassthrough(["drill.mjs"], { requires: ["remote"] }, passthrough),
    ["drill.mjs"],
  )
})

test("summarizes remote kernel passthrough without leaking values", () => {
  assert.deepEqual(
    remoteKernelPassthroughMetadata(["--remote-kernel-host", "root@example", "--remote-kernel-key", "/tmp/key"]),
    {
      hasRemoteKernelHostOverride: true,
      hasRemoteKernelSshIdentityOverride: true,
      hasRemoteKernelRepoOverride: false,
    },
  )
})

test("summarizes deployment presets without leaking machine details", () => {
  assert.deepEqual(DRILL_DEPLOYMENT_PRESETS, [
    "hosted-cloud",
    "local",
    "remote-kernel",
    "same-host-remote",
    "self-hosted-relay",
  ])
  assert.equal(isKnownDrillDeploymentPreset("local"), true)
  assert.equal(isKnownDrillDeploymentPreset("same-host-remtoe"), false)
  assert.doesNotThrow(() => validateDrillDeploymentPreset("same-host-remote", "preset"))
  assert.throws(
    () => validateDrillDeploymentPreset("same-host-remtoe", "preset"),
    /preset has unknown deployment preset "same-host-remtoe"/,
  )
  assert.throws(
    () => validateDrillDeploymentPreset("same-host-remtoe", "required deployment presets", {
      message: (preset) => `unknown required deployment preset: ${preset}`,
    }),
    /unknown required deployment preset: same-host-remtoe/,
  )
  assert.doesNotThrow(() => validateDrillDeploymentPresets(["local", "same-host-remote"], "test presets"))
  assert.throws(
    () => validateDrillDeploymentPresets(["same-host-remtoe"], "test presets"),
    /test presets\[0\] has unknown deployment preset "same-host-remtoe"/,
  )
  assert.deepEqual(
    drillDeploymentPresetMetadata(["local", "self-hosted-relay", "remote-kernel", "local"], {
      remoteKernelPassthrough: ["--remote-kernel-host", "root@example", "--remote-kernel-repo", "/tmp/private"],
    }),
    {
      deploymentPresetCount: 3,
      deploymentPresets: "local,remote-kernel,self-hosted-relay",
      includesRemoteKernel: true,
      includesHostedCloud: false,
      includesLocal: true,
      includesSameHostRemote: false,
      includesSelfHostedRelay: true,
      hasRemoteKernelHostOverride: true,
      hasRemoteKernelSshIdentityOverride: false,
      hasRemoteKernelRepoOverride: true,
    },
  )
  assert.throws(
    () => drillDeploymentPresetMetadata(["unknown"]),
    /drill deployment presets\[0\] has unknown deployment preset "unknown"/,
  )
})
