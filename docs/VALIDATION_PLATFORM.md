# Arroba Validation Platform

Arroba runtime features must be validated through reusable drill primitives, not one-off scripts with private conventions. The goal is to make local, remote, hosted, collab, native TUI, slice, and provider-specific behavior comparable across features.

## Shared Primitives

- Artifact lifecycle: use `apps/cli/scripts/lib/drill-artifacts.mjs` to prepare drill roots and preserve failed runs with `arroba-drill-failure.json`.
- Matrix execution: use `apps/cli/scripts/lib/drill-matrix-runner.mjs` for scenario selection, include-gate enforcement, command rendering, expected-failure handling, failure classification, skipped-scenario accounting, summaries, and optional reports.
- Child failure classification: use `apps/cli/scripts/lib/drill-child-process.mjs` so provider auth/account failures are separated from runtime regressions.
- Feature fixtures: put reusable setup and assertions under `apps/cli/scripts/lib/`; entry scripts should stay thin.

## Matrix Reports

Matrix scripts that support `--report PATH` write JSON with schema `arroba.drill.matrix.v1`.

Summarize one or more reports with:

```bash
node apps/cli/scripts/drill-matrix-report-summary.mjs path/to/matrix-report.json
```

The summary command exits non-zero when any input report has `status=failed`.

Required top-level fields:

- `schema`: always `arroba.drill.matrix.v1`.
- `matrix`: stable matrix name.
- `status`: `passed`, `failed`, or `dry-run`.
- `dryRun`: boolean.
- `startedAt`, `completedAt`, `durationMs`.
- `metadata`: feature-specific non-secret context such as enabled scenario groups or provider model ids.
- `scenarios`: selected scenario results.

Required scenario fields:

- `id`, `description`, `requires`.
- `exitCriteria`: proof points the scenario is expected to establish, or an empty array.
- `status`: `passed`, `failed`, `skipped`, or `dry-run`.
- `expectedFailure`, `classification`, `durationMs`, `reason`.
- `command`, `args`.

Reports must not include credentials, relay tokens, provider tokens, prompt bodies, file contents, or unredacted connector payloads. If a drill needs detailed failure output, preserve the artifact directory and record a pointer in the failure manifest instead of embedding sensitive logs in the matrix report.

## Scenario Selection

Default runs should stay fast and local. Expensive or environment-dependent groups must be behind explicit include flags such as `--include-remote`, `--include-hetzner`, or `--include-opencode`. Selecting a gated scenario by `--only` must still require its include flag, so accidental partial validation fails loudly.

When a matrix stops after the first failure, remaining selected scenarios must be reported as `skipped`. With `--continue-on-failure`, every selected scenario should run unless its own setup fails.

## Exit Criteria

Every new runtime feature should define:

- Unit tests for policy, protocol shape, and edge cases.
- At least one local drill that proves the kernel-owned runtime path.
- Matrix scenarios for remote, hosted, collab, native TUI, slice, and provider variants when the feature crosses those surfaces.
- Failure artifacts sufficient to identify the failing owner: provider account/auth, relay, kernel authority, worker execution, UI/client projection, or test harness.
- A reportable command that can be run by humans and CI without editing the script.

Feature work is not complete until the relevant matrix reports prove the intended scope or clearly classify external blockers that prevented validation.
