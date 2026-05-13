# Cross-Repo Architecture Boundary Refactor Plan

## Status

Planning baseline for the next architecture refactor across:

- `arroba`: OSS runtime, kernel, relay, CLI, shell, provider adapters, shared client protocol.
- `arroba-cloud`: hosted control plane, browser app, auth, relay token issuance, waiting room, browser terminal bootstrap.

This plan intentionally excludes iOS. The iOS app is early enough that it should follow the stabilized protocol and boundaries after this refactor, not drive the cutover.

## Progress

- 2026-05-13: Cloud browser relay kernel bootstrap ownership cutover started in `arroba-cloud`. `/browser/relay-kernel/bootstrap` now delegates target selection, relay URL validation, relay token minting, and cached waiting-room snapshot lookup to `CloudApiService.bootstrapBrowserRelayKernel`; target selection/relay URL validation live in a focused `browser-relay-target-selection` module; bootstrap route registration moved into `routes/browser-relay-kernel.ts`; browser session/dashboard/waiting-room-cache/cloud-session/logout routes moved into `routes/browser-session.ts`; device login, dev login, poll, and logout routes moved into `routes/device-login.ts`; session invite/member/collaborator routes moved into `routes/session-invites.ts`; account/user admin read and mutation routes moved into `routes/admin.ts`; browser billing checkout/portal and Stripe webhook routes moved into `routes/billing.ts`; runtime relay token, kernel presence, and relay target listing routes moved into `routes/relay.ts`; managed history record/search/export routes moved into `routes/managed-history.ts`; account bootstrap, organization creation, account listing, and audit event listing routes moved into `routes/account-control.ts`; pairing token, client/machine pair/revoke, and browser paired identity revoke routes moved into `routes/pairing.ts`; web/admin static app shell registration moved into `http/web-apps.ts`; request/route schemas moved into `http/route-schemas.ts`; browser identity, CSRF, dev auth, admin permission, and browser cloud-session token helpers moved into `http/browser-security.ts`; request primitives moved into `http/request.ts`; API error mapping/body normalization moved into `http/error-handling.ts`; Stripe webhook parsing moved into `http/billing-webhooks.ts`; browser relay request protocol helpers moved into `http/browser-relay-request.ts`; `server-helpers.ts` was deleted after active imports were moved to responsibility modules; API architecture tests guard against active `/web-cli` routes and direct bootstrap token minting in the route module.
- 2026-05-13: `contracts.ts` split started in `arroba-cloud`; admin account/user search, detail, mutation, purge, summary, and content-count contracts moved into `contracts/admin.ts`, with `contracts.ts` preserving compatibility exports.
- 2026-05-13: Managed-history policy, record append, search, and export job contracts moved from `contracts.ts` into `contracts/managed-history.ts`, with compatibility exports preserved.
- 2026-05-13: Account bootstrap, organization creation, account listing, and audit event contracts moved from `contracts.ts` into `contracts/account-control.ts`, with compatibility exports preserved.
- 2026-05-13: Device login approval/polling and logout contracts moved from `contracts.ts` into `contracts/device-login.ts`, with compatibility exports preserved.
- 2026-05-13: Shared session invite, session member, and collaborator contracts moved from `contracts.ts` into `contracts/session-invites.ts`, with compatibility exports preserved.
- 2026-05-13: Browser billing checkout and portal contracts moved from `contracts.ts` into `contracts/billing.ts`, with compatibility exports preserved.
- 2026-05-13: Pairing token, client/machine pair, machine runtime profile, and client/machine revoke contracts moved from `contracts.ts` into `contracts/pairing.ts`, with compatibility exports preserved.
- 2026-05-13: Runtime relay token, relay target listing, and kernel presence contracts moved from `contracts.ts` into `contracts/relay.ts`, with compatibility exports preserved.
- 2026-05-13: Browser dashboard/cloud-session/waiting-room cache contracts moved into `contracts/browser-session.ts`, and browser relay-kernel bootstrap contracts moved into `contracts/browser-relay-kernel.ts`, with compatibility exports preserved.
- 2026-05-13: Cloud API health/readiness/metrics/audit operational contracts moved from `contracts.ts` into `contracts/operational.ts`, with compatibility exports preserved.
- 2026-05-13: Cloud API HTTP adapter request/response contracts moved from `contracts.ts` into `contracts/http.ts`, with compatibility exports preserved.
- 2026-05-13: Cloud API service construction options and relay realm allocation contracts moved from `contracts.ts` into `contracts/service-options.ts`, with compatibility exports preserved.
- 2026-05-13: `CloudApiService` moved from `contracts.ts` into `contracts/service.ts`; `contracts.ts` is now a compatibility barrel over focused domain contract files.
- 2026-05-13: Browser terminal storage ownership extraction started in `arroba-cloud`. Current active browser storage keys now use the `arroba:terminal:*` namespace, with one-time read/migration fallback from legacy `arroba:web-cli:*` keys. Prompt draft persistence moved into `terminal/prompt-draft-store.ts`; badge trace persistence, size bounding, opt-in flag, and max-size storage moved into `terminal/badge-trace-store.ts`; key naming and migration helpers live in `terminal/storage-keys.ts`; `client.ts` now delegates storage details through narrow wrappers while keeping UI orchestration behavior unchanged.
- 2026-05-13: Browser prompt attachment source naming was cleaned up across both repos. `arroba-cloud` now builds inline prompt attachment source URLs through `terminal/prompt-attachment-source.ts` using the `arroba-terminal://prompt-attachment/...` scheme, while `arroba` materializes inline prompt attachment bytes under `arroba-terminal-prompt-attachments`; focused web and kernel tests cover both names. An OSS runtime integration test callsite was also updated to the current explicit `send_terminal_input(..., provider_run_id, bytes)` signature so focused kernel test builds compile cleanly.
- 2026-05-13: Active Cloud badge/workflow drill naming moved to terminal-badge. `package.json` now exposes `smoke:terminal-badge`; `smoke:workflow-canvas` uses `scripts/terminal-badge-drill.mjs`; the fragmented drill directory, issuer/realm/run id, logs, and local test secrets now use terminal-badge naming.
- 2026-05-13: Browser terminal session registry ownership moved out of `client.ts` into `terminal/session-store.ts`. The new store module owns active-session ids, record lookup, activation, active clearing, and deletion; `client.ts` now keeps side-effecting timer/DOM transitions but no longer reaches directly into the registry map/id. Unit coverage was added for active record tracking and active-id clearing on delete.
- 2026-05-13: Browser terminal prompt input history state moved from `client.ts` into `terminal/prompt-history-state.ts`, with focused unit coverage for the empty/idle initial state.
- 2026-05-13: Browser terminal session value state moved from `client.ts` into `terminal/session-state.ts`, with focused unit coverage for the detached initial session shape.
- 2026-05-13: Browser terminal capabilities sidebar state moved from `client.ts` into `terminal/capabilities-state.ts`, with focused unit coverage for the idle/no-selection initial state.
- 2026-05-13: Browser terminal workspace panel state/types moved from `client.ts` into `terminal/workspace-state.ts`, with focused unit coverage for the initial changes-tab/no-workspace-data state. `client.ts` still owns the workspace controller/rendering orchestration.
- 2026-05-13: Browser terminal prompt attachment value types moved from `client.ts` into `terminal/prompt-attachment-state.ts`, with focused unit coverage keeping view fields separate from upload/progress fields.
- 2026-05-13: Browser terminal runtime state/value types moved from `client.ts` into `terminal/runtime-state.ts`. The module now owns the runtime state factory plus agent, output entry, turn, interaction, pending-frame, and agent-history state types; source-inspection tests were redirected to the new owner.
- 2026-05-13: Browser terminal history search state moved from `client.ts` into `terminal/history-state.ts`. The module owns keyword/semantic mode, pagination cursors, selected-event context, result pages, and default search status; focused unit coverage verifies the initial keyword-search state.
- 2026-05-13: Browser kernel directory state and target helpers moved from `client.ts` into `terminal/kernel-directory.ts`. The module owns relay target summaries, runtime view records, directory refresh bookkeeping, target ref/request normalization, online filtering, runtime-view selection, and labels; focused unit coverage verifies initial state and target selection.
- 2026-05-13: Browser terminal prompt attachment session state moved from loose `client.ts` globals into `terminal/prompt-state.ts`. Active terminal records now persist a single prompt state object for attachments and object URL lifecycle, with focused unit coverage for the empty initial state.
- 2026-05-13: Browser history search projection moved from `client.ts` into `terminal/history-projection.ts`. The module owns sidebar/route view models, selected-result detail metadata, context event projection, pagination clamping, result dedupe, and context merge helpers; focused unit coverage verifies disconnected/ready views, detail projection, and pure helpers.
- 2026-05-13: Browser kernel subscription resume storage moved from `browser-kernel-client.ts` into `kernel/browser-kernel-subscriptions.ts`. The module owns subscription keys, event context projection, persisted resume cursor serialization, waiting-room subscription exclusion, and storage cleanup while preserving the public `BrowserKernelClient` API.
- 2026-05-13: Browser kernel request correlation moved from `browser-kernel-client.ts` into `kernel/browser-kernel-request-correlation.ts`. The module owns pending request registration, timeout cleanup, lane counts, lane-scoped rejection, request-kind detection, safe request summaries, and relay error message formatting.
- 2026-05-13: Browser kernel relay transport primitives moved from `browser-kernel-client.ts` into `kernel/browser-kernel-transport.ts`. The module owns relay target/frame types, JSON frame parsing, token-expiry parsing, websocket close diagnostics, and websocket error text; public target and lane types remain re-exported for compatibility.
- 2026-05-13: Browser kernel event dispatch moved from `browser-kernel-client.ts` into `kernel/browser-kernel-events.ts`. The module owns the `KernelEvent` union, event handler registration/removal, handler presence checks for reconnect eligibility, and dispatch fanout while `BrowserKernelClient.onKernelEvent` remains unchanged.

## Summary

Refactor `arroba` and `arroba-cloud` together while preserving compatibility.

Architecture boundaries:

- Kernel owns runtime authority: sessions, agents, provider runs, workflows, prompt state, terminal events, history, workspaces, worktrees, and runtime state transitions.
- Relay is opaque transport only. It admits scoped connections and forwards encrypted packets; it must not inspect prompts, outputs, workspace data, provider payloads, or history.
- Cloud owns auth, entitlement, relay token issuance, target selection, waiting-room/control-plane state, and browser bootstrap. Cloud must not become a runtime proxy or session authority.
- Clients render state and submit commands through the shared kernel protocol. They must not fork runtime semantics.

This is behavior-preserving by default. No protocol shape changes are intended. If a serialized shape changes, follow the protocol rule: bump the shared local daemon protocol version, update snapshot/hash tests, update client minimum versions only when needed, and add a focused drill.

## Refactor Principle: Responsibility-First, Not File Sharding

The goal is not to split large files into arbitrary chunks. Every new file or module must have a named owner responsibility and a stable dependency direction.

Allowed extractions:

- Move a complete responsibility behind a clear public boundary, such as command admission, session mutation, relay bootstrap, event replay, prompt lifecycle, browser terminal session state, or waiting-room projection.
- Extract pure domain logic with tests before moving side effects.
- Extract an adapter around an external boundary, such as Fastify routes, WebSocket transport, provider process I/O, browser crypto, or DOM mounting.
- Keep compatibility barrels only where they preserve public imports during migration.

Disallowed extractions:

- `client-part-1.ts`, `router-helpers.ts`, `server-utils.ts`, or similar bucket files.
- Moving private helper functions by line range without changing ownership.
- Creating modules that still need broad access to unrelated state.
- Splitting render, state mutation, network I/O, and policy into the same new module.
- Adding a second compatibility path without deleting the old helper in the same slice or naming a concrete blocker.

Module acceptance rule:

- A future engineer should be able to state the module's responsibility in one sentence.
- The module should import only the stores/services/contracts needed for that responsibility.
- Tests should exercise behavior through the new responsibility boundary, not through the old mega-file.

## Key Changes

### OSS Kernel

- Add a real kernel composition boundary, for example `apps/kernel/src/runtime/kernel.rs`, that owns construction of the router, runtime state, projections, actors, transport health, terminal stores, provider lanes, workspace coordination, and background schedulers.
- Keep `DaemonApp` as bootstrap/shutdown/durable snapshot compatibility only. Runtime command paths must depend on cloneable owned stores or named runtime ports, not `Arc<Mutex<DaemonApp>>`.
- Split `CommandRouter` by responsibility:
  - `CommandRouter`: command admission, authorization, priority routing, command metadata, response redaction.
  - `SessionCommandExecutor`: session lifecycle, membership, links, pairing, terminal pairing.
  - `CloudControlExecutor`: cloud relay login, token, session invite, and hosted control-plane calls.
  - `WorkspaceCommandExecutor`: workspace directories, worktrees, git overview, PR/commit helpers, workspace utilities.
  - `ProviderControlExecutor`: provider auth status, login/logout, process listing/teardown, catalog reads.
  - Existing prompt, workflow, capability, terminal output, provider launch, and runtime-tool executors remain runtime-owned.
- Move `KernelEvent`, transport frames, replay envelopes, and subscription/event relevance helpers out of `runtime_transport.rs` into a transport protocol module. `runtime_transport.rs` should focus on WebSocket connection handling, replay, subscription loops, and frame I/O.
- Move remaining runtime services out of `app/` into owning runtime modules:
  - prompt lifecycle under `runtime/prompt_lifecycle`
  - provider process/output/liveness under `runtime/provider`
  - session read/mutation ports under `runtime/session`
  - remote lease runtime under `runtime/remote`
- Remove production `app.lock()` usage outside bootstrap/composition. If a slice cannot remove a use, document the blocker and do not add new call sites.
- Keep `CompatibilityRuntimeState` only as a temporary quarantine. Each slice replaces one port's internals with owned stores, then deletes the matching compatibility method.

### Shared Protocol And Clients

- Split `packages/kernel-client` into browser-safe protocol/request/event modules and Node-only transport/crypto modules.
- CLI and shell use the Node transport. Cloud web imports only browser-safe protocol, request builders, event types, response normalizers, and shared helpers.
- Keep Rust `apps/kernel/src/local/api/types.rs` as the current wire source of truth. Do not introduce schema/codegen unless protocol drift becomes the blocker.
- Preserve public request, response, event, and relay packet shapes.
- Add protocol parity tests:
  - Rust snapshot/hash tests remain in `apps/kernel/src/local/api/tests.rs`.
  - TypeScript tests assert request builders and event unions encode/decode representative shapes.
  - Swift/iOS protocol work is not part of this refactor.

### CLI And Shell

- Keep CLI as a client implementation, not a runtime authority.
- Turn `apps/cli/src/index.tsx` into a composition shell with focused responsibility modules:
  - process/kernel launch and app bootstrap
  - session runtime/controller
  - kernel event handling
  - transcript and pane state
  - command center and command action wiring
  - waiting room and remote machine state
  - native TUI launchers remain separate
- Avoid moving UI state into `packages/kernel-client`; that package should contain shared protocol/transport/shell logic only.

### Arroba Cloud API

- Keep `apps/api/src/server.ts` as Fastify composition and route registration only.
- Add focused route modules under `apps/api/src/routes/`, starting with:
  - `browser-relay-kernel.ts`
  - `browser-session.ts`
  - `relay.ts`
  - `admin.ts`
  - `billing.ts`
  - `device-login.ts`
- Add `CloudApiService.bootstrapBrowserRelayKernel(input)`.
  - Move target freshness selection, relay URL normalization, browser relay token minting, and cached waiting-room snapshot lookup into this service method.
  - `/browser/relay-kernel/bootstrap` should only read browser identity/session, call the service, and return the same response shape.
- Split `server-helpers.ts` by responsibility:
  - `http/browser-security.ts`
  - `http/route-schemas.ts`
  - `http/web-assets.ts`
  - `browser-relay-target-selection.ts`
- Split `contracts.ts` into domain contract files, with `contracts.ts` remaining a compatibility barrel.
- Preserve `/browser/relay-kernel/bootstrap`, `/dashboard`, `/relay/token`, and browser terminal route compatibility.

### Arroba Cloud Web

- Keep the current terminal behavior and React mount boundaries. Do not do a full React rewrite in this refactor.
- Turn `apps/web/src/client.ts` into browser app bootstrap/coordinator only: route mount, dependency wiring, global event registration, and app start.
- Add `apps/web/src/terminal/app/` for the terminal coordinator and dependency container.
- Extract state modules by responsibility:
  - `terminal/session-store.ts`
  - `terminal/runtime-state.ts`
  - `terminal/kernel-directory.ts`
  - `terminal/prompt-state.ts`
  - `terminal/workspace-state.ts`
  - `terminal/capabilities-state.ts`
  - `terminal/history-state.ts`
- Extract controllers by behavior:
  - `waiting-room-controller.ts`
  - `terminal-session-controller.ts`
  - `prompt-controller.ts`
  - `workspace-controller.ts`
  - `workflow-controller.ts`
  - `capabilities-controller.ts`
  - `history-controller.ts`
- Controllers must not build large HTML strings directly. Rendering/projection belongs in render modules or React mount components.
- Split `apps/web/src/ui/waiting-room-kernel.ts` into waiting-room types, reducer/state transitions, projection helpers, and rendering.
- Split `BrowserKernelClient` internals into transport, request correlation, subscription/resume storage, and event dispatch while preserving its public class API.
- Waiting-room refresh must not overwrite active terminal transcript, focus, prompt draft, selected session, or local reconnect state.

### Naming, Docs, And Cleanup

- Rename active browser runtime storage keys from `arroba:web-cli:*` to `arroba:terminal:*`, with a one-time read fallback from old keys.
- Rename active badge/runtime drills from `web-cli-*` to `terminal-*` or `browser-relay-kernel-*`.
- Remove active `WEB_CLI` references from code, scripts, and active refactor docs. Keep historical C2 or WEB_CLI notes only under explicit archive wording.
- Split stylesheet modules by feature without changing selectors in the same slice:
  - tokens/base
  - marketing shell
  - terminal shell
  - waiting room
  - freeform panes
  - workspace
  - workflows
  - history
  - responsive overrides
- Replace fragmented drill wrappers with named harness modules under `scripts/lib/`.

## Implementation Order

1. Add architecture guardrails first:
   - active Cloud app/API code contains no `/web-cli` route references
   - browser relay bootstrap route does not directly mint relay tokens after service extraction
   - no active Cloud route proxies prompts, provider output, kernel events, attachments, workflow payloads, or workspace data
   - line-budget smoke checks for `client.ts`, `server.ts`, and `waiting-room-kernel.ts` after each extraction milestone
   - no new production command-state ownership through `DaemonApp`
2. Extract browser-safe protocol modules from `packages/kernel-client`; update CLI, shell, and Cloud web imports without changing wire shapes.
3. Extract Cloud browser relay bootstrap into `CloudApiService.bootstrapBrowserRelayKernel`, preserving response shape, stale target denial, and cached waiting-room snapshot behavior.
4. Split Cloud API route modules, helper modules, and contract files behind compatibility barrels.
5. Split Cloud web terminal state modules and pure reducer/projection helpers before moving side-effecting controllers.
6. Split Cloud web render/style modules while preserving route behavior for `/waiting-room`, `/terminal`, `/history`, `/workflows`, `/machines`, and `/test`.
7. Split OSS kernel transport events/frames and router executors by responsibility, preserving request behavior.
8. Cut remaining OSS kernel runtime ownership paths domain by domain: session, prompt, provider process/output, workflow/runtime tools, capability/terminal output.
9. Split CLI composition/state/controller modules after shared protocol and kernel boundaries are stable.
10. Clean naming, storage keys, drill names, docs, and stale compatibility barrels.
11. Run the cross-repo drill gate before resuming new feature work.

## Test Plan

Per OSS slice:

- `cargo test --manifest-path apps/kernel/Cargo.toml`
- `pnpm --filter @arroba/kernel-client run test`
- `pnpm --filter @arroba/cli run test`
- `pnpm --filter @arroba/shell run test` when shared shell/client code changes

Per Cloud slice:

- `pnpm --filter @arroba-cloud/api test`
- `pnpm --filter @arroba-cloud/web test`
- `pnpm -r --if-present lint`
- `git diff --check`

Architecture-sensitive gates:

- browser relay kernel prompt flow
- stale relay target denial
- managed relay smoke
- local freeform multi-agent
- local workflow
- remote freeform relay
- remote workflow relay
- reconnect/replay-gap/session snapshot recovery
- waiting-room refresh cannot overwrite active terminal transcript/focus state
- staging retail strict smoke for deployment-sensitive changes

Protocol-sensitive gates:

- `LOCAL_DAEMON_PROTOCOL_VERSION` changes only with wire-shape changes.
- Snapshot/hash tests fail if protocol shape changes without an intentional bump.
- Browser relay bootstrap response remains unchanged unless the protocol rule is followed.

## Assumptions

- This refactor is behavior-preserving unless a stale/dead path is explicitly removed.
- Direct deletion is preferred over long compatibility windows.
- No iOS work is included.
- No kernel protocol, relay packet, or serialized local daemon shape changes are intended.
- New modules are accepted only when they represent a real responsibility boundary, not an arbitrary chunk of an existing large file.
