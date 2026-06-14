import { access } from "node:fs/promises"
import path from "node:path"

export const SHARED_DRILL_TEST_PATHS = Object.freeze([
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
  "apps/cli/scripts/lib/drill-validation-gate.test.mjs",
  "apps/cli/scripts/lib/drill-validation-suite.test.mjs",
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
} = {}) {
  return {
    schema,
    testCount: testPaths.length,
    command: drillValidationSuiteCommand({ nodeCommand, testPaths }),
    testPaths: [...testPaths],
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
