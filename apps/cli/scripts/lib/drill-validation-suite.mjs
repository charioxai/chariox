import { access } from "node:fs/promises"
import path from "node:path"

export const SHARED_DRILL_TEST_PATHS = Object.freeze([
  "apps/cli/scripts/drill-artifact-index-summary.test.mjs",
  "apps/cli/scripts/drill-failure-summary.test.mjs",
  "apps/cli/scripts/drill-failure-taxonomy.test.mjs",
  "apps/cli/scripts/drill-matrix-report-summary.test.mjs",
  "apps/cli/scripts/drill-platform-bundle.test.mjs",
  "apps/cli/scripts/drill-validation-gate-summary.test.mjs",
  "apps/cli/scripts/drill-validation-gate.test.mjs",
  "apps/cli/scripts/drill-validation-suite.test.mjs",
  "apps/cli/scripts/lib/drill-aggregate-actions.test.mjs",
  "apps/cli/scripts/lib/drill-artifacts.test.mjs",
  "apps/cli/scripts/lib/drill-child-process.test.mjs",
  "apps/cli/scripts/lib/drill-cli-args.test.mjs",
  "apps/cli/scripts/lib/drill-environment-presets.test.mjs",
  "apps/cli/scripts/lib/drill-failure-manifest.test.mjs",
  "apps/cli/scripts/lib/drill-failure-taxonomy.test.mjs",
  "apps/cli/scripts/lib/drill-history-outline.test.mjs",
  "apps/cli/scripts/lib/drill-matrix-report.test.mjs",
  "apps/cli/scripts/lib/drill-matrix-runner.test.mjs",
  "apps/cli/scripts/lib/drill-platform-bundle.test.mjs",
  "apps/cli/scripts/lib/drill-provider-profiles.test.mjs",
  "apps/cli/scripts/lib/drill-runtime-helpers.test.mjs",
  "apps/cli/scripts/lib/drill-secrets.test.mjs",
  "apps/cli/scripts/lib/drill-time.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-aggregate.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-args.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate.test.mjs",
  "apps/cli/scripts/lib/drill-validation-suite.test.mjs",
])

export const DRILL_VALIDATION_COVERAGE_AREAS = Object.freeze([
  {
    id: "artifact-contracts",
    description: "Artifact indexes, failure manifests, aggregate actions, summaries, and platform bundles.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-artifact-index-summary.test.mjs",
      "apps/cli/scripts/drill-failure-summary.test.mjs",
      "apps/cli/scripts/drill-platform-bundle.test.mjs",
      "apps/cli/scripts/lib/drill-aggregate-actions.test.mjs",
      "apps/cli/scripts/lib/drill-artifacts.test.mjs",
      "apps/cli/scripts/lib/drill-failure-manifest.test.mjs",
      "apps/cli/scripts/lib/drill-platform-bundle.test.mjs",
    ]),
  },
  {
    id: "failure-diagnostics",
    description: "Failure classification, owners, next actions, and user-facing taxonomy summaries.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-failure-taxonomy.test.mjs",
      "apps/cli/scripts/lib/drill-child-process.test.mjs",
      "apps/cli/scripts/lib/drill-failure-taxonomy.test.mjs",
    ]),
  },
  {
    id: "matrix-validation",
    description: "Matrix runners, report validation, report summaries, and validation gates.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-matrix-report-summary.test.mjs",
      "apps/cli/scripts/drill-validation-gate-summary.test.mjs",
      "apps/cli/scripts/drill-validation-gate.test.mjs",
      "apps/cli/scripts/lib/drill-matrix-report.test.mjs",
      "apps/cli/scripts/lib/drill-matrix-runner.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-aggregate.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-args.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate.test.mjs",
    ]),
  },
  {
    id: "runtime-fixtures",
    description: "Reusable runtime, provider, environment, time, and history-outline helpers.",
    testPaths: Object.freeze([
      "apps/cli/scripts/lib/drill-cli-args.test.mjs",
      "apps/cli/scripts/lib/drill-environment-presets.test.mjs",
      "apps/cli/scripts/lib/drill-history-outline.test.mjs",
      "apps/cli/scripts/lib/drill-provider-profiles.test.mjs",
      "apps/cli/scripts/lib/drill-runtime-helpers.test.mjs",
      "apps/cli/scripts/lib/drill-secrets.test.mjs",
      "apps/cli/scripts/lib/drill-time.test.mjs",
    ]),
  },
  {
    id: "suite-contract",
    description: "The validation suite manifest and command contract itself.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-validation-suite.test.mjs",
      "apps/cli/scripts/lib/drill-validation-suite.test.mjs",
    ]),
  },
])

export function drillValidationSuiteArgs({ testPaths = SHARED_DRILL_TEST_PATHS } = {}) {
  return ["--test", ...testPaths]
}

export function drillValidationSuiteCommand({ nodeCommand = "node", testPaths = SHARED_DRILL_TEST_PATHS } = {}) {
  return [nodeCommand, ...drillValidationSuiteArgs({ testPaths })]
    .map((part) => (/[ "'\\]/.test(part) ? JSON.stringify(part) : part))
    .join(" ")
}

export function drillValidationSuiteManifest({
  nodeCommand = "node",
  schema = "arroba.drill.validation_suite.v1",
  testPaths = SHARED_DRILL_TEST_PATHS,
  coverageAreas = DRILL_VALIDATION_COVERAGE_AREAS,
} = {}) {
  const coverage = validationSuiteCoverage({ coverageAreas, testPaths })
  return {
    schema,
    testCount: testPaths.length,
    command: drillValidationSuiteCommand({ nodeCommand, testPaths }),
    coverage,
    testPaths: [...testPaths],
  }
}

export function validationSuiteCoverage({
  coverageAreas = DRILL_VALIDATION_COVERAGE_AREAS,
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  validateValidationSuiteCoverage({ coverageAreas, testPaths })
  return coverageAreas.map((area) => ({
    id: area.id,
    description: area.description,
    testCount: area.testPaths.length,
    testPaths: [...area.testPaths],
  }))
}

export function validateValidationSuiteCoverage({
  coverageAreas = DRILL_VALIDATION_COVERAGE_AREAS,
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  const suitePaths = new Set(testPaths)
  const coveredPaths = new Set()
  const areaIds = new Set()
  for (const area of coverageAreas) {
    if (!area || typeof area !== "object") throw new Error("validation suite coverage has invalid area")
    if (typeof area.id !== "string" || area.id.length === 0) throw new Error("validation suite coverage area has invalid id")
    if (areaIds.has(area.id)) throw new Error(`validation suite coverage has duplicate area id ${area.id}`)
    areaIds.add(area.id)
    if (typeof area.description !== "string" || area.description.length === 0) {
      throw new Error(`validation suite coverage area ${area.id} has invalid description`)
    }
    if (!Array.isArray(area.testPaths) || area.testPaths.length === 0) {
      throw new Error(`validation suite coverage area ${area.id} has invalid testPaths`)
    }
    for (const testPath of area.testPaths) {
      if (!suitePaths.has(testPath)) {
        throw new Error(`validation suite coverage area ${area.id} references non-suite test ${testPath}`)
      }
      if (coveredPaths.has(testPath)) {
        throw new Error(`validation suite coverage references duplicate test ${testPath}`)
      }
      coveredPaths.add(testPath)
    }
  }
  const missingCoverage = [...suitePaths].filter((testPath) => !coveredPaths.has(testPath))
  if (missingCoverage.length > 0) {
    throw new Error(`validation suite tests missing coverage areas: ${missingCoverage.join(", ")}`)
  }
}

export async function findMissingDrillValidationSuitePaths({
  rootDir = process.cwd(),
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  const missing = []
  for (const testPath of testPaths) {
    const absolutePath = path.resolve(rootDir, testPath)
    try {
      await access(absolutePath)
    } catch {
      missing.push(testPath)
    }
  }
  return missing
}
