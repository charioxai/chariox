import assert from "node:assert/strict"
import test from "node:test"

import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./drill-validation-gate-args.mjs"

test("parses validation gate requirement arguments", () => {
  const options = validationGateRequirementOptionDefaults()
  let index = parseValidationGateRequirementArg(["--preset", "workspace-live-sync"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-platform-coverage-area=runtime-fixtures"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-provider", "codex"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-runtime-signal", "lease-health"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--unknown"], 0, options)
  assert.equal(index, null)

  assert.deepEqual(options, {
    presets: ["workspace-live-sync"],
    requiredPlatformCoverageAreas: ["runtime-fixtures"],
    requiredRuntimeSignals: ["lease-health"],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredDeploymentPresets: [],
    requiredProviders: ["codex"],
    requiredScenarios: [],
  })
})

test("parses aggregate preset requirements with custom key", () => {
  const options = validationGateRequirementOptionDefaults({ presetKey: "requiredPresets" })
  const index = parseValidationGateRequirementArg(
    ["--require-preset=remote-home-extension"],
    0,
    options,
    {
      presetFlag: "--require-preset",
      presetKey: "requiredPresets",
    },
  )

  assert.equal(index, 0)
  assert.deepEqual(options.requiredPresets, ["remote-home-extension"])
  assert.deepEqual(options.requiredProviders, [])
})

test("rejects missing validation gate requirement values", () => {
  const options = validationGateRequirementOptionDefaults()
  assert.throws(
    () => parseValidationGateRequirementArg(["--require-provider"], 0, options),
    /--require-provider requires a value/,
  )
  assert.throws(
    () => parseValidationGateRequirementArg(["--preset", "--json"], 0, options),
    /--preset requires a value/,
  )
})
