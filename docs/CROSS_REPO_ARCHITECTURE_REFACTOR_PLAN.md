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
- 2026-05-13: Waiting-room core domain/state types moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-types.ts`, with `waiting-room-kernel.ts` re-exporting the existing public type surface. This starts the planned waiting-room split without changing reducer, projection, or rendering behavior.
- 2026-05-13: OSS kernel transport protocol ownership started in `arroba`. `KernelEvent`, websocket request/response frames, transport error normalization, waiting-room subscription scope constants, event stream id helpers, replay relevance checks, and kernel-event trace projection moved from `runtime_transport.rs` into `transport/kernel_protocol.rs`. The local websocket server still owns connection handling, subscription loops, replay flow, and backpressure, while the daemon relay client now shares the same event/scope helpers instead of duplicating protocol strings.
- 2026-05-13: OSS runtime session cleanup moved slice-record ownership out of the `DaemonApp` lock path. `KernelRuntimeState` now receives the cloneable `SliceStore`, and session end/delete destroys attached slice records using runtime-owned stores plus the config projection rather than locking the app. The no-app-lock session end/delete tests now pass directly.
- 2026-05-13: Cloud waiting-room worktree option projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-worktrees.ts`. The new module owns existing/create option construction, selected-option fallback, stable option ids, and worktree labels; `waiting-room-kernel.ts` preserves the public selected-worktree export. Focused web tests cover the new worktree responsibility.
- 2026-05-13: Cloud waiting-room provider catalog projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-provider-options.ts`. The module owns backend provider availability, model option filtering, Codex override options, configured model/variant fallback, terminal model ids, and provider/model labels; focused tests cover the provider/catalog projection boundary.
- 2026-05-13: OSS shared TypeScript kernel client split started in `packages/kernel-client`. Kernel event unions now live in `kernel-events.ts`, websocket/relay frame contracts in `kernel-transport-frames.ts`, pure websocket request normalization in `kernel-transport-requests.ts`, relay encryption in `relay-crypto.ts`, and relay connect/request helpers in `relay-transport.ts`. `ipc.ts` remains the Node local-socket/WebSocket client and preserves its public `LocalIpcClient` and `KernelEvent` exports. Focused request-normalizer tests were added, the CLI suite passes, and a stale waiting-room fixture was updated to the current normalized OpenCode fallback model id.
- 2026-05-13: Cloud waiting-room session inventory projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-sessions.ts`. The module owns active-session filtering, last-activity sorting, recent/picker/session-id selection, session and item active-work predicates, agent/workflow sorting, and session timestamp fallback helpers. `waiting-room-kernel.ts` keeps compatibility exports, and focused web tests cover session ordering, selection, activity, and stable agent ordering.
- 2026-05-13: Cloud waiting-room option picker projection/rendering moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-option-picker.ts`. The module owns provider/model/variant/mode/permission/worktree option rows, picker view state, unavailable-runtime hiding, HTML fallback rendering, and mode/permission option constants used by state transitions. `waiting-room-kernel.ts` preserves public exports, and focused tests cover provider/model/worktree options plus selected-row and unavailable-runtime behavior.
- 2026-05-13: Cloud waiting-room runtime/status predicates moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-runtime-status.ts`. The module owns connected-runtime availability, inventory/catalog loading predicates, bootstrap-pending state, redaction eligibility, and active local-edit detection. `waiting-room-kernel.ts` preserves the public `waitingRoomKernelCanLoadRuntimeData` and `waitingRoomLocalEditActive` exports, and focused web tests cover status, redaction, and inline-editor predicates.
- 2026-05-13: OSS shared TypeScript local IPC transport moved from `packages/kernel-client/src/ipc.ts` into `local-socket-transport.ts`, with `LocalIpcError` moved into `local-ipc-error.ts` and re-exported from `ipc.ts` for compatibility. `ipc.ts` now delegates local Unix-socket framing/response decoding while continuing to own the high-level `LocalIpcClient`; focused tests cover framed request/response decoding and error-envelope conversion, and the CLI suite still passes.
- 2026-05-13: OSS shared TypeScript WebSocket endpoint and transport-error diagnostics moved from `packages/kernel-client/src/ipc.ts` into `websocket-transport-diagnostics.ts`. The module owns websocket URL detection, nested error message extraction, and endpoint fallback diagnostics, with focused tests covering URL classification and error formatting while `LocalIpcClient.supportsKernelEvents()` behavior remains unchanged.
- 2026-05-13: Cloud waiting-room field value projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-field-values.ts`. The module owns provider, mode, permission, workspace, worktree, join-session, workspace-search status, and title-case display values shared by the create-session dialog and menu; `waiting-room-kernel.ts` preserves the public `workspaceSearchStatusText` export, and focused web tests cover unavailable/loading/edit/inventory states.
- 2026-05-13: Cloud waiting-room create-session dialog projection/rendering moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-create-session-dialog.ts`. The module owns the dialog row view model, selected field projection, alias/workspace editor metadata, and fallback HTML renderer while `waiting-room-kernel.ts` preserves public dialog exports. Focused web tests cover row ordering, selected alias state, workspace editor metadata, and fallback controls.
- 2026-05-13: Cloud waiting-room session display labels moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-labels.ts`. The module owns session id/alias labels, label-column sizing, workspace/worktree display fallback, and last-active date formatting; focused web tests cover alias trimming, width bounds, fallback order, and timestamp formatting. `waiting-room-kernel.ts` now consumes these projections from its renderer path and is down to 2,324 lines.
- 2026-05-13: Cloud waiting-room session item details moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-items.ts`. The module owns agent labels, model detail normalization, workspace detail fallback, agent workspace-search status text, and workflow label/topology summaries shared by sidebar and session picker render paths. Focused web tests cover agent refs/aliases, model prefix stripping, non-default workspace labels, workspace edit status text, and workflow node/edge/endpoint summaries; `waiting-room-kernel.ts` is down to 2,263 lines.
- 2026-05-13: Cloud waiting-room session picker projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-picker.ts`. The module owns the React-facing session picker view model, session/agent alias projections, expanded item config projection, workflow links/details, and hidden/empty picker behavior while fallback HTML rendering remains in `waiting-room-kernel.ts`. Focused web tests cover closed/empty picker states, expanded agent/workflow rows, model/workspace config projection, alias editing, and collapsed item omission; `waiting-room-kernel.ts` is down to 2,025 lines.
- 2026-05-13: OSS shared TypeScript WebSocket pending request lifecycle moved from `packages/kernel-client/src/ipc.ts` into `websocket-pending-requests.ts`. The registry owns request timeout handling, relay private-key attachment, response lookup/removal, write-failure rejection, and lane-scoped pending rejection while `LocalIpcClient` continues to coordinate sockets, subscriptions, relay framing, and event dispatch. Focused registry tests cover relay-key retention, lane rejection, and idempotent write-failure rejection; kernel-client and CLI test suites pass.
- 2026-05-14: Cloud waiting-room sidebar projection moved from `ui/waiting-room-kernel.ts` and React-owned types into `ui/waiting-room-sidebar-panel.ts`. The module owns sidebar status/list messages, workspace session grouping, active/expanded session projection, agent config/editor projection, workflow detail projection, and the sidebar view types consumed by React. `WaitingRoomSidebarPanelMount.tsx` now imports view types from the UI projection module, removing the prior UI-to-React type dependency; focused web tests cover redaction, grouping/sort order, active session expansion, alias editing, agent workspace editor state, and workflow summaries. `waiting-room-kernel.ts` is down to 1,805 lines.
- 2026-05-14: OSS shared TypeScript relay subscription frame construction moved from `packages/kernel-client/src/ipc.ts` into `relay-transport.ts`. The relay transport module now owns client subscribe/unsubscribe frame builders, target validation for subscription frames, and unsubscribe public-key derivation while `LocalIpcClient` keeps socket writes and subscription state. Focused tests cover scoped subscribe frames, absent-scope subscribe frames, unsubscribe public-key derivation, and target validation; kernel-client and CLI suites pass.
- 2026-05-14: Cloud waiting-room navigation row keys moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-navigation.ts`. The module owns the focus row order, recent-session row-key encoding, selected row-key projection, and insertion of recent/more-session rows after the join row; state mutation remains in `waiting-room-kernel.ts`. Focused tests cover recent/more-session row insertion and selected recent-session key projection, preparing the main menu projection for extraction without duplicating row-key policy. `waiting-room-kernel.ts` is down to 1,766 lines.
- 2026-05-14: Cloud waiting-room current machine/kernel display labels moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-target-labels.ts`. The module owns bootstrap/loading, disconnected, connected machine id, daemon alias/id, and kernel ref fallback label policy shared by the footer and main waiting-room menu. Focused tests cover bootstrap loading, disconnected labels, connected target identity, and kernel fallback order; `waiting-room-kernel.ts` is down to 1,737 lines.
- 2026-05-14: Cloud waiting-room main menu projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-menu.ts`. The module owns row view types, runtime/config/session/target row projection, redacted/editor/session-column value-kind policy, create-alias and workspace-editor row metadata, recent-session rows, and hidden-session count rows. React now imports menu view types from the projection module while the fallback HTML renderer still consumes the same view model from `waiting-room-kernel.ts`. Focused tests cover redaction, create alias rows, workspace editor rows, recent-session session-column rows, more-session rows, working session rows, session alias projection, and target labels; `waiting-room-kernel.ts` is down to 1,584 lines.
- 2026-05-14: Cloud waiting-room session lifecycle decision policy moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-lifecycle.ts`. The module owns archive/delete target projection for selected picker sessions, selected recent sessions, join-row all-session actions, explicit session ids, and empty/error states. `waiting-room-kernel.ts` re-exports the decision helpers for compatibility while `removeWaitingRoomSessions` intentionally remains in the kernel until inventory normalization/state mutation is separated. Focused tests cover picker, recent, join-row, explicit-session, and error decisions; `waiting-room-kernel.ts` is down to 1,550 lines.
- 2026-05-14: Cloud waiting-room footer status projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-footer.ts`. The module owns session lifecycle hints, disconnected-kernel lifecycle fallback text, more-session hints, duplicate websocket status suppression, and plain footer fallback. `waitingRoomKernelCanLoadRuntimeData` was narrowed to the target/connection fields it actually needs so footer projection can depend on a small runtime-status boundary. Focused tests cover actionable session hints, disconnected lifecycle states, more-session text, websocket suppression, and default footer text; `waiting-room-kernel.ts` is down to 1,533 lines.
- 2026-05-14: Cloud waiting-room expansion state transitions moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-expansion-state.ts`. The module owns explicit sidebar session expand/collapse state, picker session detail toggles, and sidebar/picker agent/workflow detail toggles, including the shared idempotent list-set helpers. `waiting-room-kernel.ts` re-exports the state transition helpers for compatibility, and focused tests cover expanded/collapsed id bookkeeping, duplicate prevention, picker toggles, and surface-scoped item toggles; `waiting-room-kernel.ts` is down to 1,479 lines.
- 2026-05-14: Cloud waiting-room launch payload projection moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-launch-config.ts`. The module owns trimming alias/workspace values, terminal-facing model id normalization, provider/mode/permission/variant projection, and selected worktree fallback for session launch requests. `waiting-room-kernel.ts` keeps the compatibility export, and focused tests cover normalized launch values plus create-worktree fallback; `waiting-room-kernel.ts` is down to 1,456 lines.
- 2026-05-14: Cloud waiting-room cached inventory serialization moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-inventory-cache.ts`. The module owns the stable browser cache payload, intentionally stripping snapshot freshness metadata while preserving schema version, sessions, and launch target. `waiting-room-kernel.ts` keeps the compatibility export, and focused tests cover freshness omission, null inventory, and absent launch-target fallback; `waiting-room-kernel.ts` is down to 1,446 lines.
- 2026-05-14: OSS TypeScript kernel subscription state policy moved from `packages/kernel-client/src/ipc.ts` into `packages/kernel-client/src/kernel-subscriptions.ts`. The module owns session vs waiting-room subscription identity, resume cursor selection, waiting-room sentinel ids, relay subscription scope projection, and local websocket subscribe control envelopes. `LocalIpcClient` keeps socket writes, relay encryption, heartbeats, and reconnect orchestration while delegating subscription state decisions. Focused tests cover matching-session resume, new-session reset, waiting-room sentinel/scope, and subscribe envelope shape; `ipc.ts` is down to 789 lines, and both `pnpm --filter @arroba/kernel-client run test` and `pnpm --filter @arroba/cli run test` pass.
- 2026-05-14: Cloud waiting-room fallback main-menu HTML rendering moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-menu-renderer.ts`. The module owns HTML rendering for the already-extracted `WaitingRoomMenuView`, including redacted rows, create-alias editors, workspace editors, session aliases, session-column rows, row metadata, and working markers. `waiting-room-kernel.ts` now only chooses whether to hand React a menu view or call the fallback renderer. Focused tests cover redacted rows, inline editors, and session rows, and `pnpm --filter @arroba-cloud/web run test` passes with 261 tests; `waiting-room-kernel.ts` is down to 1,333 lines.
- 2026-05-14: Cloud waiting-room stale render helpers left behind by the menu renderer extraction were deleted from `ui/waiting-room-kernel.ts`. Removed helpers had no active call sites for create-session alias value rendering, workspace value rendering, sidebar workspace value rendering, and generic key/value rendering. The full web suite still passes with 261 tests, and `waiting-room-kernel.ts` is down to 1,278 lines.
- 2026-05-14: Cloud waiting-room state normalization moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-state-normalizer.ts`. The module owns provider/model/variant fallback, launch workspace defaults, workspace draft preservation, selected worktree fallback, recent/session-picker index clamping, session-picker closing when sessions disappear, and option-picker clamping/closing. Focused tests cover catalog fallback, launch workspace/worktree defaults, active workspace drafts, session/picker clamping, and empty-session cleanup; `pnpm --filter @arroba-cloud/web run test` passes with 265 tests, and `waiting-room-kernel.ts` is down to 1,216 lines.
- 2026-05-14: Cloud waiting-room session removal after archive/delete moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-lifecycle.ts`, now that state normalization is a separate dependency. The session lifecycle module owns archive/delete target decisions plus local inventory removal and normalized selection repair after successful lifecycle operations. Focused tests cover removal, selected recent/picker index repair, open picker preservation when sessions remain, and no-op behavior without inventory or ids; `pnpm --filter @arroba-cloud/web run test` passes with 267 tests, and `waiting-room-kernel.ts` is down to 1,200 lines.
- 2026-05-14: Cloud waiting-room focus navigation state moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-navigation.ts`, beside the existing row-key projection. The navigation module now owns row-order movement, row-index focus, field focus, recent-session row focus projection, wraparound, and workspace edit-state clearing when focus leaves the workspace row. Focused tests cover recent row selection, wraparound movement, and workspace edit preservation/clearing; `pnpm --filter @arroba-cloud/web run test` passes with 270 tests, and `waiting-room-kernel.ts` is down to 1,158 lines.
- 2026-05-14: Cloud waiting-room workspace/worktree state transitions moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-workspace-state.ts`. The module owns workspace edit open/close, draft changes, directory search loading/suggestions/errors, suggestion movement, workspace commit side effects, worktree loading/error state, and worktree result normalization while delegating final consistency to the state normalizer. Focused tests cover editing cleanup, search states, suggestion wrap/clamping, workspace commit, worktree loading preservation, normalized worktree records, and worktree errors; `pnpm --filter @arroba-cloud/web run test` passes with 275 tests, and `waiting-room-kernel.ts` is down to 1,026 lines.
- 2026-05-14: Cloud waiting-room session picker state transitions moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-picker-state.ts`. The module owns picker open/close, active-session availability gating, selected picker index movement/focus, wraparound, clamping, and closing any option picker when the session picker opens. Focused tests cover empty inventories, ended-session filtering through the session boundary, close behavior, movement wraparound, no-op empty moves, and focus clamping; `pnpm --filter @arroba-cloud/web run test` passes with 280 tests, and `waiting-room-kernel.ts` is down to 996 lines.
- 2026-05-14: Cloud waiting-room option picker state transitions moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-option-picker-state.ts`, leaving `ui/waiting-room-option-picker.ts` focused on option projection/rendering. The new state module owns picker open/close, option-picker/session-picker exclusivity, workspace edit cleanup on open, selected option index movement/focus, selected-value commits, provider/model/variant normalization, mode/permission validation, and invalid-choice close behavior. Focused tests cover runtime-unavailable opens, exclusivity cleanup, movement wraparound, no-open no-ops, focus clamping, selected commits, and invalid choices; `pnpm --filter @arroba-cloud/web run test` passes with 288 tests, and `waiting-room-kernel.ts` is down to 872 lines.
- 2026-05-14: Cloud waiting-room focused value cycling moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-focused-value-state.ts`. The module owns keyboard cycling for create-session fields, runtime-availability gating, provider/model normalization, variant/mode/permission wraparound, and worktree option cycling. Focused tests cover unavailable runtime no-ops, provider normalization, model and variant cycling, mode/permission cycling, and worktree wraparound; `pnpm --filter @arroba-cloud/web run test` passes with 293 tests, and `waiting-room-kernel.ts` is down to 785 lines.
- 2026-05-14: Cloud waiting-room fallback session-tree rendering moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-session-tree-renderer.ts`. The module owns sidebar session-tree HTML, session-picker HTML, shared agent/workflow detail-tree rendering, inline activity meters, session/agent alias controls, and sidebar agent workspace editor markup; `waiting-room-kernel.ts` now only coordinates state, footer/menu rendering, and picker renderer delegation. Focused tests cover sidebar tree rendering and picker open/closed fallback rendering; `pnpm --filter @arroba-cloud/web run test` passes with 295 tests, and `waiting-room-kernel.ts` is down to 277 lines.
- 2026-05-14: Cloud waiting-room base state ownership moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-kernel-state.ts`. The module owns the initial waiting-room state factory, control-plane data update patch semantics, explicit null clearing, and normalization after kernel/catalog/inventory refreshes. `waiting-room-kernel.ts` keeps compatibility exports while focused tests cover base state shape, omitted-field preservation, null inventory clearing, and normalized runtime selections; `pnpm --filter @arroba-cloud/web run test` passes with 298 tests, and `waiting-room-kernel.ts` is down to 188 lines.
- 2026-05-14: Cloud waiting-room launch-state finalization moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-launch-state.ts`. The module owns preparing state for session launch by trimming active alias drafts, closing alias editing, committing active workspace drafts, clearing workspace suggestions, and staging worktree reload state before launch payload projection. Focused tests cover alias finalization and workspace draft commit behavior; `pnpm --filter @arroba-cloud/web run test` passes with 300 tests, and `waiting-room-kernel.ts` is down to 172 lines.
- 2026-05-14: Cloud waiting-room render coordination moved from `ui/waiting-room-kernel.ts` into `ui/waiting-room-renderer.ts`. The renderer module owns DOM sink coordination for status/footer text, React menu handoff, fallback menu HTML, and session/option picker fallback layer composition. `waiting-room-kernel.ts` is now a compatibility barrel over focused waiting-room modules; focused renderer tests cover fallback HTML sinks, React `setMenu` handoff, and independent picker suppression; `pnpm --filter @arroba-cloud/web run test` passes with 303 tests, and `waiting-room-kernel.ts` is down to 139 lines.
- 2026-05-14: Cloud terminal freeform agent defaults moved from `apps/web/src/client.ts` into `terminal/freeform-agent-defaults.ts`. The module owns fallback provider/model lists, variant/mode/permission option constants, and deterministic default agent alias generation used by runtime config, spawn-agent dialogs, and live-agent templates. Focused tests cover the terminal defaults and alias generation; `pnpm --filter @arroba-cloud/web run test` passes with 305 tests.
- 2026-05-14: Cloud terminal route policy moved from `apps/web/src/client.ts` into `terminal/route-policy.ts`. The module owns terminal shell view projection, freeform route detection, kernel waiting-room hydration route detection, terminal shell route checks, and idle-detached `/terminal` redirect policy. Focused tests cover titles/nav, route classification/hydration, and redirect gating; the stale-while-revalidate source test now inspects the new hydration boundary, `pnpm --filter @arroba-cloud/web run test` passes with 308 tests, and `client.ts` is down to 17,709 lines.
- 2026-05-14: Cloud terminal app-sidebar policy moved from `apps/web/src/client.ts` into `terminal/app-sidebar-state.ts`. The module owns sidebar width storage/clamping, waiting-room sidebar label/date projection, recommended width sizing, placeholder view projection, and app-sidebar context validation while `client.ts` keeps DOM mounting and resize event wiring. Focused tests cover width policy, runtime session label/date fallbacks, deterministic recommended sizing, and context placeholders; `pnpm --filter @arroba-cloud/web run test` passes with 312 tests, and `client.ts` is down to 17,644 lines.
- 2026-05-14: Cloud workspace file-viewer code tokenization moved from `apps/web/src/client.ts` into `terminal/workspace-code-tokens.ts`. The module owns language-specific line tokenization, keyword tables, JSON/Markdown special cases, blank-line preservation, and regex escaping for the workspace file viewer while `client.ts` only builds the viewer model. Focused tests cover TypeScript keywords/comments/numbers, current JSON string/boolean behavior, Markdown heading/inline spans, and blank-line tokens; `pnpm --filter @arroba-cloud/web run test` passes with 316 tests, and `client.ts` is down to 17,535 lines.
- 2026-05-14: OSS runtime user-config mutation policy moved from `apps/kernel/src/runtime/router.rs` into `runtime/user_config_policy.rs`. The module owns mutation value typing, provider reload outcome summaries, daemon-restart path classification, and unwired-config path classification while `CommandRouter` keeps request execution. Focused Rust tests cover reload summaries plus restart/unwired path decisions; `cargo test --manifest-path apps/kernel/Cargo.toml --lib runtime::user_config_policy`, `cargo test --manifest-path apps/kernel/Cargo.toml --lib` (664 tests), and `cargo test --manifest-path apps/kernel/Cargo.toml --test kernel_websocket_integration -- --test-threads=1` (15 tests) pass. The broad all-target kernel run hit parallel websocket connection-refused setup failures before the serial integration rerun passed; `runtime/router.rs` is down to 15,755 lines.
- 2026-05-14: Cloud workspace file-viewer projection moved from `apps/web/src/client.ts` into `terminal/workspace-file-viewer.ts`. The module owns file language detection, icon labels, file-size formatting, file-viewer status metadata, binary/loading/error body projection, and code-line projection via the extracted tokenization module while `client.ts` passes current workspace state into the projection. Focused tests cover language/icon fallbacks, status/size metadata, loading/error/binary/code body states, and tokenized code rows; `pnpm --filter @arroba-cloud/web run test` passes with 319 tests, and `client.ts` is down to 17,410 lines.
- 2026-05-14: Cloud workspace git projection moved from `apps/web/src/client.ts` into `terminal/workspace-git-projection.ts`. The module owns workspace change row projection, status key/label normalization, change titles, primary git action titles/icons/disabled policy, and commit-message row sizing from sidebar width while `client.ts` keeps workspace orchestration and DOM syncing. Focused tests cover path context, status labels, action policy, and commit textarea sizing; `pnpm --filter @arroba-cloud/web run test` passes with 323 tests, and `client.ts` is down to 17,326 lines.
- 2026-05-14: Cloud workspace worktree projection moved from `apps/web/src/client.ts` into `terminal/workspace-worktree-projection.ts`. The module owns worktree option ids, label fallbacks, selected-worktree lookup/path fallback, picker index selection, main-worktree detection, and repo-label fallback projection while `client.ts` passes current workspace/waiting-room state into the pure helpers. Focused tests cover option construction, picker selection, main-worktree policy, and launch-target/session repo labels; `pnpm --filter @arroba-cloud/web run test` passes with 327 tests, and `client.ts` is down to 17,255 lines.
- 2026-05-14: Cloud workspace repo-file tree projection moved from `apps/web/src/client.ts` into `terminal/workspace-repo-files-projection.ts`. The module owns repo-files empty/loading/error body states, nested directory/file row projection, expanded-directory recursion, loading placeholders, and truncated-row labels while `client.ts` only delegates the current repo-file state. Focused tests cover unavailable/empty states, nested expanded rows, loading placeholders, and truncated rows; `pnpm --filter @arroba-cloud/web run test` passes with 330 tests, and `client.ts` is down to 17,160 lines.

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
- Refactor progress docs are committed and pushed as their own slice instead of being treated as local dirty work to preserve.
