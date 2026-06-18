import { validateDrillAggregateNextAction } from "./drill-aggregate-actions.mjs"
import {
  validateDrillValidationGatePreset,
} from "./drill-validation-gate-presets.mjs"
import { validateDrillValidationGateReport } from "./drill-validation-gate-report.mjs"
import { validateDrillValidationResultStatus } from "./drill-validation-statuses.mjs"

export const DRILL_FOCUSED_RUNTIME_GATE_SCHEMA = "arroba.drill.focused_runtime_gate.v1"
export const FOCUSED_RUNTIME_GATE_PRESETS = Object.freeze(["runtime-authority", "distributed-state-health"])

export function validateDrillFocusedRuntimeGateReport(report, source = "focused runtime gate report") {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== DRILL_FOCUSED_RUNTIME_GATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  validateDrillValidationResultStatus(report.status, source)
  validateFocusedPresetArray(report.presets, `${source}.presets`)
  if (!Array.isArray(report.reports)) {
    throw new Error(`${source}.reports is not an array`)
  }
  if (report.reports.length !== FOCUSED_RUNTIME_GATE_PRESETS.length) {
    throw new Error(`${source}.reports does not cover focused runtime presets`)
  }
  const reportPresets = []
  for (const [index, entry] of report.reports.entries()) {
    validateFocusedReportEntry(entry, `${source}.reports[${index}]`)
    reportPresets.push(entry.preset)
  }
  if (JSON.stringify(reportPresets) !== JSON.stringify(report.presets)) {
    throw new Error(`${source}.reports presets do not match top-level presets`)
  }
  if (!Array.isArray(report.nextActions)) {
    throw new Error(`${source}.nextActions is not an array`)
  }
  for (const [index, action] of report.nextActions.entries()) {
    validateFocusedNextAction(action, `${source}.nextActions[${index}]`)
  }
  const expectedStatus = report.reports.some((entry) => entry.report.status === "failed") ? "failed" : "passed"
  if (report.status !== expectedStatus) {
    throw new Error(`${source} status does not match embedded report statuses`)
  }
  const expectedNextActions = report.reports.flatMap((entry) =>
    entry.report.nextActions.map((action) => ({ ...action, preset: entry.preset })),
  )
  if (JSON.stringify(report.nextActions) !== JSON.stringify(expectedNextActions)) {
    throw new Error(`${source}.nextActions do not match embedded report next actions`)
  }
}

function validateFocusedReportEntry(entry, source) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    throw new Error(`${source} is not an object`)
  }
  validateFocusedRuntimePreset(entry.preset, `${source}.preset`)
  validateDrillValidationGateReport(entry.report, `${source}.report`)
  if (JSON.stringify(entry.report.presets ?? []) !== JSON.stringify([entry.preset])) {
    throw new Error(`${source}.report presets do not match entry preset`)
  }
}

function validateFocusedNextAction(action, source) {
  validateDrillAggregateNextAction(action, source)
  validateFocusedRuntimePreset(action.preset, `${source}.preset`)
}

function validateFocusedPresetArray(value, source) {
  if (!Array.isArray(value)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, preset] of value.entries()) {
    validateFocusedRuntimePreset(preset, `${source}[${index}]`)
  }
  if (JSON.stringify(value) !== JSON.stringify(FOCUSED_RUNTIME_GATE_PRESETS)) {
    throw new Error(`${source} must equal ${FOCUSED_RUNTIME_GATE_PRESETS.join(",")}`)
  }
}

function validateFocusedRuntimePreset(preset, source) {
  validateDrillValidationGatePreset(preset, source)
  if (!FOCUSED_RUNTIME_GATE_PRESETS.includes(preset)) {
    throw new Error(`${source} is not a focused runtime preset`)
  }
}
