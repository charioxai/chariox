export const SHARED_DRILL_TEST_PATHS = Object.freeze([
  "apps/cli/scripts/drill-failure-summary.test.mjs",
  "apps/cli/scripts/drill-matrix-report-summary.test.mjs",
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
  "apps/cli/scripts/lib/drill-provider-profiles.test.mjs",
  "apps/cli/scripts/lib/drill-runtime-helpers.test.mjs",
  "apps/cli/scripts/lib/drill-secrets.test.mjs",
  "apps/cli/scripts/lib/drill-time.test.mjs",
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
