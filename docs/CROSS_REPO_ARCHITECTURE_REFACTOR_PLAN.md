# Cross-Repo Architecture Boundary Refactor Plan

## Scope

Refactor `arroba` and `arroba-cloud` together while preserving runtime compatibility.

- `arroba`: kernel, relay, CLI/shell clients, provider adapters, shared protocol/client code.
- `arroba-cloud`: hosted auth/control plane, relay token issuance, browser bootstrap, waiting room, browser terminal UI.
- Excluded: iOS. It should follow the stabilized protocol and boundaries after this refactor.

Cloud is auth/control-plane/bootstrap only. The kernel owns runtime sessions, agents, provider runs, workspaces, history, workflows, terminal events, and state transitions. The relay remains opaque transport.

## Current Checkpoint

2026-05-15:

- Cloud API boundary split is in place: `server.ts` is route composition; `cloud-api-service.ts` delegates to focused use cases; dependency construction lives in `cloud-api-service-dependencies.ts`; route/helper/contract files are domain-owned.
- Cloud web responsibility modules now cover browser kernel transport, waiting-room state/projection/rendering/cache persistence/connection display/refresh scheduling/refresh controller/kernel event policy/kernel-directory refresh, app sidebar/context/resize/workflow/history/prompt/output/workspace/capabilities, terminal lifecycle/transport/target/launch/provider profile, freeform policy, and session projection fingerprinting. `client.ts` is still the main coordinator and is 9,893 lines.
- OSS runtime now has responsibility-owned boundaries for router composition, runtime state views, prompt/provider/workflow/session/capability ownership, managed I/O, projection policy, relay/cloud bridges, and session actors. `runtime/capability_executor.rs` is 295 lines after moving context, health/admission, and transferred-file artifact recording into submodules.
- Remote managed-I/O dispatch now separates composition, outbound leased-worker forwarding, home-kernel admission/routing, forwarded reads, forwarded text edit/write, and forwarded patch/delete/move mutations. History requests now separate session transcript, prompt-input history, archive query/search, and semantic search mapping. Workspace repo files now separate listing projection, file content loading, and shared timing.
- Latest verified batch: BrowserKernelClient transport, browser kernel request builders, history kernel bridge, and CLI Cloud command/worktree placement policies are responsibility-owned modules; Cloud web and CLI tests pass.
- Latest verified batch: CLI provider auth/process command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI model/variant/view selection is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI user config command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI remote machine command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI MCP/skill capability command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI slice command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workspace/worktree command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent command handling and focus cycling are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command coordinator, automation flow, overlays, interaction strips, status badges, and prompt chrome rendering are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI split-pane footer rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI status indicator chrome rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt/footer chrome summary rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workspace shell submission is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent activity/busy latch and focused busy derivation policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI terminal record agent routing policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment pending-state policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider-native command submission policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow endpoint prompt admission is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt submission projection/status policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI interaction choice selection/reply policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI focused interaction keyboard policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt history and prompt-turn navigation policies are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room key navigation/lifecycle policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-browser key policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-browser list/index policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-browser controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI dialog overlay state priority is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment UI state controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment intake/controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI clipboard selection controller is responsibility-owned; CLI tests pass.
- Latest verified batch: Cloud API route-adapter guardrail prevents direct service/repository imports; API build/tests pass.
- Latest verified batch: Cloud API relay facade owns token, target, browser bootstrap, and kernel presence wiring; API build/tests pass.
- Latest verified batch: Cloud API pairing facade owns pairing, revocation, and machine runtime profile wiring; API build/tests pass.
- Latest verified batch: Cloud API browser session facade owns browser session and device-login wiring; API build/tests pass.
- Latest verified batch: Cloud API session-invite facade owns collaboration invite wiring; API build/tests pass.
- Latest verified batch: Cloud API service is thin composition; account, admin, billing, dashboard, and managed-history wiring live in domain facades; API build/tests pass.
- Latest verified batch: Cloud API service composition guardrail prevents direct domain use-case imports/regrowth; API build/tests pass.
- Latest verified batch: CLI command-center selection/submission policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI dialog overlay focus capture/restore policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt content-change/drop policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt draft persistence scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI turn-completion scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI terminal output record batching is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI shared prompt input history refresh scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI footer flash timing is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session chrome update scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI response-pane focus repaint scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI connection health watchdog is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prepended-history scroll restoration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent focus transition tracking is responsibility-owned; CLI tests pass.
- Latest verified batch: Cloud API route schemas are domain-owned with a generic-helper guardrail; API build/tests pass.
- Latest verified batch: CLI older-transcript history loading is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-history hydration stale-result guard is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript-history auto-load triggers are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript render deferral is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt cancellation in-flight handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider recovery relaunch/reapply flow is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel event subscription scope tracking is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel restart reattachment/backoff recovery is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI attached-kernel resync/catch-up flow is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI exit cleanup and force-quit retry handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room transition cleanup is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI terminal restore/process-exit teardown is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt input history append/record/sequence tracking is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI submitted-prompt UI reset/restore is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-history keyboard navigation is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt content-change/drop side effects are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment token highlighting is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt text snapshot/mutation state is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center state and keyboard controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room control activation policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room lifecycle confirmation state is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider/model/variant selection controller is responsibility-owned; CLI tests pass.
- Latest gates for owned files: kernel-client tests, Cloud API build/API tests, focused router test, file-level rustfmt, and scoped diff checks pass. Cloud web and full-kernel gates remain pending where they touch dirty unrelated slices.

## Responsibility Rule

Do not split large files by line range. A new module must own a named responsibility with a stable dependency direction.

Allowed:

- Move complete responsibilities such as command admission, session mutation, relay bootstrap, event replay, prompt lifecycle, browser session state, waiting-room projection, or external adapter I/O.
- Extract pure state/projection/request policy with tests before moving side effects.
- Keep compatibility barrels only to preserve imports during migration.

Disallowed:

- Bucket files such as `client-part-1.ts`, `router-helpers.rs`, or `server-utils.ts`.
- Moving private helpers without changing ownership.
- Modules that require broad unrelated state access.
- Mixing render, state mutation, network I/O, and policy in one new module.

## Protocol Rule

No protocol shape changes are intended. If `LocalDaemonRequest`, `LocalDaemonResponse`, relay terminal events, browser/kernel transport semantics, or serialized client protocol shapes change:

1. Bump the shared local daemon protocol version.
2. Update protocol snapshot/hash tests.
3. Update client minimum protocol versions only when needed.
4. Add a focused drill for the changed behavior.

## Workstreams

### OSS Runtime

- Keep shrinking `CommandRouter` into responsibility executors:
  - session lifecycle/membership/focus
  - cloud relay login/token/session invite calls
  - provider auth/catalog/process controls
  - workspace/git/worktree/file requests
  - relay status/remote inventory projections
  - semantic/agent utilities
- Add a kernel composition boundary that owns runtime state, stores, projections, actors, transport health, provider lanes, workspace coordination, and schedulers.
- Remove production `app.lock()` command-path dependencies outside bootstrap/composition. If a slice cannot remove one, document the blocker and avoid new call sites.
- Keep request/response behavior and wire shapes unchanged.

### Shared Protocol And Clients

- Keep `apps/kernel/src/local/api/types.rs` as the wire source of truth.
- Keep browser-safe request/event/protocol helpers separate from Node-only socket/crypto code in `packages/kernel-client`.
- CLI and shell remain clients, not runtime authorities.
- Split CLI composition/state/event handling after kernel-client and runtime boundaries are stable.

### Cloud API

- Keep `apps/api/src/server.ts` as Fastify composition only.
- Keep route modules domain-owned: browser relay bootstrap, browser session, relay, admin, billing, device login, pairing, managed history, account control, session invites.
- Keep `CloudApiService.bootstrapBrowserRelayKernel(input)` as the browser bootstrap use case.
- Keep `contracts.ts` as a compatibility barrel over domain contract files until imports are migrated.
- Preserve `/browser/relay-kernel/bootstrap`, `/dashboard`, `/relay/token`, and browser terminal route compatibility.

### Cloud Web

- Turn `apps/web/src/client.ts` into app bootstrap/coordinator only: route mount, dependency wiring, global event registration, and app start.
- Continue extracting by responsibility:
  - terminal app/container wiring
  - waiting-room controller and background refresh orchestration
  - terminal session lifecycle/connect/reattach
  - freeform agent config/dialog controllers
  - prompt/history/workspace/capabilities controllers
  - render/projection modules or React mounts for HTML/view ownership
- Controllers should not build large HTML strings. Rendering belongs in render modules or React mount components.
- Waiting-room refresh must not overwrite active transcript, focus, prompt draft, selected session, or local reconnect state.

### Naming, Docs, Cleanup

- Active browser runtime storage keys use `arroba:terminal:*` with one-time legacy `arroba:web-cli:*` read fallback.
- Active drills use `terminal-*` or `browser-relay-kernel-*` names.
- Active docs/code/scripts should not reference live `/web-cli` routes except archived historical notes with explicit archive wording.
- Keep this plan concise. Add one checkpoint line per coherent verified batch, not one entry per tiny helper.
- Commit and push verified docs/code batches together; do not leave plan/docs dirty as assumed user work.

## Execution Order

1. Maintain architecture guardrails and line-budget checks for `client.ts`, `server.ts`, `waiting-room-kernel.ts`, and `runtime/router.rs`.
2. Finish Cloud web coordinator extraction around real responsibilities, starting with remaining terminal session/freeform/waiting-room orchestration.
3. Continue OSS router extraction around runtime responsibilities, then introduce the kernel composition boundary.
4. Split remaining CLI composition/state/controller code after shared protocol/runtime seams are stable.
5. Remove stale compatibility barrels/helpers once imports have moved.
6. Run cross-repo smoke/drill gates before resuming feature work.

## Test Gates

Per OSS slice:

- `cargo fmt --manifest-path apps/kernel/Cargo.toml`
- `cargo test --manifest-path apps/kernel/Cargo.toml --lib -- --test-threads=1`
- `pnpm --filter @arroba/kernel-client run test` when shared TypeScript client code changes
- `pnpm --filter @arroba/cli run test` or shell tests when client code changes

Per Cloud slice:

- `pnpm --filter @arroba-cloud/api test` when API changes
- `pnpm --filter @arroba-cloud/web test` when web changes
- `pnpm -r --if-present lint`
- `git diff --check`

Architecture/drill gates:

- browser relay kernel prompt flow
- stale relay target denial
- managed relay smoke
- local/remote freeform and workflow relay drills
- reconnect/replay-gap/session snapshot recovery
- waiting-room refresh preserves active terminal state
- staging retail strict smoke for deployment-sensitive changes

## Done Criteria

- Cloud has no runtime terminal proxy behavior and remains bootstrap/control plane only.
- Relay remains opaque transport.
- Kernel-owned runtime state is not forked in Cloud or clients.
- `client.ts`, `server.ts`, `waiting-room-kernel.ts`, and `runtime/router.rs` are coordinators/composition files rather than domain owners.
- Protocol shapes are unchanged, or the protocol rule has been followed.
