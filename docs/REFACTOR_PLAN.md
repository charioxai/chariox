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

### 5. Bootstrap and IPC request extraction

Done:

- extracted shared CLI runtime/session types into `apps/cli/src/cli-types.ts`
- extracted local IPC request builders into `apps/cli/src/ipc-requests.ts`
- extracted bootstrap orchestration into `apps/cli/src/session-bootstrap.ts`
- added focused tests in `apps/cli/src/ipc-requests.test.ts` and `apps/cli/src/session-bootstrap.test.ts`
- simplified `apps/cli/src/index.tsx` so bootstrap flow and IPC request payload construction no longer live inline in the main CLI file

Why this mattered:

- bootstrap orchestration and request payload construction had been mixed into the same large file as rendering and session UI state
- moving them out reduces `index.tsx` responsibility and makes protocol/bootstrap behavior testable without rendering the CLI

### 6. Response-layout render wiring extraction

Done:

- extracted response-layout render-tree wiring into `apps/cli/src/response-layout-render.ts`
- added focused tests in `apps/cli/src/response-layout-render.test.ts`
- simplified `apps/cli/src/index.tsx` so pane layout mutation, auxiliary pane sync, and render-tree repaint traversal no longer live inline in the app root

Why this mattered:

- response layout application had still been tightly coupled to render-tree mutation inside the main CLI file
- moving that wiring out completes the planned Phase 1 extraction seams and reduces the remaining `index.tsx` surface to higher-level UI orchestration

### 7. Daemon history helper extraction

Done:

- extracted session history paging types and slicing logic into `apps/daemon/src/session_history_page.rs`
- extracted prompt transcript rendering into `apps/daemon/src/prompt_transcript.rs`
- simplified `apps/daemon/src/app.rs` so history pagination and prompt transcript formatting are delegated to dedicated daemon modules

Why this mattered:

- `app.rs` had been carrying transport-adjacent orchestration alongside low-level history slicing and transcript formatting helpers
- moving those helpers out is the first Phase 2 step toward making `DaemonApp` a coordinator instead of a dumping ground

### 8. Local API provider-handler extraction

Done:

- extracted provider-launch, provider-run lookup, and provider-catalog request handling into `apps/daemon/src/local/provider_requests.rs`
- simplified `apps/daemon/src/local/api.rs` so it keeps request/response types and dispatches provider-specific requests through dedicated local handlers

Why this mattered:

- `local/api.rs` had still been mixing transport dispatch with OpenCode-specific request-building and catalog-fetch logic
- moving those branches out leaves the local API layer closer to a transport boundary and narrows the next refactor seam to provider service internals

### 9. OpenCode runtime extraction

Done:

- extracted OpenCode runtime state, event draining, snapshot replay, and transcript rendering into `apps/daemon/src/provider/opencode_runtime.rs`
- simplified `apps/daemon/src/provider/service.rs` so generic provider-run lifecycle code delegates provider-specific stream/render handling to the OpenCode runtime module

Why this mattered:

- `service.rs` had still been interleaving generic run lifecycle transitions with a large OpenCode-only event/render state machine
- moving that state machine out leaves a much smaller provider boundary and makes the remaining provider-specific setup logic easier to isolate later

## Current State

The refactor has started, but the largest simplification targets still remain:

- `apps/cli/src/index.tsx` is still the main monolith
- `apps/daemon/src/app.rs` is still orchestration-heavy, but the history/prompt helper extraction is complete
- `apps/daemon/src/local/api.rs` is now mostly request/response definitions plus transport dispatch
- `apps/daemon/src/provider/service.rs` now focuses on provider lifecycle/orchestration, while `opencode_runtime.rs` owns event parsing and transcript rendering

## Next Phases

### Phase 1. Continue splitting the TypeScript CLI

Primary target:

- break `apps/cli/src/index.tsx` into smaller modules by responsibility

Next extraction seams:

- Phase 1 extraction seams are complete for this pass

Expected outcome:

- smaller, testable units around rendering, session state, and transport
- less duplicated agent/pane bookkeeping
- lower change risk for future multi-agent UI work

### Phase 2. Simplify daemon orchestration boundaries

Targets:

- Phase 2 extraction seams are complete for this pass

Expected outcome:

- `DaemonApp` becomes the coordinator instead of the dumping ground
- local IPC handling becomes easier to reason about and test
- provider-specific behavior stops leaking across transport and orchestration layers

### Phase 3. Split provider lifecycle from provider-specific rendering

Targets:

- separate generic provider-run lifecycle management from the remaining OpenCode session bootstrap and run-selection sync logic
- keep provider state transitions, process control, runtime binding, and transcript rendering in different modules

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

The next concrete change should stay in Phase 2 by reducing `apps/daemon/src/local/api.rs` to transport/request handling only.
