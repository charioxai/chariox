# Arroba Validation Platform

Arroba runtime features must be validated through reusable drill primitives, not one-off scripts with private conventions. The goal is to make local, remote, hosted, collab, native TUI, slice, and provider-specific behavior comparable across features.

## Shared Primitives

- Artifact lifecycle: use `apps/cli/scripts/lib/drill-artifacts.mjs` to prepare drill roots and preserve failed runs with `arroba-drill-failure.json`.
- Failure summaries: use `apps/cli/scripts/lib/drill-failure-manifest.mjs` or `apps/cli/scripts/drill-failure-summary.mjs` to validate and summarize preserved failed runs without printing credentials or large payloads.
- Failure taxonomy: use `apps/cli/scripts/lib/drill-failure-taxonomy.mjs` for classification owners and next actions shared by failure manifests and matrix reports.
- Aggregate actions: use `apps/cli/scripts/lib/drill-aggregate-actions.mjs` to group and validate owner/classification/next-action counts consistently across reports.
- Secret hygiene: use `apps/cli/scripts/lib/drill-secrets.mjs` for shared drill metadata redaction and token-shaped value detection.
- Time fields: use `apps/cli/scripts/lib/drill-time.mjs` to validate strict ISO timestamps and report start/end ordering.
- Matrix execution: use `apps/cli/scripts/lib/drill-matrix-runner.mjs` for scenario selection, include-gate enforcement, command rendering, expected-failure handling, failure classification, skipped-scenario accounting, summaries, and reports.
- Environment presets: use `apps/cli/scripts/lib/drill-environment-presets.mjs` for shared Hetzner pass-through parsing and non-secret preset metadata.
- Provider profiles: use `apps/cli/scripts/lib/drill-provider-profiles.mjs` for shared provider list parsing, provider-model overrides, model resolution, and non-secret profile metadata.
- Runtime waits: use `apps/cli/scripts/lib/drill-runtime-helpers.mjs` wait helpers so timeouts include the last observed runtime state.
- Child failure classification: use `apps/cli/scripts/lib/drill-child-process.mjs` so provider auth/account failures are separated from runtime regressions.
- Feature fixtures: put reusable setup and assertions under `apps/cli/scripts/lib/`; entry scripts should stay thin.

Run the shared non-live validation platform checks with:

```bash
node apps/cli/scripts/drill-validation-suite.mjs
node apps/cli/scripts/drill-validation-suite.mjs --check
node apps/cli/scripts/drill-validation-suite.mjs --json
node apps/cli/scripts/drill-validation-suite.mjs --json --output .artifacts/drill-validation-suite.json
node apps/cli/scripts/drill-validation-suite.mjs --list
node apps/cli/scripts/drill-validation-suite.mjs --command
```

The `--json` output uses schema `arroba.drill.validation_suite.v1` and lists the exact test paths and command covered by the suite. Use `--output PATH` with `--json` when CI or staging jobs should collect the coverage manifest as an artifact.

Export the shared failure taxonomy with:

```bash
node apps/cli/scripts/drill-failure-taxonomy.mjs
node apps/cli/scripts/drill-failure-taxonomy.mjs --target drill --output .artifacts/drill-failure-taxonomy.json
```

## Matrix Reports

Matrix scripts write JSON with schema `arroba.drill.matrix.v1`. Use `defaultDrillMatrixReportPath(...)` so reports are written under `.artifacts/drill-matrices/<matrix>/<timestamp>.json` when the caller does not pass `--report PATH`. `--report PATH` remains the override for CI jobs or custom collection directories.

Summarize one or more reports with:

```bash
node apps/cli/scripts/drill-matrix-report-summary.mjs path/to/matrix-report.json
node apps/cli/scripts/drill-matrix-report-summary.mjs --find .artifacts/drill-matrices
node apps/cli/scripts/drill-matrix-report-summary.mjs --find .artifacts --max-depth 4
node apps/cli/scripts/drill-matrix-report-summary.mjs --require-complete --find .artifacts/drill-matrices
node apps/cli/scripts/drill-matrix-report-summary.mjs --json --output path/to/aggregate.json path/to/*.json
```

The summary command exits non-zero when any input report has `status=failed`. `--find ROOT` discovers valid matrix reports below an artifact root and ignores unrelated JSON files.
Matrix report discovery is bounded by default and prunes heavy irrelevant directories such as `.git`, `node_modules`, `.pnpm-store`, `debug`, and `release`, so broad artifact roots are safe to scan in CI. Use `--max-depth N` when a CI artifact layout needs a tighter or wider traversal bound.
For dry-run reports, it prints selected scenario exit criteria so reviewers can confirm matrix scope before running live drills.
Use `--require-complete` for release/staging gates that must reject skipped or dry-run scenarios even when no scenario failed.
When more than one report is selected, the human summary prints an aggregate section with total coverage, failure owners, next actions, and incomplete scenarios.
Failed-scenario summaries include an owner and next action so humans and CI can route the fix without opening raw logs first.
The shared taxonomy can also be exported as schema `arroba.drill.failure_taxonomy.v1` by calling `drillFailureTaxonomyManifest(...)` from `apps/cli/scripts/lib/drill-failure-taxonomy.mjs`; use this instead of duplicating classification tables in feature drills or UI diagnostics.
The `--json`/`--output` aggregate schema is `arroba.drill.matrix.aggregate.v1`; its `reports`, `failedScenarios`, and `incompleteScenarios` entries include the originating report `source` path when available. Its `reports` entries must have internally consistent status/count/duration fields, and aggregate totals must equal the sum of those entries. Its `owners` map counts failed-scenario owners, `nextActions` groups repeated owner/classification/action pairs, its `failedScenarios` entries include `classification`, `owner`, `reason`, `artifactHints`, and `nextAction`, and its `incompleteScenarios` entries list skipped and dry-run coverage gaps.

Required top-level fields:

- `schema`: always `arroba.drill.matrix.v1`.
- `matrix`: stable matrix name.
- `status`: `passed`, `failed`, or `dry-run`; it must match scenario statuses.
- `dryRun`: boolean; true only when every selected scenario is `dry-run`.
- `startedAt`, `completedAt`, `durationMs`.
- `metadata`: feature-specific non-secret context such as enabled scenario groups or provider model ids.
- `scenarios`: selected scenario results; reports with no selected scenarios are invalid.

Top-level `durationMs` must equal `completedAt - startedAt` exactly. This keeps generated reports comparable across local, remote, hosted, and collab matrices and catches hand-authored report drift.

Required scenario fields:

- `id`, `description`, `requires`.
- `exitCriteria`: proof points the scenario is expected to establish, or an empty array.
- `status`: `passed`, `failed`, `skipped`, or `dry-run`.
- `expectedFailure`, `classification`, `durationMs`, `reason`.
- `command`, `args`.
- `artifactHints`: optional paths to preserved artifact roots or failure manifests discovered from child drill output.

Matrix runners preflight selected scenario definitions and all selected scenario commands before spawning child drills. Scenario ids must be unique and non-empty, selected scenarios must include descriptions, `requires` and `exitCriteria` must be string arrays or valid strings where supported, and `commandForScenario` must return a non-empty command with string args for every selected scenario.

Scenario outcome fields must be internally consistent. Failed scenarios require a non-empty `reason` and `classification` so CI can route the failure. Skipped scenarios require a non-empty `reason` and zero `durationMs`. Dry-run scenarios require zero `durationMs` and must not include a `reason` or `classification`. Passed scenarios must not include a `reason`.

Reports must not include credentials, relay tokens, provider tokens, prompt bodies, file contents, or unredacted connector payloads. If a drill needs detailed failure output, preserve the artifact directory and record a pointer in the failure manifest instead of embedding sensitive logs in the matrix report.
The matrix report validator rejects secret-looking metadata keys, token-shaped metadata values, and token-shaped artifact hints. Matrix runners validate before writing reports and skip token-shaped artifact hints extracted from child output. Child failure summaries redact token-shaped values before preserving output tails.

## Failure Manifests

Failed drills that preserve artifacts write `arroba-drill-failure.json` with schema `arroba.drill.failure.v1`.

Summarize one or more preserved roots or manifest files with:

```bash
node apps/cli/scripts/drill-failure-summary.mjs path/to/preserved-root
node apps/cli/scripts/drill-failure-summary.mjs path/to/arroba-drill-failure.json
node apps/cli/scripts/drill-failure-summary.mjs --find apps/cli/target .artifacts
node apps/cli/scripts/drill-failure-summary.mjs --find --max-depth 4 .artifacts
node apps/cli/scripts/drill-failure-summary.mjs --json --output path/to/failure-aggregate.json --find apps/cli/target .artifacts
```

Required top-level fields:

- `schema`: always `arroba.drill.failure.v1`.
- `rootDir`: preserved artifact root.
- `failedAt`: ISO timestamp for the failure.
- `metadata`: non-secret drill context such as drill name, provider profile, relay mode, or scenario id.
- `error`: error name, message, and optional stack.

Failure manifests redact sensitive metadata keys, token-shaped metadata values, and token-shaped error text before writing. The validator rejects token-shaped metadata values, including nested values, so externally supplied manifests cannot pass with credentials embedded. Failure summaries also redact sensitive metadata keys, token-shaped error messages, and omit nested values. Keep raw logs, screenshots, and packet captures in the preserved artifact root, not in the manifest.
When more than one failure manifest is selected, the summary command prints an aggregate owner/classification section so preserved failure batches can be routed quickly.
Failure manifest discovery is bounded by default, prunes heavy irrelevant directories, and accepts `--max-depth N` for CI layouts that need a tighter or wider traversal bound.
The `--json`/`--output` aggregate schema is `arroba.drill.failure.aggregate.v1`.
Its `nextActions` entries group repeated owner/classification/action pairs so CI and humans can route a failure batch without scanning every manifest.
Each failure entry includes the drill name, preserved artifact root, optional manifest `source` path, owner, classification, and next action.

## Scenario Selection

Default runs should stay fast and local. Expensive or environment-dependent groups must be behind explicit include flags such as `--include-remote`, `--include-hetzner`, or `--include-opencode`. Selecting a gated scenario by `--only` must still require its include flag, so accidental partial validation fails loudly.

When a matrix stops after the first failure, remaining selected scenarios must be reported as `skipped`. With `--continue-on-failure`, every selected scenario should run unless its own setup fails.

## Exit Criteria

Every new runtime feature should define:

- Unit tests for policy, protocol shape, and edge cases.
- At least one local drill that proves the kernel-owned runtime path.
- Matrix scenarios for remote, hosted, collab, native TUI, slice, and provider variants when the feature crosses those surfaces.
- Failure artifacts sufficient to identify the failing owner: provider account/auth, relay, kernel authority, worker execution, UI/client projection, or test harness.
- Diagnostic wait failures that include `last_observation` should classify as runtime state timeouts unless the failure is clearly relay, cloud, provider auth/account, Docker, or test harness owned.
- Distributed-runtime failures should classify to the narrowest actionable owner: `kernel-authority` for rejected session/agent/lease/provider-run bindings and duplicate provider-run binding health signals, `worker-execution` for leased-agent or remote worker launch/execution failures, `ui-client-projection` for web/TUI terminal or transcript projection failures, `remote-extension-sync` for home/worker manifest drift, `workspace-live-sync-conflict` for sync conflicts or external changes, and `slice-auth` for missing or wrong provider accounts inside slices.
- A reportable command that can be run by humans and CI without editing the script.
- Default matrix report output under `.artifacts/drill-matrices` so dry-runs and live runs leave auditable evidence without requiring a custom `--report` flag.

Feature work is not complete until the relevant matrix reports prove the intended scope or clearly classify external blockers that prevented validation.
