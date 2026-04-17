# M3 Implementation Checklist

## Status

Execution checklist for **M3 - Local Capability Surface and Provider Expansion**.

M3 is in progress as of 2026-03-18.

This checklist starts with the OpenCode structured-adapter upgrade that follows the M2 PTY bootstrap. Once that path is stable, M3 continues with daemon-owned slash commands, local capabilities, and additional providers.

Client-surface note:

- the primary local CLI surface for M3 is the TypeScript OpenTUI client in `apps/cli`

## 1. Target M3 outcomes

From `docs/ROADMAP.md`, M3 outcomes are:

- upgrade OpenCode from the M2 PTY bootstrap path to a structured local server/session/event adapter
- add explicit persistent session management with delete semantics, unattached CLI state, and session id/alias resolution
- add syntax-highlighted markdown/code rendering in the TypeScript CLI transcript for provider responses
- shell command capability
- directory tree + file view/edit capabilities
- screenshot capture capability
- git/worktree inspection capability
- file transfer + attach-transferred workflow
- daemon-owned slash-command dispatch for Arroba capabilities
- Claude Code and Codex provider support after the OpenCode baseline is solid

Exit criteria:

- OpenCode prompt lifecycle no longer depends on PTY-idle heuristics
- detached sessions remain resumable until explicit deletion, and deleting the current session returns the CLI to a no-session state
- capability failures remain isolated from the terminal lane
- multiple supported providers can run through the same daemon-managed local CLI model
- local slash-command UX is usable enough to drive capabilities without a web surface

## 2. M3 implementation principles

- Finish the OpenCode structured adapter before expanding to more providers.
- Keep Arroba's local daemon API client-facing contract stable while changing provider internals.
- Prefer provider-native machine-readable surfaces over PTY scraping when a provider supports them.
- Preserve the M2 PTY baseline as a fallback path only where structured integration is unavailable.
- Bring already-implemented capabilities onto the new command/runtime path instead of rebuilding them in parallel.

## 3. Current progress snapshot

- [x] Core docs now describe OpenCode as the first structured local provider adapter target.
- [x] The daemon provider model now has a structured-endpoint field for provider runs.
- [x] The OpenCode launch path now plans `opencode serve` with a daemon-known local endpoint.
- [x] A first OpenCode HTTP client exists in the daemon for health check, session creation, prompt submission, abort, and session polling.
- [x] The provider layer now owns OpenCode runtime state for provider endpoint/session binding and structured event-driven output.
- [x] Runtime tests now have a mock OpenCode HTTP server fixture path for the structured adapter migration.
- [x] Unexpected provider exit now removes leaked PTY runtime state instead of only marking the run ended.
- [x] Missing OpenCode session status now surfaces as a provider error instead of being treated as an implicit `idle` turn completion.
- [x] OpenCode startup no longer bind-allocates a free local port; the daemon now requires an explicit configured port to avoid the selection race.
- [x] The TypeScript CLI now retries transient IPC polling failures instead of treating the first poll error as immediately fatal.
- [x] The TypeScript CLI now keeps exit-cleanup failures visible instead of immediately exiting successfully and hiding them.
- [x] Explicit persistent session-management work has started: sessions now use commit-like ids plus optional aliases, explicit delete exists, and the CLI can return to a no-session landing state after deletion.
- [x] TypeScript CLI transcript highlighting now has an explicit M3 plan separate from LSP semantics.

## 4. Workstreams

## 4.1 OpenCode Structured Runtime Bootstrap

- [x] Add provider-run metadata for structured endpoints.
- [x] Launch OpenCode in headless server mode with a daemon-known local endpoint.
- [x] Wait for provider health and create an OpenCode session during provider-run initialization.
- [x] Persist structured provider session metadata in a provider-owned runtime layer instead of only in `DaemonApp`.
- [ ] Add cleanup guarantees for OpenCode session/runtime state on provider termination, daemon shutdown, and failed initialization.
Current state: failed initialization, normal session teardown, and unexpected provider exit now clean up PTY/runtime state; daemon-shutdown cleanup still needs explicit coverage.

## 4.2 OpenCode Prompt and Output Path

- [x] Route OpenCode prompt submission through provider session APIs instead of PTY writes.
- [x] Start deriving OpenCode terminal output from structured provider-native APIs.
- [x] Stop treating missing OpenCode session state as an implicit idle turn.
- [x] Replace polling-based message snapshots with OpenCode SSE event ingestion.
- [x] Map OpenCode `session.status` and assistant completion timestamps into the canonical Arroba prompt lifecycle.
- [x] Remove OpenCode dependence on idle-timeout prompt completion heuristics.
- [x] Surface OpenCode `session.error` and tool lifecycle events as richer client-facing output.
Current state: OpenCode `session.error` now reaches the transcript as a distinct provider-error stream instead of only a notice, and tool/status updates are rendered as stable structured transcript blocks in the TypeScript CLI.
- [x] Add explicit Arroba stop/cancel behavior mapped to OpenCode `session.abort`, with queue advancement deferred until provider confirmation.

## 4.3 Project-Wide Logging and Debugging

- [x] Define one daemon-owned machine-local log root for Arroba runtime processes.
Current state: `ARROBA_LOG_DIR` overrides the default; otherwise Arroba uses `XDG_STATE_HOME/arroba/logs`, then `~/.local/state/arroba/logs`, then `./.arroba/logs`.
- [x] Use structured log records with shared correlation fields:
  - timestamp
  - level
  - component
  - process kind
  - pid
  - session id
  - provider run id
  - attachment id/client id when applicable
  - request id / trace id when applicable
- [x] Write logs per process under the shared log root instead of forcing all processes to contend for one file.
- [ ] Cover at least:
  - daemon
  - TypeScript CLI
  - server process
  - future provider-side helper processes when Arroba launches them directly
Current state: daemon, TypeScript CLI launcher, TypeScript CLI, and server are on the shared logger; provider-side helper processes remain pending.
- [ ] Add a debug-bundle or log-collection path for one session/provider run across multiple local processes.
- [x] Define default privacy policy for logs:
  - metadata/error logs by default
  - prompt/output content capture only with explicit opt-in or debug mode
- [x] Document log location, rotation/retention policy, and env vars for local debugging.
- [x] Add an early built-in local log viewer command.
Current state: `arroba-cli logs` can filter/follow the shared NDJSON logs by process kind, component, session, provider run, client, and level.

## 4.4 TypeScript CLI Hardening

- [x] Add syntax-highlighted fenced code block rendering in assistant/reasoning transcript entries.
- [x] Add markdown-aware rendering for assistant and reasoning transcript entries instead of treating all provider text as plain wrapped text.
- [x] Register a trimmed parser set plus common fence-language alias normalization for terminal transcript rendering.
- [x] Surface OpenCode `session.error` and tool lifecycle events as richer TypeScript CLI transcript/status output instead of plain notices only.
- [ ] Add TypeScript CLI integration-level tests for bootstrap, transcript rendering, polling recovery, and exit cleanup.
Current state: startup session-selection/bootstrap coverage now exists in `apps/cli/src/sessions.test.ts`, but live transcript/runtime integration coverage is still incomplete.
- [ ] Keep the TypeScript client and the Rust launcher aligned on help text, env vars, and expected local startup flow.

## 4.5 Persistent Session Management

- [x] Separate user-facing session deletion from detach/exit semantics.
- [x] Add daemon-owned `session.delete` semantics that tear down runtime state without overloading ordinary detach.
- [x] Make `session.delete` a true delete rather than a resumable `session.end` alias.
- [x] Keep sessions resumable after the last client detaches until explicit deletion.
- [x] Add a reusable no-session TypeScript CLI state instead of always exiting the process when session context disappears.
- [x] When the current session is deleted, clear transcript/session chrome and render an Arroba ASCII-art unattached landing state.
- [x] Add session references based on commit-like ids plus optional aliases.
- [x] Allow attach/delete commands to resolve full ids, unique id prefixes, aliases, and unique alias prefixes.
- [x] Reject ambiguous session references with a structured error instead of guessing.
- [x] Add temporary session-management commands for create/attach/delete ahead of the slash-command system.

## 4.6 Slash Commands and OpenCode Discovery

- [ ] Add daemon-owned slash-command dispatch to the local CLI path.
- [ ] Prefer OpenCode machine-readable command discovery over catalog-only fallback.
- [ ] Integrate OpenCode agent and skill discovery into `/agent` completion state.
- [ ] Keep shipped Arroba catalogs as compatibility fallback when provider APIs are unavailable or unsupported.

## 4.7 Capability Surface on the New Path

- [ ] Wire shell capability through slash-command dispatch.
- [ ] Wire directory tree and file view/edit capabilities through slash-command dispatch.
- [ ] Wire screenshot capture through slash-command dispatch.
- [ ] Wire git/worktree inspection through slash-command dispatch.
- [ ] Wire file transfer and attach-transferred flows through slash-command dispatch.
- [ ] Add CLI rendering for capability results distinct from provider output.

## 4.8 Additional Providers

- [ ] Add Claude Code provider support using the same daemon-owned local CLI model.
- [ ] Add Codex provider support using the same daemon-owned local CLI model.
- [ ] Document provider-specific capability/auth/command discovery differences.

## 5. Testing and Verification

- [x] Add initial TypeScript CLI behavior tests for retry and exit-cleanup policy helpers.
- [ ] Add broader TypeScript CLI behavior tests for bootstrap, transcript rendering, polling/recovery, exit cleanup, and no-session-state transitions.
- [ ] Add unit tests for the OpenCode structured client and adapter state transitions.
- [ ] Add integration tests for health-check, session-create, prompt-submit, and structured output polling.
- [ ] Add integration tests for OpenCode abort, server death, and failed initialization cleanup.
Current state: session-teardown abort coverage, server-death cleanup, and missing-session regressions are covered; failed-initialization cleanup still needs direct tests.
- [x] Add integration coverage for SSE-based event ingestion once implemented.
- [ ] Keep existing daemon tests passing while the OpenCode adapter migrates away from PTY-driven prompt delivery.

## 6. Suggested execution order

1. OpenCode structured runtime bootstrap
2. prompt submit and structured output path
3. prompt lifecycle completion from provider signals
4. project-wide logging/debugging foundation
5. persistent session management and no-session CLI state
6. TypeScript CLI hardening and richer event rendering
7. slash-command dispatch
8. capability wiring
9. additional providers

## 7. Verification commands for claiming meaningful M3 progress

Run and pass locally before claiming meaningful M3 progress:

```bash
pnpm lint
pnpm build
pnpm test
cargo test --manifest-path apps/kernel/Cargo.toml
```

Recommended additional Rust checks:

```bash
cargo fmt --manifest-path apps/kernel/Cargo.toml --check
cargo clippy --manifest-path apps/kernel/Cargo.toml --all-targets --all-features -- -D warnings
```
