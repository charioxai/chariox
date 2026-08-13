import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_FOCUSED_RUNTIME_GATE_SCHEMA,
  FOCUSED_RUNTIME_GATE_PRESETS,
  validateDrillFocusedRuntimeGateReport,
} from "./drill-focused-runtime-gate-report.mjs"

test("accepts a focused runtime gate report with two passed embedded gates", () => {
  assert.doesNotThrow(() => validateDrillFocusedRuntimeGateReport(focusedReport()))
})

test("rejects unsupported schema and mismatched status", () => {
  assert.throws(
    () => validateDrillFocusedRuntimeGateReport({ ...focusedReport(), schema: "wrong" }),
    /unsupported schema/,
  )
  assert.throws(
    () => validateDrillFocusedRuntimeGateReport({
      ...focusedReport({ status: "passed" }),
      reports: [
        focusedEntry("runtime-authority", validationGateReport({ status: "failed" })),
        focusedEntry("distributed-state-health", validationGateReport({ preset: "distributed-state-health" })),
      ],
    }),
    /status does not match embedded report statuses/,
  )
})

test("rejects reports that do not cover focused presets exactly", () => {
  assert.throws(
    () => validateDrillFocusedRuntimeGateReport({
      ...focusedReport(),
      presets: ["runtime-authority"],
    }),
    /presets must equal runtime-authority,distributed-state-health/,
  )
  assert.throws(
    () => validateDrillFocusedRuntimeGateReport({
      ...focusedReport(),
      reports: [focusedEntry("runtime-authority", validationGateReport({ preset: "runtime-authority" }))],
    }),
    /reports does not cover focused runtime presets/,
  )
  assert.throws(
    () => validateDrillFocusedRuntimeGateReport({
      ...focusedReport(),
      reports: [
        focusedEntry("runtime-authority", validationGateReport({ preset: "distributed-state-health" })),
        focusedEntry("distributed-state-health", validationGateReport({ preset: "distributed-state-health" })),
      ],
    }),
    /reports\[0\]\.report presets do not match entry preset/,
  )
})

test("rejects stale focused next-action projections", () => {
  const failedReport = validationGateReport({
    preset: "runtime-authority",
    status: "failed",
    nextActions: [nextAction()],
  })
  assert.doesNotThrow(() => validateDrillFocusedRuntimeGateReport(focusedReport({
    status: "failed",
    reports: [
      focusedEntry("runtime-authority", failedReport),
      focusedEntry("distributed-state-health", validationGateReport({ preset: "distributed-state-health" })),
    ],
    nextActions: [{ ...nextAction(), preset: "runtime-authority" }],
  })))
  assert.throws(
    () => validateDrillFocusedRuntimeGateReport(focusedReport({
      status: "failed",
      reports: [
        focusedEntry("runtime-authority", failedReport),
        focusedEntry("distributed-state-health", validationGateReport({ preset: "distributed-state-health" })),
      ],
      nextActions: [{ ...nextAction(), preset: "distributed-state-health" }],
    })),
    /nextActions do not match embedded report next actions/,
  )
})

function focusedReport({
  status = "passed",
  reports = [
    focusedEntry("runtime-authority", validationGateReport({ preset: "runtime-authority" })),
    focusedEntry("distributed-state-health", validationGateReport({ preset: "distributed-state-health" })),
  ],
  nextActions = reports.flatMap((entry) => entry.report.nextActions.map((action) => ({ ...action, preset: entry.preset }))),
} = {}) {
  return {
    schema: DRILL_FOCUSED_RUNTIME_GATE_SCHEMA,
    status,
    presets: [...FOCUSED_RUNTIME_GATE_PRESETS],
    reports,
    nextActions,
  }
}

function focusedEntry(preset, report) {
  return { preset, report }
}

function validationGateReport({
  preset = "runtime-authority",
  status = "passed",
  nextActions = [],
} = {}) {
  const checkStatus = status === "passed" ? "passed" : "failed"
  return {
    schema: "chariox.drill.validation_gate.v1",
    status,
    presets: [preset],
    checks: {
      configuration: { status: "passed" },
      platformBundle: {
        status: "passed",
        dir: "/tmp/platform",
        artifacts: [],
        validationSuite: {
          testCount: 1,
          coverageAreas: [{ id: "matrix-validation", testCount: 1 }],
          validationPresets: [],
        },
      },
      artifacts: {
        status: "skipped",
        roots: [],
        inputs: [],
        indexPaths: [],
      },
      matrices: checkStatus === "passed"
        ? {
          status: "passed",
          roots: [],
          inputs: [],
          reportPaths: [],
          requireComplete: false,
          reports: [],
        }
        : {
          status: "failed",
          roots: [],
          inputs: [],
          reportPaths: [],
          requireComplete: false,
          reports: [],
          error: "missing focused evidence",
        },
      failures: {
        status: "skipped",
        inputs: [],
        roots: [],
        manifestPaths: [],
        manifests: [],
      },
    },
    nextActions,
  }
}

function nextAction() {
  return {
    owner: "kernel-authority",
    classification: "kernel-authority",
    nextAction: "collect missing runtime authority matrix evidence",
    count: 1,
  }
}
