# Cross-Repo Architecture Boundary Refactor Plan

## Scope

Refactor `arroba` and `arroba-cloud` together while preserving runtime compatibility.

- `arroba`: kernel, relay, CLI/shell clients, provider adapters, shared protocol/client code.
- `arroba-cloud`: hosted auth/control plane, relay token issuance, browser bootstrap, waiting room, browser terminal UI.
- Excluded: iOS. It should follow the stabilized protocol and boundaries after this refactor.

Cloud is auth/control-plane/bootstrap only. The kernel owns runtime sessions, agents, provider runs, workspaces, history, workflows, terminal events, and state transitions. The relay remains opaque transport.

## Current Checkpoint

2026-05-14:

- Cloud API boundary split is in place: `server.ts` is route composition; `cloud-api-service.ts` delegates to focused use cases; dependency construction lives in `cloud-api-service-dependencies.ts`; route/helper/contract files are domain-owned.
- Cloud web responsibility modules now cover browser kernel transport, waiting-room state/projection/rendering/cache persistence/connection display/refresh scheduling/refresh controller/kernel event policy/kernel-directory refresh, app sidebar/context/resize/workflow/history/prompt/output/workspace/capabilities, terminal lifecycle/transport/target/launch/provider profile, freeform policy, and session projection fingerprinting. `client.ts` is still the main coordinator and is 9,893 lines.
- OSS modules now cover router composition/actors, command metadata/admission/dispatch, relay/cloud/session bridges, Cloud API HTTP transport/session collaboration, projections, runtime state views, prompt/workflow/provider/session/agent/profile/capability/attachment ownership, prompt activity/transcript fan-out, local prompt admission/dispatch/cancellation, prompt queue advancement, prompt skill context, provider focus sync, provider launch request building/failure recovery/liveness/substitute handling, provider-switch context handoff reconstruction, runtime notice fan-out, managed I/O argument normalization/workspace identity/parser/patch planning/patch application/whole-file application/file state/edit result/reservation policy/remote state conversion/remote patch/remote whole-file application, runtime interactions, remote prompt submission/lifecycle/worker relay submission, workflow endpoints/settings/node topology/edge topology/turn prompts/completion evidence/prompt failure handling/blocked-claim retry/run request execution/launch scheduling admin/runtime console tools/runtime output tools, tool dispatch, response refresh/redaction, agent lane queueing/command execution, session actor agent/terminal adapters, session membership scope policy, and responsibility-owned tests. Next targeted OSS hotspots: `runtime/state/workflow_publication_owned_state.rs` 364 lines, `runtime/state/workflow_launch_owned_state.rs` 350 lines, `runtime/state/provider_reload.rs` 342 lines, and `runtime/state/workflow_admin.rs` 303 lines. Reduced coordinators: `runtime/cloud_api_client.rs` 169 lines, `runtime/agent_actor.rs` 295 lines, `runtime/router.rs` 207 lines, `runtime/session_actor.rs` 136 lines, `runtime/session_actor/store.rs` 255 lines, `runtime/session_membership.rs` 187 lines, `runtime/state/provider.rs` 264 lines, `runtime/state/provider_launch_runtime.rs` 166 lines, `runtime/state/provider_launch_failure_runtime.rs` 213 lines, `runtime/state/provider_launch_owned_state.rs` 211 lines, `runtime/state/context_handoff.rs` 203 lines, `runtime/state/context_handoff/builder.rs` 302 lines, `runtime/state/agent_config_owned_state.rs` 203 lines, `runtime/state/agent_profile_owned_state.rs` 171 lines, `runtime/state/capability_owned_state.rs` 108 lines, `runtime/state/prompt_dispatch.rs` 237 lines, `runtime/state/prompt_activity_owned_state.rs` 235 lines, `runtime/state/provider_prompt_settlement_runtime.rs` 296 lines, `runtime/state/remote_prompt_dispatch_runtime.rs` 160 lines, `runtime/state/workflow_completion_owned_state.rs` 258 lines, `runtime/state/workflow_completion_snapshot_owned_state.rs` 122 lines, `runtime/state/workflow_prompt_failure_owned_state.rs` 95 lines, `runtime/state/workflow_admin.rs` 303 lines, `runtime/state/workflow_tool.rs` 259 lines, `runtime/state/workflow_console_tool.rs` 112 lines, `runtime/state/workflow_output_tool.rs` 192 lines, `runtime/state/workflow_request_runtime_state.rs` 291 lines, `runtime/state/workflow_run_request_runtime_state.rs` 191 lines, `runtime/state/workflow_scheduling_owned_state.rs` 122 lines, `runtime/state/workflow_dispatch.rs` 228 lines, `runtime/state/workflow_definition_owned_state.rs` 125 lines, `runtime/state/workflow_node_owned_state.rs` 279 lines, `runtime/state/prompt.rs` 192 lines, `runtime/state/prompt_cancellation_owned_state.rs` 253 lines, `runtime/state/managed_io.rs` 161 lines, `runtime/state/managed_io/patch.rs` 185 lines, `runtime/state/managed_io/remote.rs` 251 lines, `runtime/state/provider_output_runtime.rs` 230 lines, `runtime/state/mod.rs` 273 lines.
- Latest verified batch: OSS workflow completion evidence lookup moved into `workflow_completion_snapshot_owned_state.rs`, and workflow prompt failure/cancellation handling moved into `workflow_prompt_failure_owned_state.rs`.
- Latest gates for owned files: `cargo test --manifest-path apps/kernel/Cargo.toml workflow --lib -- --test-threads=1`, `cargo test --manifest-path apps/kernel/Cargo.toml provider_prompt --lib -- --test-threads=1` (0 matching tests), rustfmt check on owned files, and diff check pass. Cloud API typecheck and focused API service tests pass; full Cloud API test is currently blocked by dirty Cloud web client changes expecting `terminal-runtime.js`. Full kernel `cargo fmt --check` is blocked by unrelated existing formatting drift outside these slices.

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
