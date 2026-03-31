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

### 10. OpenCode binding extraction

Done:

- extracted OpenCode session bootstrap, health checks, prompt submission, abort handling, and run-selection sync into `apps/daemon/src/provider/opencode_binding.rs`
- simplified `apps/daemon/src/provider/service.rs` so it mainly coordinates generic provider lifecycle and delegates OpenCode-specific session binding work to dedicated provider modules

Why this mattered:

- `service.rs` still held the last chunk of OpenCode-specific client/bootstrap logic after the runtime state machine had already moved out
- moving that code out completes the current Phase 3 split between generic lifecycle control and provider-specific binding/rendering details

### 11. Launcher build freshness checks

Done:

- updated `apps/daemon/src/bin/arroba-cli.rs` so the Rust launcher only rebuilds `apps/cli` when TypeScript CLI outputs are missing or older than the CLI sources/build inputs
- added focused launcher tests covering missing-output, fresh-output, and stale-output cases

Why this mattered:

- the launcher had been paying a full `pnpm --filter @arroba/cli run build` cost on every startup even when nothing changed
- checking freshness first makes the local entrypoint faster and keeps rebuild work tied to actual source changes

### 12. CLI session-state core extraction

Done:

- extracted detached-session construction, session layout derivation, prompt-work detection, and session-transition state calculation into `apps/cli/src/session-state.ts`
- added focused tests in `apps/cli/src/session-state.test.ts`
- simplified `apps/cli/src/index.tsx` so `applySessionState()` now delegates its pure state calculations to the extracted module instead of computing them inline

Why this mattered:

- `index.tsx` still mixed rendering with low-level session-state derivation
- moving the first pure state cluster out establishes the second-pass CLI refactor around tested state transitions instead of one-off helper extraction

### 13. CLI attach/detach transition extraction

Done:

- extracted detached waiting-room reset state and attached-session UI reset state into `apps/cli/src/session-state.ts`
- added focused transition coverage in `apps/cli/src/session-state.test.ts`
- simplified `apps/cli/src/index.tsx` so `transitionToNoSession()` and the post-attach state reset path now delegate their pure transition bundles to the CLI state core

Why this mattered:

- attach/detach transitions had still been encoded as large inline state mutation bundles in `index.tsx`
- moving those reset bundles out makes the remaining CLI state work more mechanical and reduces the chance of drift across session-entry and session-exit paths

### 14. CLI waiting-room controller extraction

Done:

- extracted waiting-room normalization, activation, and model/variant selection decisions into `apps/cli/src/waiting-room-controller.ts`
- added focused controller coverage in `apps/cli/src/waiting-room-controller.test.ts`
- simplified `apps/cli/src/index.tsx` so waiting-room selection and launch handlers now delegate their decision logic instead of mixing validation, normalization, and side-effect setup inline

Why this mattered:

- the waiting-room flow had still been combining state normalization, command validation, and attached-session launch preparation inside `index.tsx`
- moving those decisions into a tested controller keeps Phase 5 centered on explicit CLI state transitions and reduces the amount of session-entry logic coupled to the render shell

### 15. CLI session chrome runtime-state extraction

Done:

- extracted current provider/model selection, prompt-meta state, prompt-usage state, footer hint derivation, session status mode, and attached footer summary formatting into `apps/cli/src/session-chrome-state.ts`
- added focused coverage in `apps/cli/src/session-chrome-state.test.ts`
- simplified `apps/cli/src/index.tsx` so `updateSessionChrome()` now delegates its runtime-state derivation instead of assembling status and summary text inline

Why this mattered:

- `updateSessionChrome()` had still been a dense mix of state inspection, prompt-meta formatting, and session summary rendering
- moving that derivation into a tested module gives Phase 5 a clearer boundary between state calculation and TUI mutation, which is the main architectural goal of this second pass

### 16. CLI status-indicator state extraction

Done:

- extracted focused status-badge derivation and visible-activity-label selection into `apps/cli/src/session-chrome-state.ts`
- added focused coverage in `apps/cli/src/session-chrome-state.test.ts`
- simplified `apps/cli/src/index.tsx` so the status-indicator path and active-activity selection no longer recompute those pure decisions inline

Why this mattered:

- the status badge and visible activity label were still being derived inside the main CLI shell even after the rest of the chrome state had moved out
- moving them into the state layer completes the planned Phase 5 extraction seam for transcript-visible session chrome decisions

### 17. CLI slash-command routing extraction

Done:

- extracted slash-command parsing and top-level dispatch into `apps/cli/src/commands.ts`
- added focused command parsing/dispatch coverage in `apps/cli/src/commands.test.ts`
- simplified `apps/cli/src/index.tsx` so prompt submission and command-center execution no longer duplicate the same slash-command routing branches inline

Why this mattered:

- `index.tsx` had still been duplicating slash-command detection and routing across the command-center and prompt-submit paths
- moving the routing seam out starts Phase 6 with a behavior-preserving split between command parsing/dispatch and the TUI shell

### 18. CLI polling/effect controller extraction

Done:

- extracted generic poll-loop retry/session-unavailable/fatal handling into `apps/cli/src/polling-effects.ts`
- extracted connection-watchdog decision logic into the same controller module
- added focused coverage in `apps/cli/src/polling-effects.test.ts`
- simplified `apps/cli/src/index.tsx` so the background output/notices/session polling path now delegates retry and health decisions instead of carrying that state machine inline

Why this mattered:

- `index.tsx` had still been mixing long-running transport-effect policy with transcript/session UI orchestration
- moving the polling state machine out starts the second half of Phase 6 and makes recovery behavior testable without booting the full TUI

### 19. CLI background refresh effect extraction

Done:

- extracted transcript-scroll history-load decisions and waiting-room intro animation decisions into `apps/cli/src/background-effects.ts`
- added focused coverage in `apps/cli/src/background-effects.test.ts`
- simplified `apps/cli/src/index.tsx` so the transcript scroll monitor, short-viewport history check, and waiting-room animation loop now delegate their control decisions to a dedicated background-effects module

Why this mattered:

- `index.tsx` had still been carrying several recurring background-effect decision branches even after polling/recovery moved out
- extracting those decisions continues Phase 6 by reducing non-render loop policy inside the TUI shell and making history/intro behavior easier to test directly

### 20. CLI command-action extraction

Done:

- extracted session, provider, model, variant, view, and agent action handlers into `apps/cli/src/command-actions.ts`
- added focused helper coverage in `apps/cli/src/command-actions.test.ts`
- simplified `apps/cli/src/index.tsx` so the main slash-command side-effect handlers are now wired through a dedicated action module instead of living inline beside the TUI shell

Why this mattered:

- even after parsing/dispatch moved out, `index.tsx` still contained a large command side-effect block for session and agent operations
- moving those handlers out completes the main Phase 6 command split and leaves the CLI shell closer to orchestration plus rendering only

### 21. CLI session-lifecycle extraction

Done:

- extracted no-session transition, attachment detach, and attach-binding orchestration into `apps/cli/src/session-lifecycle.ts`
- added focused controller coverage in `apps/cli/src/session-lifecycle.test.ts`
- simplified `apps/cli/src/index.tsx` so the main session entry/exit workflow no longer lives inline beside transcript/render orchestration

Why this mattered:

- `index.tsx` still had one large session-lifecycle workflow cluster even after command and polling extraction
- moving that cluster out completes the final planned CLI extraction pass and leaves the remaining shell much closer to TUI composition plus local coordination glue

## Current State

The refactor has started, but the largest simplification targets still remain:

- `apps/cli/src/index.tsx` is still the main monolith
- `apps/daemon/src/app.rs` is still orchestration-heavy, but the history/prompt helper extraction is complete
- `apps/daemon/src/local/api.rs` is now mostly request/response definitions plus transport dispatch
- `apps/daemon/src/provider/service.rs` now focuses on provider lifecycle/orchestration, while `opencode_binding.rs` and `opencode_runtime.rs` own OpenCode-specific binding and transcript behavior
- `apps/daemon/src/bin/arroba-cli.rs` now skips unnecessary CLI rebuilds by checking build freshness first
- `apps/cli/src/index.tsx` still contains most runtime wiring, but the session-state, attach/detach, waiting-room decision, and session-chrome runtime-state seams are now extracted

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

- Phase 3 extraction seams are complete for this pass

Expected outcome:

- cleaner OpenCode-first implementation
- easier future extension to additional providers without polluting the generic runtime path

### Phase 4. Startup/build-path cleanup

Targets:

- Phase 4 extraction seams are complete for this pass

Expected outcome:

- faster local startup
- simpler launcher behavior

### Phase 5. Establish a CLI state core

Targets:

- Phase 5 extraction seams are complete for this pass

Expected outcome:

- `index.tsx` becomes a thinner app shell instead of the state engine
- CLI state transitions become testable without rendering the TUI

### Phase 6. Split CLI commands and effects

Targets:

- command parsing/dispatch extraction is complete for this pass
- polling/recovery controller extraction is complete for this pass
- first background refresh-loop extraction seams are complete for this pass
- command-action handler extraction is complete for this pass
- session-lifecycle extraction is complete for this pass
- additional CLI shell splits are optional later, not the current priority

Expected outcome:

- command behavior stops being coupled to rendering concerns
- transport and recovery behavior become easier to reason about and test

### Phase 7. Decompose daemon app coordination

Targets:

- extract prompt lifecycle and queue advancement orchestration from `apps/daemon/src/app.rs`
- split attachment/session membership cleanup and terminal fanout coordination into dedicated daemon modules

Expected outcome:

- `DaemonApp` becomes a thinner composition shell
- daemon behavior becomes easier to change without touching one large file

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

The next concrete change should start Phase 7 by extracting prompt lifecycle and queue advancement orchestration out of `apps/daemon/src/app.rs`.
