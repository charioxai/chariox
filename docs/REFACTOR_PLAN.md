# Refactor Plan

## Status

Living plan for the current code-simplification pass across the CLI, daemon, and supporting packages.

## Goals

- remove dead and misleading code paths
- reduce duplicate layout/runtime logic
- shrink the largest mixed-responsibility modules
- improve maintainability without changing runtime behavior

## Completed

### 1. Legacy Rust CLI removal

Done:

- deleted the old `arroba-cli-rust` binary implementation
- removed the extra binary target from `apps/daemon/Cargo.toml`
- removed the daemon integration test that still depended on the deleted Rust-only CLI
- updated runtime and contributor docs so the supported local CLI paths are now:
  - `arroba-cli`
  - direct `apps/cli` development

Why this mattered:

- the old Rust-only CLI was no longer the real product path
- keeping it around added maintenance overhead and created avoidable confusion about which client surface was authoritative

### 2. Split-pane layout cleanup

Done:

- deleted `apps/cli/src/agent-pane-layout.ts` and its test because that path was dead
- deleted `packages/domain/src/layout.ts` because it was unused in the repository
- extracted the live split-pane selection and geometry logic into `apps/cli/src/response-panes.ts`
- added focused tests in `apps/cli/src/response-panes.test.ts`
- rewired `apps/cli/src/index.tsx` to use the extracted pane helper instead of duplicating pane-selection and sizing logic inline

Why this mattered:

- the repo had multiple overlapping pane-layout implementations
- only one of them matched the live CLI behavior
- centralizing the active path reduces drift and makes the next `index.tsx` split safer

### 3. Split-pane footer extraction

Done:

- extracted split-pane footer badge-state and label-formatting logic into `apps/cli/src/split-pane-footer.ts`
- added focused tests in `apps/cli/src/split-pane-footer.test.ts`
- simplified the footer-rendering path in `apps/cli/src/index.tsx` so pane footer state is derived outside the app root

Why this mattered:

- split-pane footer state had been embedded inside the main CLI render file
- moving the badge and footer formatting logic out reduces `index.tsx` responsibility and gives the first Phase 1 seam dedicated test coverage

### 4. Agent-pane refresh extraction

Done:

- extracted agent-pane transcript retention and refresh orchestration into `apps/cli/src/agent-pane-state.ts`
- added focused tests in `apps/cli/src/agent-pane-state.test.ts`
- simplified `apps/cli/src/index.tsx` so the async history refresh loop and transcript-retention trimming no longer live inline in the app root

Why this mattered:

- pane refresh and retention logic had been mixed directly into the main CLI component
- moving that logic out makes transcript/pane state behavior easier to test without involving render-tree wiring

## Current State

The refactor has started, but the largest simplification targets still remain:

- `apps/cli/src/index.tsx` is still the main monolith
- `apps/daemon/src/app.rs` still mixes orchestration with history/prompt helper logic
- `apps/daemon/src/local/api.rs` still mixes transport handling with provider-specific work
- `apps/daemon/src/provider/service.rs` still mixes provider lifecycle control with OpenCode-specific transcript rendering

## Next Phases

### Phase 1. Continue splitting the TypeScript CLI

Primary target:

- break `apps/cli/src/index.tsx` into smaller modules by responsibility

Next extraction seams:

- session bootstrap and local IPC request-builder logic
- response-layout render wiring that still mutates the OpenTUI renderables directly from the app root

Expected outcome:

- smaller, testable units around rendering, session state, and transport
- less duplicated agent/pane bookkeeping
- lower change risk for future multi-agent UI work

### Phase 2. Simplify daemon orchestration boundaries

Targets:

- move history-slicing and transcript helper code out of `apps/daemon/src/app.rs`
- reduce `apps/daemon/src/local/api.rs` to transport/request handling only
- move provider-catalog/provider-specific logic behind a clearer provider service boundary

Expected outcome:

- `DaemonApp` becomes the coordinator instead of the dumping ground
- local IPC handling becomes easier to reason about and test

### Phase 3. Split provider lifecycle from provider-specific rendering

Targets:

- separate generic provider-run lifecycle management from OpenCode-specific event parsing and transcript formatting
- keep provider state transitions, process control, and transcript rendering in different modules

Expected outcome:

- cleaner OpenCode-first implementation
- easier future extension to additional providers without polluting the generic runtime path

### Phase 4. Startup/build-path cleanup

Targets:

- stop doing unnecessary CLI rebuild work on every launcher execution
- make the launcher check whether the TypeScript CLI output is missing or stale before building

Expected outcome:

- faster local startup
- simpler launcher behavior

## Verification Strategy

For each phase:

- keep behavior-preserving refactors small and incremental
- add or move tests alongside each extraction seam
- prefer focused package-level verification first
- rerun broader daemon/CLI verification after each structural milestone

Current verification already completed during this pass:

- `cargo test --manifest-path apps/daemon/Cargo.toml --no-run`
- `pnpm --filter @arroba/domain test`
- `pnpm --filter @arroba/cli test`
- `git diff --check`

## Immediate Next Step

The next concrete change should stay in Phase 1 by extracting session bootstrap and local IPC request-builder logic out of `apps/cli/src/index.tsx`.
