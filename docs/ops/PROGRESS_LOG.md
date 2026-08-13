# Chariox Progress Log

Chronological notes to preserve execution context between contributors/agents.

## 2026-06-11

### Current-head local remote home-extension validation

- Revalidated the local self-hosted relay remote home-extension matrix on current OSS `main` HEAD `05a9bdee6`: `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs` passed. The matrix ran `local-single` and `local-collab`, covering single-user and scoped-relay collaboration remote workers invoking home-owned script, MCP, and connector tools through the worker projection path, plus home-side revoke enforcement for stale projected tools.

### Current-head local slice lifecycle validation

- Revalidated local Docker slice lifecycle on current OSS `main` HEAD `66f4515ba`: `pnpm --filter @chariox/cli run slice:lifecycle-drill` passed. The run covered headed slice creation, worker-kernel discovery, noVNC endpoint readiness, Codex/OpenCode/Claude provider-auth summary extraction, Codex device-login startup inside the slice, provider-auth removal and alias slash-command UX, waiting-room idle slice deletion, independent provider accounts across slices, wrong-worktree rejection for sessions and agents, multi-session/multi-agent reuse of one slice, active-agent delete blocking, and cleanup of the drill-owned containers and artifact root.

### Current-head Hetzner remote home-extension validation

- Revalidated the actual Hetzner worker remote home-extension matrix on current OSS `main` HEAD `7d7e0c5` using isolated checkout `/tmp/chariox-remote-home-extension-head-1781135946-38328`. Built `chariox-kernel` and `chariox-relay` in that checkout, then ran `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --include-hetzner --only hetzner-single,hetzner-collab --continue-on-failure --hetzner-repo /tmp/chariox-remote-home-extension-head-1781135946-38328`; both scenarios passed. The run covered single-user and scoped-relay collaboration over the real remote worker, home-owned script/MCP/connector projection and execution on home, and stale projected-tool revoke enforcement. The isolated remote checkout and drill-owned remote roots/processes were removed afterward.

## 2026-06-10

### Current-head local remote home-extension validation

- Revalidated the local self-hosted relay remote home-extension matrix on current OSS `main` HEAD `87e560bf9`: `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --only local-single,local-collab` passed. The run covered single-user and collab remote workers invoking home-owned script, MCP, and connector tools through the relay, plus home-side revoke enforcement for stale projected tools.

### Current-head Workspace Live Sync scope validation

- Revalidated local Codex Workspace Live Sync permission gating on current OSS `main` HEAD `2e5b53f63`: `node apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs --only local-permission-codex` passed. The run proved synced-root writes still require Chariox Workspace Live Sync permission/tooling while a separate outside Git repo remains provider-native and writable.
- Revalidated same-host remote Codex Workspace Live Sync permission gating on current OSS `main` HEAD `24645c1c8`: `node apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs --include-remote --only remote-permission-codex` passed. The run covered local relay, home kernel, worker kernel, remote agent leasing, synced-root Workspace Live Sync permission, and provider-native writes to a separate outside Git repo.
- Revalidated same-host remote tracked Codex Workspace Live Sync fanout on current OSS `main` HEAD `3f0684451`: `node apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs --include-remote --only remote-tracked-codex` passed. The run covered relay/home/worker remote agent leasing, two target worktrees, bidirectional tracked fanout, `.charioxignore`, outside-turn exclusion, unchanged Git heads, conflict detection, resolver convergence, and final ready sync status.

## 2026-06-05

### Web terminal output identity hardening

- Hardened Chariox Cloud `main` commit `7d2d88e` so live `terminal_output` records no longer infer an agent from the selected or sole web terminal pane. Kernel/provider output without an explicit `agent_id` is skipped with diagnostics instead of being rendered into an arbitrary agent transcript.
- Tightened the workflow console selected-agent trace the same way: provider-run-only terminal logs remain visible as logs, but they are not projected as selected-agent transcript entries unless the output carries the matching agent identity.
- Revalidated in `/Users/miguel/chariox-cloud` with `pnpm --filter @chariox-cloud/web test -- output-pane-controller output-records terminal-reconnect-history workflow-console-model workflow-kernel-bridge`; the command built the web package and passed all 1632 web tests. `git diff --check` also passed.

### TUI terminal output identity hardening

- Hardened OSS TUI terminal output projection so kernel/provider `terminal_output` records no longer infer ownership from the current streaming agent, active prompt, processing agent, or focused agent. Records without explicit `agent_id` now count as daemon activity but do not mutate the visible transcript or split-agent panes.
- Tightened Codex native TUI projection to require the record `agent_id` to match the attached native agent before forwarding output into the provider TUI.
- Revalidated with `pnpm --filter @chariox/cli test -- terminal-record-agent-resolver kernel-event-controller native-tui-codex-kernel-output-projection`; the command linted, built, and passed 1214 CLI tests. `git diff --check` also passed.

### Remote home extension current-HEAD local validation

- Revalidated the local self-hosted relay remote home-extension matrix on current OSS `main` HEAD `6bea6275`: `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --only local-single` passed, and `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --only local-collab` passed. The runs covered home-owned script, MCP, and connector projection/execution from a remote worker, collaborator use of home grants, denial of collaborator authority over home grants, worker-local collision checks, and home-side revoke enforcement for stale projected tools.
- Revalidated the actual-Hetzner worker remote home-extension matrix on current OSS `main` HEAD `1eadd763` using an isolated temporary Hetzner checkout instead of the shared `/tmp/chariox-native-remote-validate` path. Built `chariox-kernel` and `chariox-relay` in `/tmp/chariox-remote-home-extension-head-1780612416-73064`, then ran `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --include-hetzner --only hetzner-single --hetzner-repo /tmp/chariox-remote-home-extension-head-1780612416-73064` and `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --include-hetzner --only hetzner-collab --hetzner-repo /tmp/chariox-remote-home-extension-head-1780612416-73064`; both passed. The temporary checkout was removed after validation.
- Revalidated hosted Cloud second-kernel remote home-extension staging on current OSS `main` HEAD `3c397142` with Cloud `main` HEAD `f27017f`: `/Users/miguel/chariox-cloud pnpm run smoke:hosted-oss-drill -- --second-kernel` passed for single-user, and `/Users/miguel/chariox-cloud pnpm run smoke:hosted-oss-drill -- --second-kernel --multi-user` passed for collab. The runs covered `https://chariox-cloud-staging.osc-fr1.scalingo.io`, hosted relay `wss://195.201.123.115.sslip.io`, second-kernel worker leasing, home-owned script/MCP/connector projection and execution, collaborator-owned remote agent checks, Cloud invite acceptance, and denial of collaborator grant/revoke/request authority. The collab run retried one transient third-client relay websocket timeout and then completed successfully.

## 2026-06-04

### Hosted relay and permission drill revalidation

- Fixed `live-remote-workspace-live-sync-permission-drill.mjs` to isolate the home/worker daemon `HOME` roots while intentionally preserving provider auth/state paths (`CODEX_HOME`, `OPENCODE_CONFIG_DIR`, and OpenCode `XDG_*` data/state/cache). This prevents the same-host remote permission drill from silently using the real Chariox config while still surfacing the real provider account state instead of a misleading missing-model error.
- Revalidated remote Workspace Live Sync permission parity on current OSS `main` HEAD `1e76dfe2`: `pnpm --filter @chariox/cli run workspace-live-sync:remote-permission-drill` passed locally, and the same drill with `--hetzner-worker` passed after fast-forwarding `/tmp/chariox-native-remote-validate` to `1e76dfe2` and rebuilding the remote kernel/relay with `/root/.cargo/bin/cargo`. Both runs proved synced-repo writes route through Workspace Live Sync while an outside repo remains provider-native and writable.
- Confirmed current OpenCode Zen provider state is blocked before Chariox behavior by `Insufficient balance`; after the drill env fix, `workspace-live-sync:opencode-remote-permission-drill` reaches that real provider account error instead of `Model not found`.
- Revalidated Scalingo hosted relay staging with `CHARIOX_CLOUD_DEV_AUTH_SECRET` loaded from Scalingo: base hosted relay, hosted second-kernel home-owned script/MCP/connector projection, hosted multi-user collaboration, hosted token rotation, and hosted remote CLI all passed against `https://chariox-cloud-staging.osc-fr1.scalingo.io` and `wss://195.201.123.115.sslip.io`.
- Fixed the hosted terminal-pairing drill after it exposed stale history API usage: `waitForHistoryText` now reads `GetSessionHistoryOutline` plus blob content instead of the removed flat `GetSessionHistory` request. The hosted terminal-pairing drill then passed with Codex `gpt-5.5` using the clean Hetzner validation checkout.
- Tightened hosted remote CLI drills so automation `/exit` must lead to a clean CLI process exit before the harness ends sessions or tears down SSH. This removes the previous cleanup race where a provider-backed terminal-pairing run could pass while the remote CLI later reported exit code `1`. Revalidated hosted remote CLI and hosted terminal pairing with Codex `gpt-5.5`; both now pass with local/remote CLI exit code `0`.
- Aligned TUI help and command-center copy for slice-backed agent spawning with the actual launch primitives: `/agent spawn` now advertises `--slice off|new|s` plus `--slice-display headless|headed`, and command-center text names off/new/new:headed/reuse explicitly. Revalidated with `pnpm --filter @chariox/cli test -- cli-options command-center agent-spawn-command-handlers` (1200 tests).

### Remote home extension local relay validation

- Revalidated the local self-hosted relay remote home-extension matrix. `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --only local-single` passed for a single user, and `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --only local-collab` passed for collab. Both runs proved that a remote worker without the local extension definition/credential can invoke home-owned script, MCP, and connector tools through the relay, and that home-side revoke enforcement blocks stale use.
- Fast-forwarded the Hetzner validation checkout `/tmp/chariox-native-remote-validate` to current OSS `main` HEAD `9e681bf0`, rebuilt the remote kernel and relay with the rustup stable toolchain, and revalidated the actual-Hetzner worker matrix. `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --include-hetzner --only hetzner-single` and `node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs --include-hetzner --only hetzner-collab` both passed, covering the same home-owned script/MCP/connector execution and revoke enforcement with the worker kernel on Hetzner.
- Revalidated hosted Cloud second-kernel staging with OSS `62226180` and Cloud `d20b396`: `CHARIOX_OSS_REPO=/Users/miguel/chariox pnpm run smoke:hosted-oss-drill -- --second-kernel` passed for single-user, and `CHARIOX_OSS_REPO=/Users/miguel/chariox pnpm run smoke:hosted-oss-drill -- --second-kernel --multi-user` passed for collab. The drills covered the staging API `https://chariox-cloud-staging.osc-fr1.scalingo.io`, hosted relay `wss://195.201.123.115.sslip.io`, second-kernel worker leasing, home-owned script/MCP/connector invocation, collaborator-owned remote agent checks, and denial of collaborator grant/revoke/request authority.

### Hetzner OpenCode Workspace Live Sync validation

- Attempted the actual-Hetzner OpenCode Zen Workspace Live Sync matrix with `node apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs --include-hetzner --include-opencode --only hetzner-managed-opencode,hetzner-tracked-opencode,hetzner-permission-opencode --continue-on-failure`. All three scenarios were blocked before Chariox Workspace Live Sync behavior by the OpenCode account response `Insufficient balance`.
- Hardened `live-workspace-live-sync-permission-drill.mjs` so remote wrapper runs can pass the authoritative home history directory to the child drill. The Hetzner permission rerun now fails fast with the provider error in ~25s instead of waiting for the full approval-interaction timeout.
- Classified OpenCode `Insufficient balance` / `Manage your billing` provider errors as substitutable resource-limit failures, so agents with configured substitutes can recover from that account state through the same automatic substitution path as quota, credit, rate-limit, and run-limit failures.

## 2026-06-02

### Slice audit recovery guidance

- Aligned slice recovery guidance around the durable `slice.audit` trail. Protocol docs, CLI health, and Cloud Settings tests now treat `/slice audit <slice>` as part of unhealthy-slice and provider-auth recovery instead of relying only on logs or login/import actions.

## 2026-06-01

### Local runtime debug bundle export

- Added `chariox-cli logs --bundle <dir>` to export filtered local structured logs plus a manifest for a session/provider-run support handoff. The command reuses the existing process-kind/component/session/provider-run/client/level filters and refuses `--follow` so bundles are finite artifacts.
- Updated logging docs and closed the M3 log-collection checklist item for local multi-process session/provider-run bundles.
- Added attached-TUI `/kernel debug-bundle [label]` so users can export a current-session local debug bundle from the same surface where remote/slice/session health is shown.
- Added kernel-owned `ExportDebugBundle { session_id, bundle_label, limit }` / `DebugBundleExported` protocol support so TUI, web, and remote clients can request the same session-scoped bundle without arbitrary remote output paths. Labels are sanitized and bundles are written under the kernel machine's debug-bundles root.

### Home extension invocation replay audit

- Added durable audit events for home-executed remote extension idempotent replays and duplicate non-idempotent rejection. Script/connector and MCP proxy paths now use the same audited invocation-admission helper, so silent replay/duplicate paths show up in `/extension audit` and web extension diagnostics.
- Revalidated focused home-extension invocation tests and the full remote authorization module.
- Extended TUI/shell and Cloud web extension audit renderers to show invocation id, provider tool-call id, attempt, and idempotency key, and to explain replayed idempotent calls as a no-retry condition.

### Slice health issue attribution

- Added daemon-health slice issue attribution for unhealthy slices and failed slice operations. `slice_lifecycle.issues` now carries slice id/name, status, last operation/status/error, sessions, agents, and worktree; CLI `/kernel health` and Cloud web Settings render the affected slice directly instead of only aggregate counts.
- Bumped the local daemon protocol to 90 and revalidated daemon health, protocol shapes/version conformance, kernel-client/CLI health formatting, Cloud web health projection, Settings rendering, OSS/Cloud builds, lint, and diff checks.

### Remote extension sync issue attribution

- Added daemon-health remote extension sync issue attribution for home-proxy remote agents whose manifest is missing, failed, stale, or pending revoke. `remote_extension_sync.issues` now names the session, agent, worker kernel/machine, lease, leased agent, active worker provider run, state, hash, error, worktree, and home-proxy grants; CLI `/kernel health` and Cloud web Settings render the affected agent directly.
- Bumped the local daemon protocol to 91 and revalidated daemon health, protocol shapes/version conformance, kernel-client/CLI health formatting, Cloud web health projection, Settings rendering, OSS/Cloud builds, lint, and diff checks.

### Workspace Live Sync identity issue attribution

- Added daemon-health Workspace Live Sync identity issue attribution for managed/tracked provider runs whose observed workspace identity changed. `workspace_live_sync.workspace_identity.issues` now names the provider run, root, generation, validity, baseline/current fingerprint, branch, head commit, and repo URL; CLI `/kernel health` and Cloud web Settings render the affected run directly.
- Bumped the local daemon protocol to 92 and revalidated Workspace Live Sync identity health, daemon health, protocol shapes/version conformance, kernel-client/CLI health formatting, Cloud web health projection, Settings rendering, OSS/Cloud builds, lint, and diff checks.

### Workspace Live Sync external change attribution

- Added daemon-health Workspace Live Sync external-change attribution for tracked artifacts changed outside Chariox after their last managed observation. `workspace_live_sync.external_changes.issues` now names the artifact key, provider run when still tracked, workspace fingerprint, root, and path; CLI `/kernel health` and Cloud web Settings render the affected file directly.
- Bumped the local daemon protocol to 93 and revalidated external-change health, daemon health, protocol shapes/version conformance, kernel-client/CLI health formatting, Cloud web health projection, Settings rendering, OSS/Cloud builds, lint, and diff checks.

### Remote execution placement issue attribution

- Added daemon-health remote execution attribution for malformed remote-agent bindings and actively working remote agents missing an active worker provider-run id. `remote_execution.issues` names the session, agent, worker kernel/machine, lease, leased agent, state, processing flag, worktree, and details; CLI `/kernel health` and Cloud web Settings render the affected remote/slice agent directly.
- Bumped the local daemon protocol to 94 and revalidated remote execution health, daemon health, protocol shapes/version conformance, kernel-client/CLI health formatting, Cloud web health projection, Settings rendering, OSS/Cloud builds, lint, and diff checks.

### Slice provider auth issue attribution

- Added daemon-health slice provider auth attribution for attached-agent slices with missing provider account summaries or `unknown`/`not_configured` provider auth. `slice_lifecycle.provider_auth_issues` now names slice, sessions, agents, worktree, provider, state, alias/identity, and details; CLI `/kernel health` and Cloud web Settings render the affected slice auth blocker directly.
- Bumped the local daemon protocol to 95 and revalidated slice provider auth health, daemon health, protocol shapes/version conformance, kernel-client/CLI health formatting, Cloud web health projection, Settings rendering, OSS/Cloud builds, lint, and diff checks.

### Provider catalog health surfacing

- Surfaced existing daemon-health provider catalog cache state in CLI `/kernel health` and Cloud web Settings. Stale provider/model metadata is now counted as a health issue with cache age/TTL and a direct refresh/reselect next action before users launch new sessions or agents.

### Agent home-kernel placement surfacing

- Added session home-kernel and owner context to CLI/web-CLI agent inspection, and added a home-kernel badge to Cloud web freeform agent panes. Agent surfaces now distinguish session authority from worker/slice execution, worktree, provider run, extension grants, and remote extension manifest state.

### Remote extension recovery actions

- Tightened TUI/shell and Cloud web recovery text for remote extension manifest failures so diagnostics now name both `/extension sync-status <agent>` and `/machine kernels <worker-machine>` before retrying sync. Web slice provider-auth health now names `/slice doctor <slice>` directly before login/import actions.

### Workspace Live Sync outside-root writes

- Tightened the Codex managed Workspace Live Sync launch policy on macOS so Codex uses full filesystem mode while Chariox's macOS seatbelt fence denies writes only under the selected live-sync roots. This keeps managed sync authoritative for the synced repo/worktree while allowing provider-native edits in other repositories outside the synced roots. Non-macOS Codex managed runs remain read-only until an equivalent provider-independent write fence exists there.
- Fixed OpenCode managed Workspace Live Sync request shaping so native edit/write/apply_patch stay disabled while `bash` is explicitly enabled when the Chariox macOS write fence is active. This lets OpenCode modify repositories outside the synced root without allowing direct native writes inside the synced root.
- Extended the local/remote Workspace Live Sync permission drill to initialize the synced workspace as its own Git repo, create a separate Git repo outside it, fail fast on provider launch/provider errors, and require the managed agent to write the outside repo directly. Codex and OpenCode Zen passed this drill locally on 2026-06-01.

## 2026-05-31

### Slice operation diagnostics

- Added protocol-visible slice operation diagnostics: `last_operation`, `last_operation_status`, `last_error`, and `last_operation_at_ms`. Start/stop/delete and restart reconciliation now update these fields while keeping `status` as the lifecycle authority and `slice.audit`/logs as the detailed trail.
- Bumped the local daemon protocol to 79 and updated Rust, kernel-client, CLI, and web slice record types. `/slice status`, `/slice doctor`, waiting-room slice rows, and the web Slices panel now surface failed/last operation context.
- Focused validation passed: kernel formatting, `cargo test --manifest-path apps/kernel/Cargo.toml slice -- --nocapture`, client protocol conformance, kernel-client shell slice tests, CLI slice/waiting-room tests, and Cloud web slice panel/projection tests.
- Live local Docker validation passed after warming the image build cache with `pnpm --filter @chariox/cli run slice:lifecycle-drill`. The first cold run timed out while compiling the slice image before the final Docker tag existed; the rerun completed start, worker discovery, screen endpoint, provider auth import/remove/login, independent account summaries, wrong-worktree rejection, multi-session/multi-agent reuse, and cleanup.
- Extended OSS and Cloud browser kernel request timeouts to 10 minutes so first-run slice provisioning can complete while lifecycle progress remains request/response based. Revalidated `pnpm --filter @chariox/kernel-client test -- websocket-pending-requests local-socket-transport`, `/Users/miguel/chariox-cloud pnpm test -- browser-kernel-request-sender browser-kernel-request-correlation`, and local `pnpm --filter @chariox/cli run slice:lifecycle-drill`.
- Fast-forwarded the Hetzner validation checkout `/tmp/chariox-native-remote-validate` to OSS `11680f2`, used the current Rust toolchain, temporarily enabled swap for the 3.7 GiB host, and reran `pnpm --filter @chariox/cli run slice:lifecycle-drill`; it passed headed slice start, worker discovery, screen endpoint, auth import/remove/login, independent auth, wrong-worktree rejection, multi-session/multi-agent reuse, and cleanup.
- Revalidated Cloud/web slice behavior with `/Users/miguel/chariox-cloud pnpm run smoke:browser-relay-kernel:slice`; it passed the browser relay kernel slice panel flow, headed slice lifecycle, provider auth actions, worktree scoping, reuse, active-agent delete blocking, stop, and delete. The available Scalingo-hosted browser relay smoke also passed after loading `CHARIOX_CLOUD_DEV_AUTH_SECRET` from Scalingo; that staging smoke validates hosted relay/browser/kernel/collab transport but does not include slices.

### Slice diagnostics hardening

- Added a kernel-owned `GetSliceLogs` local daemon request/`SliceLogs` response and bumped the shared local daemon protocol to 75. Local Docker slices now expose tailed provisioner action logs plus recent container logs as structured diagnostic entries.
- Added `/slice logs [slice-ref] [--tail <lines>]` to the OpenTUI slash command path and `slice logs [slice-ref] [--tail <lines>]` to the shell/web CLI command path. Cloud web slice controls now include a logs action and bounded inline log viewer.
- Focused validation passed: `cargo test --manifest-path apps/kernel/Cargo.toml slice_logs -- --nocapture`, `pnpm --filter @chariox/cli test -- slice-command-handlers.test.ts shell-executor.test.ts`, and `/Users/miguel/chariox-cloud pnpm --filter @chariox-cloud/web test -- kernel-requests.test.ts SlicesSidebarPanelMount.test.tsx slices-panel-controller.test.ts`.

## 2026-05-29

### Workspace live sync local validation

- Re-ran the full non-Scalingo local validation pass on current OSS `main` HEAD `ec2fddfc` and Cloud `main` HEAD `eac4338`. Focused kernel/protocol validation passed: `cargo test --manifest-path apps/kernel/Cargo.toml protocol_shapes -- --nocapture` (15), `cargo test --manifest-path apps/kernel/Cargo.toml client_protocol_conformance -- --nocapture` (4), and `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture` (126).
- Revalidated local clients and UI surfaces: `pnpm --filter @chariox/kernel-client test` (77), `pnpm --filter @chariox/cli test` (1065), `pnpm --filter @chariox/tool-display test` (9), `swift test --package-path apps/ios/CharioxPackage` (65), `node --check apps/cli/scripts/live-shell-scriptability-drill.mjs`, `pnpm --filter @chariox/cli run shell:drill`, and `/Users/miguel/chariox-cloud pnpm test` (Cloud API 53, worker 4, package suites, web 1108).
- Re-ran the current-head Codex live Workspace Live Sync matrix without Scalingo: local managed with two targets, local same-branch tracked with two targets and bidirectional sync, local cross-branch tracked with two targets and bidirectional sync, local permission gating, same-host relay managed with two targets, same-host relay tracked with two targets and bidirectional sync, and same-host relay permission gating. The tracked drills covered `.charioxignore`, force-excludes, outside-turn ignore, unchanged Git heads/no kernel commits, conflict detection, and resolver convergence. Scalingo/staging hosted drills remain intentionally deferred until the hosted platform is healthy.
- Added the `workspace-live-sync:remote-tracked-restart-drill` alias for the same-host relay restart exit criterion and re-ran it on current `main`. The drill restarted relay before sync, then passed cross-branch tracked fanout with two target worktrees, bidirectional propagation, conflict reporting, resolver convergence, `.charioxignore`, outside-turn exclusion, and unchanged Git heads/no kernel commits.
- Added a kernel-owned Workspace Live Sync enrollment notice when a session workspace link is attached. The notice is delivered through runtime notices to attached clients, names the attached worktree, reports the current mode, and recommends managed mode or the exact `workspace sync enable` command when action is needed.
- Re-ran focused kernel validation: `cargo fmt --manifest-path apps/kernel/Cargo.toml --check`, `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture` (126 tests), and `cargo test --manifest-path apps/kernel/Cargo.toml local_request_api_manages_session_workspace_links -- --nocapture`; coverage includes workspace-link enrollment notice polling, collab fanout, remote membership, `.charioxignore`, force excludes, tracked snapshots, forwarded target-session notices, and conflict/rebase behavior.
- Revalidated the non-Scalingo Codex live matrix after the enrollment-notice change: `workspace-live-sync:managed-drill`, `workspace-live-sync:tracked-drill`, `workspace-live-sync:same-branch-tracked-drill`, `workspace-live-sync:remote-managed-drill`, `workspace-live-sync:remote-tracked-drill`, `workspace-live-sync:permission-drill`, and `workspace-live-sync:remote-permission-drill` all passed. Also fixed `shell:drill` to initialize its isolated workspace as a Git worktree root and re-ran `node --check apps/cli/scripts/live-shell-scriptability-drill.mjs`, `pnpm --filter @chariox/kernel-client test -- shell-executor` (77 tests), `pnpm --filter @chariox/cli test -- workspace-command-handlers` (1065 tests), and `pnpm --filter @chariox/cli run shell:drill`. Scalingo/staging hosted drills remain intentionally skipped while the platform issue is investigated separately.
- Fast-forwarded the Hetzner validation checkout `/tmp/chariox-native-remote-validate` to current HEAD `e50e6731caaa88a32af78db2ff19c1aab5da1445` and rebuilt the remote kernel/relay with Cargo 1.95. Current-head actual Hetzner worker Codex validation passed for managed two-target full drill, tracked two-target bidirectional cross-branch full drill using target branch `hetzner-live-sync-e50e673-tracked`, and remote permission gating with `--hetzner-worker`. Scalingo/staging hosted drills remain deferred.
- Rechecked OpenCode Workspace Live Sync with the working OpenCode Zen provider path rather than the stale OpenAI OAuth path. `openai/gpt-5.2` still fails before Chariox behavior with `Token refresh failed: 401`, but `opencode/gpt-5.2` is healthy and is the validated OpenCode path below.

### OpenCode workspace live sync recheck

- Confirmed OpenCode itself works through the OpenCode Zen provider: `opencode run -m opencode/gpt-5.2 'Reply exactly OPENCODE_ZEN_OK.'` returned `OPENCODE_ZEN_OK`. The OpenAI OAuth path remains stale: `opencode run -m openai/gpt-5.2 ...` fails with `Token refresh failed: 401`.
- Fixed Workspace Live Sync write argument normalization so provider placeholder snapshot ids made entirely of zeroes are treated as absent, matching the existing create/new/absent sentinels. This unblocked OpenCode managed writes that passed `snapshot_id` placeholders for new files. Focused tests passed: `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync_snapshot_id -- --nocapture` and `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync_write_snapshot_id -- --nocapture`.
- Fixed the OpenCode managed live-drill move fixture: because OpenCode does not run the patch phase, the harness now seeds the move source in target worktrees too, so the managed rename starts from a synced file. `node --check apps/cli/scripts/live-workspace-live-sync-drill.mjs` passed.
- Re-ran `node apps/cli/scripts/live-workspace-live-sync-drill.mjs --provider opencode --provider-model opencode=opencode/gpt-5.2 --mode managed --managed-target-count 2 --positive-only --timeout-ms 700000`; it passed in ~63s with two target worktrees, text write/edit, text move, opaque write/move, and direct-write suppression.
- Closed local OpenCode Zen Workspace Live Sync parity. Full managed passed with `node apps/cli/scripts/live-workspace-live-sync-drill.mjs --provider opencode --provider-model opencode=opencode/gpt-5.2 --mode managed --managed-target-count 2 --timeout-ms 700000 --keep-artifacts-on-failure`, covering two target worktrees, text/opaque write/edit/move/delete fanout, direct-write suppression, deterministic overlap conflict, non-overlap rebase, and overlap rejection. Full tracked passed with `node apps/cli/scripts/live-workspace-live-sync-drill.mjs --provider opencode --provider-model opencode=opencode/gpt-5.2 --mode tracked --tracked-target-count 2 --tracked-bidirectional --target-branch live-sync-opencode-tracked-parity-target --timeout-ms 700000 --keep-artifacts-on-failure`, covering two cross-branch targets, bidirectional target-origin sync, `.charioxignore`, outside-turn ignore, no commits, conflict reporting, resolver convergence, and ready/conflict/ready status transitions. The managed drill was hardened so the overlap check uses structured edit results after both agents read the same base snapshot instead of relying on provider prose timing.
- Closed same-host relay OpenCode Zen Workspace Live Sync parity. `node apps/cli/scripts/live-remote-workspace-live-sync-drill.mjs --provider opencode --provider-model opencode=opencode/gpt-5.2 --mode managed --managed-target-count 2 --full --timeout-ms 1000000 --keep-artifacts-on-failure` passed with relay/home/worker kernels, two managed targets, direct-write suppression, deterministic overlap conflict, non-overlap rebase, and overlap rejection. `node apps/cli/scripts/live-remote-workspace-live-sync-drill.mjs --provider opencode --provider-model opencode=opencode/gpt-5.2 --mode tracked --tracked-target-count 2 --tracked-bidirectional --target-branch remote-opencode-zen-tracked-target --full --timeout-ms 1000000 --keep-artifacts-on-failure` passed with two cross-branch tracked targets, bidirectional fanout, `.charioxignore`, outside-turn ignore, no commits, conflict reporting, resolver convergence, and final ready status.
- Revalidated OpenCode Zen after removing the old OpenAI-backed OpenCode assumptions from drill defaults, CLI catalog projection, Cloud web catalog projection, and fixtures. `opencode run -m opencode/gpt-5.2 'Reply exactly OPENCODE_ZEN_OK.'` returned `OPENCODE_ZEN_OK`; `node --check` passed for the touched live drill scripts; `pnpm --filter @chariox/cli test` passed 1065 tests; `cargo test --manifest-path apps/kernel/Cargo.toml --lib opencode -- --nocapture` passed 49 tests; `/Users/miguel/chariox-cloud pnpm --filter @chariox-cloud/web test` passed 1108 tests.
- Revalidated hosted Scalingo/Hetzner relay coverage with OpenCode Zen. The combined hosted run passed remote CLI, terminal pairing with `CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PROVIDER=opencode` and `CHARIOX_CLOUD_HOSTED_REMOTE_CLI_MODEL=opencode/gpt-5.2`, and token rotation before a transient second-kernel WebSocket reset. Separate reruns passed the second-kernel hosted worker path and the multi-user hosted invite/join path against `https://chariox-cloud-staging.osc-fr1.scalingo.io` and `wss://195.201.123.115.sslip.io`.

## 2026-05-28

### Workspace live sync notice validation

- Added target-side runtime notices for forwarded remote Workspace Live Sync applies. When a home kernel fans out a tracked change through relay, the target kernel now records the same summary/conflict/failure notice in the local target session, without attributing it to a source-side provider run that does not exist on the target.
- Re-ran `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture`; 126 focused Workspace Live Sync tests passed, including forwarded target-session notice delivery.
- Tightened workspace-link enrollment so local Workspace Live Sync attachments must be existing Git worktree roots, matching the "attached worktrees only" contract and avoiding accidental dormant path/branch targets. Managed live-drill fixtures now initialize the source and target directories as Git worktrees before attachment.
- Validation for the worktree-root enforcement passed: `cargo fmt --manifest-path apps/kernel/Cargo.toml --check`, `node --check apps/cli/scripts/live-workspace-live-sync-drill.mjs`, `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture` (125 tests), `pnpm --filter @chariox/cli run workspace-live-sync:managed-drill` with two managed targets, and `pnpm --filter @chariox/cli run workspace-live-sync:remote-managed-drill` with same-host relay plus two managed targets.
- Strengthened `apps/cli/scripts/live-workspace-live-sync-drill.mjs` so live Workspace Live Sync runs now fail unless session history contains the user-facing runtime notices for fanout: managed runs with attached targets require a `Workspace live sync managed summary`, and tracked runs require both a `Workspace live sync tracked turn summary` and a conflict notice with resolver next-action text.
- Revalidated the non-Scalingo Codex paths affected by that drill change. Fresh passes: `pnpm --filter @chariox/cli run workspace-live-sync:managed-drill`, `workspace-live-sync:tracked-drill`, `workspace-live-sync:same-branch-tracked-drill`, `workspace-live-sync:remote-managed-drill`, and `workspace-live-sync:remote-tracked-drill`.
- Fast-forwarded the Hetzner validation checkout `/tmp/chariox-native-remote-validate` to current HEAD `8583676029746ed2e1c78a45d3a53c0cff9059fc` and rebuilt the remote kernel/relay. Revalidated actual Hetzner worker Workspace Live Sync with the same notice-gated drill: managed two-target full drill passed with `--timeout-ms 1000000` after one provider-latency retry, and tracked two-target bidirectional cross-branch full drill passed with target branch `hetzner-live-sync-notice-gate-target`.
- Re-ran `pnpm --filter @chariox/cli test`; 1065 tests passed. No Scalingo/staging drills were run because hosted validation remains deferred.

### Workspace live sync sync-group status

- Made workspace links explicit as the Workspace Live Sync session sync-group surface. `GetWorkspaceLiveSyncStatus` now returns `sync_groups` alongside flattened targets/conflicts/ignore state; CLI shell, slash UI, iOS status text/protocol, and Cloud web side panel render the grouping counts. Bumped the local daemon protocol to 63 and refreshed protocol shape hashes.
- Validation passed for the changed surface: `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture`, `cargo test --manifest-path apps/kernel/Cargo.toml protocol_shapes -- --nocapture`, `cargo test --manifest-path apps/kernel/Cargo.toml client_protocol_conformance -- --nocapture`, `pnpm --filter @chariox/kernel-client test -- shell-executor`, `pnpm --filter @chariox/cli test -- workspace-command-handlers session-chrome-state cli-polling-controller`, `swift test --package-path apps/ios/CharioxPackage`, and `pnpm --filter @chariox-cloud/web test -- workspace-sidebar-projection WorkspaceSidebarPanelMount`.
- A broad Cloud workspace test invocation also ran package/API/web tests but hit an unrelated API server assertion (`404 !== 302` in `apps/api/dist/server.test.js`) before the aggregate completed. The direct web package build/test for the changed Workspace Live Sync UI passed.

### Workspace live sync tracked parity validation

- Tightened the shell and slash `workspace sync targets` surfaces so explicit sync groups remain visible even when no flattened target rows are present. Re-ran `pnpm --filter @chariox/kernel-client test -- shell-executor` (76 tests) and `pnpm --filter @chariox/cli test -- workspace-command-handlers` (full CLI package test, 1065 tests).
- Fresh non-Scalingo Codex validation passed for tracked parity with managed mode: local managed two-target fanout, local same-branch tracked two-target bidirectional sync, local cross-branch tracked two-target bidirectional sync, same-host relay managed two-target fanout, same-host relay tracked two-target bidirectional sync, local permission gating, and relay permission gating. Scalingo/staging drills remain deferred while the hosted platform is unhealthy.
- Re-ran `/Users/miguel/chariox-cloud pnpm test` after the API dist fixture fix and workflow-tab cleanup. Cloud API (53), worker (4), package suites, and web (1108) passed, including Workspace Live Sync side-panel status/control coverage.
- Rechecked `pnpm --filter @chariox/cli run workspace-live-sync:opencode-managed-drill`; OpenCode still fails before Workspace Live Sync behavior with `Token refresh failed: 401`. Cleaned the failed drill's relay/kernel/provider processes and transient drill roots.

### Workspace live sync session-mode contract

- Split Workspace Live Sync configuration into global launch policy and session-scoped runtime mode. `/config workspace-live-sync` and shell `config workspace-live-sync` now write only `providers.workspace_live_sync = off|managed|tracked`; `workspace sync off|managed|tracked` sends `SetWorkspaceLiveSyncMode { session_id, mode }` and receives `WorkspaceLiveSyncModeUpdated { session }`.
- Bumped the local daemon protocol to 62 and refreshed kernel, TypeScript CLI/shell, iOS, and Cloud web request/response shapes. Provider launch paths now resolve Workspace Live Sync mode from the session override first, then the global launch policy.
- Local validation passed after the split: `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture` (124 tests), `pnpm --filter @chariox/kernel-client test` (76 tests), `pnpm --filter @chariox/cli test` (1065 tests), `swift test --package-path apps/ios/CharioxPackage` (65 tests), `/Users/miguel/chariox-cloud pnpm test` (Cloud API/worker/web suites), and syntax checks for changed live-drill scripts.
- Local Codex live drills passed after the split: `workspace-live-sync:managed-drill` with two managed targets, `workspace-live-sync:tracked-drill` with two cross-branch tracked targets plus bidirectional fanout/resolver convergence, and `workspace-live-sync:permission-drill` with approval-gated write resumption. Scalingo/staging hosted drills remain intentionally deferred.
- Same-host relay Codex drills passed after the split: `workspace-live-sync:remote-managed-drill`, `workspace-live-sync:remote-tracked-drill`, `workspace-live-sync:remote-permission-drill`, plus relay restart recovery in tracked mode with two tracked targets and bidirectional fanout.
- Actual Hetzner-worker Codex drills passed after the split while Scalingo remained deferred: managed mode with two targets, tracked mode with two cross-branch targets and bidirectional fanout, and remote Workspace Live Sync permission gating.
- Added focused kernel coverage for the collab/fork shape: a source worktree owned by the local user fans a Workspace Live Sync change through an explicit workspace link to a second user's attached fork/worktree, records the second user as the target, and skips the source attachment. Re-ran `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture` (125 tests) and `pnpm --filter @chariox/cli run shell:drill`, including scriptable `workspace sync` status/targets/conflicts/ignore/mode/enable/disable/link commands.
- Re-ran the local tracked same-branch Codex drill explicitly with `--target-branch main`, two tracked targets, and bidirectional fanout. It passed turn-end tracked sync, binary/text add/update/delete/rename, dirty-target rebase/conflict, resolver convergence, `.charioxignore`, outside-turn ignore, and no-commit checks while every attached target remained on `main`.
- Re-ran the non-Scalingo local/relay Workspace Live Sync validation matrix after the Scalingo staging drill was deferred. Fresh passes: kernel `workspace_live_sync` tests (125), `@chariox/kernel-client` tests (76), `@chariox/cli` tests (1065), shell drill, iOS Swift package tests (65), `/Users/miguel/chariox-cloud pnpm test`, local managed, local same-branch tracked, local cross-branch tracked, local permission, same-host relay managed, same-host relay tracked, same-host relay permission, and same-host relay restart tracked. The same-branch tracked alias hit provider latency once at the default timeout, then passed with `--timeout-ms 700000`; tracked aliases now use that timeout.

### Workspace live sync local/client validation closure

- Completed the remaining non-Scalingo validation sweep for Workspace Live Sync after tracked-mode parity work. `pnpm --filter @chariox/kernel-client test` passed with 76 tests, `pnpm --filter @chariox/cli test` passed with 1065 tests, `swift test --package-path apps/ios/CharioxPackage` passed with 65 tests, and `/Users/miguel/chariox-cloud pnpm test` passed through the Cloud API, worker, package, and web app suites, including the Workspace Live Sync side-panel/enrollment coverage.
- Rechecked the old managed-I/O naming surface across OSS and Cloud; the remaining references are mode-specific "managed" wording or historical progress-log notes rather than the feature name. Scalingo/staging hosted drills are intentionally deferred until the hosted platform issue is cleared.

### Workspace live sync Hetzner validation closure

- Extended `apps/cli/scripts/live-remote-workspace-live-sync-permission-drill.mjs` with `--hetzner-worker`, matching the remote workspace live sync drill's actual Hetzner topology: relay and worker kernel run on the configured Hetzner host while the home kernel remains local, with Codex auth synchronized and fixture workspaces mirrored before leased provider launch.
- Updated `apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs` so wrappers can provide an isolated root directory and a post-fixture copy command, and so the drill pumps terminal output while waiting for the workspace live sync permission interaction and final file write.
- Confirmed Codex `gpt-5.2` remote workspace live sync permission validation in both same-host local relay mode and actual Hetzner worker mode. The remote agent's `write_artifact` request surfaced as a home-kernel permission interaction, approval resumed the same turn, and the expected file landed in the coordinated workspace.
- Confirmed full Codex workspace live sync validation against the actual Hetzner worker in both tracked and managed modes. Tracked mode covered two targets, explicit cross-branch binding, bidirectional propagation, `.charioxignore`, outside-turn ignore, no commits, conflict detection, and resolver convergence. Managed mode covered two targets, structured text/opaque writes, move/delete fanout, direct-write blocking, collision behavior, non-overlap rebase, and overlap rejection.
- Current OpenCode live workspace sync drills are blocked before workspace live sync behavior by provider auth (`Token refresh failed: 401`). Treat current OpenCode live validation as an environment gap until auth is refreshed.

### Workspace live sync drill entrypoint cleanup

- Changed the `@chariox/cli` workspace live sync drill aliases to the currently green Codex `gpt-5.2` path, with OpenCode treated as an explicit add-on while provider auth is failing before runtime behavior.
- Made the local and remote workspace live sync drill argument parsers tolerate the conventional `pnpm run <script> -- <args>` separator, and verified both aliases print help through that path.
- Re-ran `pnpm --filter @chariox/cli test` and the focused remote workspace live sync membership authorization test. CLI tests passed with 1065 tests; the kernel test verified non-member denial plus member workspace-link create/attach/status identity recording.

### Workspace live sync protocol contract refresh

- Updated `docs/PROTOCOL.md` section 5.0.1 from managed-only wording to the current Workspace Live Sync contract: managed and tracked modes, turn-end tracked fanout, `.charioxignore` plus force-excludes, explicit workspace-link requirements, relay apply/status shapes, no auto commits, conflict surfacing, and resolver-entry convergence.
- Re-ran `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture`; 124 focused kernel tests passed, including protocol shape/version hashes, relay apply shape, ignore initialization/force-excludes, journal sequencing, tracked snapshots, rebase/conflict handling, membership auth, and relay peer application.

### Workspace live sync validation aliases

- Added explicit `@chariox/cli` aliases for Codex managed, tracked, permission, remote managed, remote tracked, and remote permission Workspace Live Sync drills so the validated local/relay matrix can be rerun without reconstructing long command lines.
- Made the local and remote permission drill parsers accept the normal `pnpm run <script> -- <args>` separator; the local permission drill also accepts `--provider-model PROVIDER=MODEL`, matching the other Workspace Live Sync wrappers.
- Verified all Workspace Live Sync aliases reach `--help`, and re-ran `pnpm --filter @chariox/cli test`; 1065 tests passed.

## 2026-05-15

### M14B actual Hetzner native TUI worker validation

- Added the actual Hetzner worker path to `live-remote-native-tui-drill.mjs`.
  The drill starts the relay and worker kernel on the Hetzner host, reaches the
  relay through an SSH local forward, and bridges Codex/OpenCode worker-local
  provider endpoints back to local native TUIs through an SSH provider endpoint
  bridge.
- Confirmed OpenCode against the Hetzner worker for prompt/turns, provider
  permissions, prompt attachments, two native TUIs plus one Chariox observer in
  one session, no cross-agent marker contamination, and badge transitions back
  to idle.
- Confirmed Codex against the Hetzner worker for the same prompt/turn,
  permission, attachment, separation, and badge-status matrix.
- Confirmed Claude Code prompt/turns against the Hetzner worker through the
  remote-rendered PTY path. The first extended run exposed a credential-transfer
  gap: macOS stores Claude Code credentials in the Keychain, while the Linux
  worker expects `~/.claude/.credentials.json`, so copying `.claude.json` alone
  left the worker in `Not logged in`.
- Confirmed the credential-transfer path for Claude Code by exporting the local
  `Claude Code-credentials` Keychain payload to the worker's
  `~/.claude/.credentials.json`; `claude auth status` then reported
  `loggedIn: true` on the worker.
- Confirmed Claude Code extended Hetzner validation after credential transfer:
  image prompt attachments and native-origin/Chariox-origin permission prompts
  pass through the remote-rendered PTY path.
- Fixed OpenCode native TUI projection for cross-host multi-TUI runs by adding a
  periodic transcript refresh while the provider event stream is active.
- Updated remote permission assertions to check execution files on the worker
  when the provider run is Hetzner-backed.

## 2026-05-14

### M14B native TUI validation reset

- Replaced the stale M14 split-native plan with `docs/M14B_NATIVE_TUI_VALIDATION_PLAN.md`, focused on Chariox-managed native TUI validation across local, standard remote home-worker, and slice scenarios.
- Removed the misleading native-TUI workspace live sync artifact checks from `live-remote-native-tui-drill.mjs`. Native TUI attachment validation now explicitly means prompt files/images, while workspace live sync remains covered by dedicated workspace live sync drills.

### M14 remote native TUI same-host relay

- Confirmed `apps/cli/scripts/live-remote-native-tui-drill.mjs --providers opencode,codex,claude --keep-artifacts-on-failure` against the same-host relay topology.
- The drill launches two native TUI agents plus one Chariox observer CLI in one Chariox session per provider, sends prompts from both Chariox and native TUIs, verifies provider responses appear without cross-agent marker contamination, and checks agent badges move from idle to working/thinking and back to idle.
- Fixed Claude Code remote-rendered auth in the drill by keeping Chariox state isolated through explicit runtime env vars while launching the kernel/provider process with the real user `HOME`, which lets Claude Code see its normal authenticated configuration.
- Fixed Claude Code Chariox-origin prompt submission reliability by staging visible prompt typing and Enter submission instead of sending the prompt plus carriage return in one PTY write.

### M14B standard home-worker native TUI

- Added the Claude Code standard home-worker remote-rendered PTY path. The home
  kernel owns the Chariox session, the worker kernel owns the Claude provider
  process and PTY, and the local native TUI launcher renders/controls that PTY
  through the existing kernel and relay paths.
- Added relay peer support for leased native provider PTY input, plus worker-to-
  home projection of native prompts from active prompt state and worker history.
- Confirmed the Claude standard home-worker prompt/turn drill with two Claude
  native TUIs plus one Chariox observer CLI in one Chariox session, separated
  agents, no marker contamination, and badge transitions returning to idle.
- Confirmed Claude standard home-worker image prompt attachments in both
  directions. Claude-origin local `@path` image prompts are intercepted by the
  remote-rendered wrapper, transmitted as inline prompt attachments, materialized
  on the worker, and injected into Claude Code as worker-local native `@path`
  mentions; Chariox-origin image attachments use the same worker materialization
  path.
- Confirmed Claude standard home-worker permissions in both directions. Native-
  origin and Chariox-origin prompts both surface permission approval in the
  remote-rendered Claude TUI, and the approval is sent back through kernel-owned
  PTY input to the worker provider run.

## 2026-04-25

### OSS iOS app planning baseline

- Added `docs/ios/IOS_APP_PLAN.md` for the native OSS iOS client sub-project.
- Captured the key boundary: iOS is a client surface like the TypeScript CLI and Cloud browser terminal, while the kernel remains the runtime authority for sessions, agents, workflows, provider runs, permissions, workspace live sync, and relay membership.
- Recommended native SwiftUI under `apps/ios`, direct kernel WebSocket transport through `URLSessionWebSocketTask`, Keychain for relay/cloud credentials, XCTest/XCUITest for committed tests, XcodeBuildMCP for the default build/test/run validation loop, and iOS Simulator MCP for explicit QA/dogfooding passes when requested or confirmed.
- Documented that Maestro is a candidate tool future agents should suggest when useful, but they must ask Miguel before adding it to the repo, installing it as a project dependency, or making it part of the official QA gate.
- Added `IOS-001` to the repo-native task board.
- Installed SwiftUI/iOS implementation skills into `~/.codex/skills`: `swiftui-expert-skill`, `swiftui-ui-patterns`, `swiftui-view-refactor`, `swift-concurrency-expert`, `swiftui-performance-audit`, and `ios-debugger-agent`.
- Added Codex MCP server entries for `XcodeBuildMCP` and `ios-simulator`.

## 2026-04-24

### M12 provider-native permissions and mode defaults

- Added provider-native `mode` (`build|plan`) and `permissions` (`required|yolo`) as first-class runtime launch state in the kernel, with effective resolution from session defaults plus agent overrides.
- Added session config keys `agents.mode` and `agents.permissions`, plus agent-local override mutation through the new `UpdateAgentConfig` local daemon request/response.
- Mapped the effective values into provider launch behavior: Codex now derives approval/sandbox policy from `mode + permissions`, while OpenCode now sets `default_agent` and native permission rules for `edit`, `bash`, and `task`.
- Added shell commands `session mode`, `session permissions`, `agent mode`, and `agent permissions`, including `inherit` for clearing agent overrides and `context` output that shows the effective current values.
- Added matching CLI slash commands `/session mode`, `/session permissions`, `/agent mode`, and `/agent permissions`.
- Extended split-pane agent footers to show effective mode and permissions alongside identity/provider/model metadata.
- Verified the slice with `cargo test --manifest-path apps/kernel/Cargo.toml --no-run`, `pnpm --filter @chariox/kernel-client run test`, and `pnpm --filter @chariox/cli run lint`.

### M12 popup blocking spike validation

- Extended the existing controlled-exec spike with a blocking `request_popup` path in the interaction gateway, including timeout/default-on-timeout handling and externally-resolved late answers.
- Added fake drills proving that the popup request really blocks until timeout or later resolution, then resumes with a structured reply in the same turn.
- Re-ran live Codex and OpenCode drills with a forced delayed popup response. Both providers waited on the popup tool call and then completed the same turn after the delayed answer arrived.
- Verified with `cd experiments/controlled-exec-spike && npm run check`, `npm test`, `npm run drill:fake`, and `npm run drill:providers`.
- Latest live artifacts: `experiments/controlled-exec-spike/artifacts/2026-04-24T11-19-09-661Z-provider-drill/`.

### M12 popup + native permission closure

- Added the production popup interaction layer to Chariox proper: `request_popup` now blocks the current turn until the user answers or a timeout/default resolves.
- Added always-injected shared runtime instructions so Chariox runtime MCP tools are advertised independently of workspace live sync mode.
- Surfaced provider-native permissions through the same interaction model:
  - Codex `item/commandExecution/requestApproval`
  - OpenCode native permission events
- Routed CLI interaction-strip answers back to provider-native approval channels and resumed the same turn after approval.
- Fixed transient interaction lifecycle bugs by moving pending interaction / pending MCP continuation stores onto shared process-wide state.
- Fixed Codex native approval handling by restoring the correct unrestricted `required -> untrusted` mapping and the correct app-server decision vocabulary.
- Added live drill coverage:
  - `apps/cli/scripts/live-native-permission-drill.mjs`
  - `apps/cli/scripts/live-popup-drill.mjs`
- Confirmed live native permission drills for both Codex and OpenCode.
- Confirmed live Codex popup execution on the real-home auth path and retained artifacts proving `request_popup` completed and resumed with `USER_FEEDBACK_RESULT:green`.
- Closed the initial local M12 scope. Shell popup queue UX stayed outside M12.

## 2026-04-27

### Workspace live sync permission follow-on

- Added workspace live sync permission gating for mutating Chariox runtime tools. When effective permissions require approval, `write_artifact`, `edit_artifact`, `apply_patch`, `move_artifact`, and `delete_artifact` now block on a Chariox interaction before the mutation applies.
- Split prompt assembly by execution path: all structured runs get the shared runtime instructions, unmanaged runs get the native-permissions block, and workspace live sync runs get the workspace live sync block.
- Added `apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs` and confirmed live workspace live sync permission drills for Codex and OpenCode.

### Remote permission UX follow-on

- Extended the local native-permission and workspace live sync permission drills so they can be driven through an already-running home kernel and a leased remote worker.
- Added relay wrapper drills:
  - `apps/cli/scripts/live-remote-workspace-live-sync-permission-drill.mjs`
  - `apps/cli/scripts/live-remote-native-permission-drill.mjs`
- Confirmed remote workspace live sync permission drills for Codex and OpenCode. Home-kernel interaction strips now surface remote workspace live sync approval requests and resume the same remote turn after approval.
- Fixed leased-worker provider launch propagation so remote backing provider runs now inherit the leased agent's `execution_mode` and `permission_level` when the worker launches or reloads a provider run.
- Fixed the remote native permission completion projection issue by refreshing the home session snapshot after leased prompt settlement, then confirmed remote native provider permission drills for Codex and OpenCode.

### Remote popup UX follow-on

- Added remote support to `apps/cli/scripts/live-popup-drill.mjs` so it can reuse an existing home kernel, spawn a leased remote agent, and drive the home CLI interaction strip against that remote agent.
- Added `apps/cli/scripts/live-remote-popup-drill.mjs` and `pnpm --filter @chariox/cli run remote-popup:drill`.
- Fixed non-permission `request_popup` forwarding for leased worker agents by routing worker-side runtime popup requests through the existing relay native-interaction channel to the home kernel, with timeout/default handling owned by the home interaction.
- Confirmed remote non-permission popup drills for Codex and OpenCode. Both providers passed feedback choice, warning-level choice, and timeout/default popup paths, and each resumed the same remote turn with the selected/default reply.

## 2026-04-19

### M7.5 Chariox Shell core skeleton

- Added the first `chariox-shell` implementation slice: shell command parsing, slash-command normalization, shell context/result types, `as <name>` bindings, variable substitution, TUI-only command classification, and result rendering helpers.
- Verified the slice with the focused kernel-client shell-core test path.

### M7.5 minimal shell executor

- Added a minimal `chariox-shell` executor over normalized shell commands, returning structured shell results for shell-local commands and low-risk kernel-backed session/agent commands.
- Covered `session list`, `session new --dir|--worktree`, `session attach|use`, `agent list`, `agent spawn --dir|--worktree|--machine`, `agent focus`, and `agent cycle`, with variable binding and context update behavior.
- Verified with the focused kernel-client shell-core and shell-executor test path.

### M7.5 standalone chariox-shell entrypoint

- Added `apps/shell/src/shell.ts` as the standalone `chariox-shell` REPL entrypoint, wired to the existing local kernel IPC client and the shared shell parser/executor.
- Added shell package wiring for `chariox-shell` and `pnpm --filter @chariox/shell run start`, with options for kernel endpoint, workspace/worktree, provider, model, and effort.
- Verified with focused shell tests plus a built `node apps/shell/dist/shell.js --help` smoke check.

### M7.5 chariox-shell script runner

- Added `chariox-shell run <file>` for line-oriented Chariox command scripts with comments, blank lines, variable bindings, and stop-on-error behavior.
- Added mocked IPC fixture coverage proving a script can create a session, bind its id, spawn an agent in that current session, and stop before later commands on failure.
- Verified with the focused kernel-client shell tests, shell app tests, and a built `node apps/shell/dist/shell.js run <tmpfile>` smoke check.

### M7.5 shell app split

- Refactored the shell out of the TUI package into `apps/shell`, keeping it as a sibling app to `apps/cli`.
- Extracted shared kernel-facing client code into `packages/kernel-client`, including IPC transport, request builders, minimal kernel runtime types, shell parser, and shell executor.
- Left the TUI-specific CLI types and UI command handling in `apps/cli`; the CLI imports shared IPC/request code through narrow compatibility re-export files.

### M7.5 workspace shell pane

- Added a right-side `chariox-shell` pane to the workflow workspace screen while preserving the workflow outline/canvas on the left.
- Routed `@ <command>` prompt submissions on the workflow screen through the shared shell parser/executor and rendered input/output transcript entries in the pane.
- Kept TUI-only commands outside the shell path and added focused workspace-shell unit coverage.

### M7.5 shell executor read/status coverage

- Added shared `chariox-shell` executor support for `machine list|kernels`, `relay status`, `config show`, `mcp list|show`, `skill list|show`, and `provider status`.
- Added kernel-client runtime types for relay status, remote machines, and remote kernel presence so shell and TUI surfaces can share the same response model.
- Updated standalone shell usage examples and covered the new command families with focused kernel-client tests.


### M7.5 shell executor MCP/skill mutations

- Added shared `chariox-shell` executor support for `mcp install|update|uninstall|import|grant|revoke|grants` and `skill install|update|uninstall|import|grant|revoke|grants`.
- MCP install/update parsing now covers stdio transports with command/args/env vars and streamable HTTP transports with optional bearer-token env vars, matching the provider-facing registry shape.
- Covered install/update/import/grant/revoke/grants flows with focused kernel-client executor tests.


### M7.5 shell executor workflow coverage

- Added shared `chariox-shell` executor support for core workflow commands: `workflow list|new|show|alias|run|runs|run-show|cancel|resume`.
- Added graph-management coverage for `workflow node add|remove`, `workflow edge add|remove`, and `workflow endpoint new|alias|bind`, including current workflow context updates and variable binding for created workflows/nodes.
- Covered workflow list/create/show/alias, run lifecycle, graph, and endpoint flows with focused kernel-client executor tests.


### M7.5 shell executor config mutations

- Added shared `chariox-shell` executor support for `config path`, `config set`, `config unset`, and `config workspace-live-sync`.
- `config workspace-live-sync` accepts `off|managed|tracked` and writes the same user-config key as the TUI command, while reporting that shell changes apply on the next provider launch. Active sessions use `workspace sync off|managed|tracked`.
- Covered config path/set/unset/workspace-live-sync flows with focused kernel-client executor tests.


### M7.5 shell executor Slice 6 closure

- Added remaining shared `chariox-shell` executor coverage for workflow advanced config, node runtime flags, watchdogs, workflow queue management, provider login/logout/reauth/process inspection/teardown, and active prompt cancellation.
- `stop` and `cancel` resolve the current session attachment from session state before sending `CancelActivePrompt`, matching the kernel authorization model without adding TUI-only state to the shell context.
- Covered workflow advanced, provider auth/process, and cancellation flows with focused kernel-client executor tests. Slice 6 command-family coverage is now closed.

### M7.5 shell scriptability hardening

- Added script runner ergonomics for repeated `--var NAME=VALUE` seed bindings, `--continue-on-error`, and line-numbered failure diagnostics.
- `chariox-shell run <file>` still stops on first error by default, while validation/drill scripts can continue after structured command failures or thrown transport/kernel errors and return non-zero if any command failed.
- Added standalone shell session attachments so `stop` and attachment-scoped session config commands can run from `chariox-shell`, not only from the TUI.
- Added and passed `live-shell-scriptability-drill.mjs` through `pnpm --filter @chariox/cli run shell:drill` against an isolated local kernel.
- Added `source <file>` / `run <file>` support inside `chariox-shell` and nested scripts, preserving context and variable bindings after loading scripts from disk.

### Session/agent git worktree placement

- Added local session and agent placement commands for existing directories and git worktree creation: `/session new [DIR]`, `/session new --worktree DIR --branch BRANCH [--from REF]`, and `/agent spawn ... --worktree DIR --branch BRANCH [--from REF]`.
- Extended remote agent spawn so `/agent spawn ... --machine MACHINE --worktree REMOTE_DIR --branch BRANCH [--from REF]` forwards a worktree placement spec to the worker kernel, which runs `git worktree add` on the remote machine before creating the leased backing session/agent. Git/repo/configuration failures are surfaced as worker errors.
- Verified local placement with a real temporary git repo/worktree command-action drill and remote placement with a worker-side remote-lease git worktree materialization drill, plus focused CLI and kernel tests.

## 2026-04-18

### M4.6 workspace live sync Codex hardening and live drills

- Root-caused the local workspace live sync drill failure to Chariox's Codex app-server permission approval path: managed Codex turns used the read-only sandbox, but Chariox was approving Codex `item/permissions/requestApproval` requests wholesale, allowing native shell to acquire filesystem write permission.
- Hardened managed Codex runs so permission approvals preserve non-write requests but never grant filesystem write upgrades, while unrestricted Codex runs keep the previous permissive behavior.
- Split the local workspace live sync drill's positive provider phases into smaller serialized prompts so the drill validates each managed read/write/edit/apply-patch/move/opaque step deterministically before entering the direct-write negative checks.
- Verified with `cargo check --manifest-path apps/kernel/Cargo.toml`, focused Codex permission/runtime tests, `node --check apps/cli/scripts/live-workspace-live-sync-drill.mjs`, the full local workspace live sync drill for OpenCode Zen `opencode/gpt-5.2` plus Codex `gpt-5.2`, and the local runtime MCP reattach drill for both providers.

## 2026-04-15

### M4.5 production ownership closure status

- Closed the seven ownership points: direct-cutover baseline, session ownership, prompt ownership, provider process/output ownership, workflow/runtime-tool ownership, transport/relay ownership, and runtime fallback deletion now route command/runtime behavior through owned runtime ports.
- Completed the M4.5 dead-code purge by deleting now-unused app-backed session/projection/remote-lease/workflow-console helpers, the obsolete app-backed runtime-tool dispatcher and its tests, and stale test-only calls into compatibility helpers.
- Verified with clean `cargo check`, daemon lib tests, runtime integration tests, kernel websocket integration tests, relay-client tests, and daemon bin tests. Recommendation: treat M4.5 ownership as closed and move next to final docs/invariant alignment before the final I/O-coordination slice.

### M4.5 live drill gate alignment

- Added [LIVE_DRILLS.md](/Users/miguel/chariox/docs/ops/LIVE_DRILLS.md) as the gate before new tasks. It covers local freeform multi-agent mode, local workflow drills, remote freeform relay drills, and remote workflow relay drills.
- Adopted **freeform multi-agent mode** as the name for normal non-workflow multi-agent sessions, replacing the working phrase "unscheduled mode".
- Next recommendation: run the live drill matrix and fix only drill blockers before opening new feature work.

### M4.5 prompt ownership hard-center update

- Moved the single-agent slow-structured local prompt submit path onto owned `CompatibilityRuntimeState` stores, including user history append, prompt-owner submit/mirror mutation, queued notice emission, prompt echo, and dispatch preparation without waiting on the compatibility app mutex.
- Added owned queued prompt advancement for the same single-agent slow-structured path, including expected queue-front activation, provider claim acquisition, structured submit enqueue, prompt activity tracking, and session/agent-runtime projection refresh.
- Kept multi-agent structured prompt scheduling, PTY prompt delivery, provider launch-on-submit, remote prompt submit, workflow-owned prompt progression, and broad provider dispatch semantics on the existing compatibility/provider/workflow paths until points 4 and 5 own those side effects.
- Verified the slice with `cargo test` in `apps/kernel`: 338 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 structured local prompt cancellation ownership update

- Moved structured local prompt cancellation admission and prompt-owner mutation onto owned `CompatibilityRuntimeState` stores for non-remote, non-workflow prompts, including attachment validation, provider-run validation, cancelling-state mirroring, cancellation notices, prompt-activity settlement markers, structured abort dispatch creation, and projection refresh.
- Kept PTY Ctrl-C cancellation, remote prompt cancellation, workflow-owned prompt cancellation, and abort dispatch execution side effects on existing compatibility/provider-runtime paths for follow-up slices.
- Verified the slice with `cargo test` in `apps/kernel`: 336 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 simple local prompt completion ownership update

- Moved the simple local prompt-completion path onto owned `CompatibilityRuntimeState` stores when there is no queued prompt to advance and the active prompt is not remote-backed or workflow-owned, including prompt-owner mutation, session prompt-state mirroring, completion notification recording, prompt-activity cleanup, and projection refresh.
- Kept queued prompt advancement, workflow completion, remote prompt completion, and claim-release retry side effects on the compatibility fallback until those owners are cut over in follow-up slices.
- Verified the slice with `cargo test` in `apps/kernel`: 335 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 local agent destroy ownership update

- Moved local agent destroy onto the owned `CompatibilityRuntimeState` session and agent stores when the target agent is not remote-backed, preserving the compatibility fallback for remote execution lease/relay teardown.
- Added a no-app-lock regression proving local destroy removes the agent from both session and agent-runtime projections while the compatibility app mutex is held.
- Verified the slice with `cargo test` in `apps/kernel`: 334 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 local agent spawn ownership update

- Moved local session-scoped agent spawn onto the owned `CompatibilityRuntimeState` session and agent stores when runtime-owned state is available, keeping remote machine spawn on the compatibility fallback because it still owns relay/lease side effects.
- Fixed session-runtime projection refresh for `AgentSpawned` and `AgentDestroyed` responses so session and agent-runtime projections update from agent lifecycle responses instead of depending on a separate compatibility snapshot path.
- Verified the slice with `cargo test` in `apps/kernel`: 333 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 session alias ownership update

- Moved session alias updates onto the owned `CompatibilityRuntimeState` session store when runtime-owned state is available, with a no-app-lock regression covering alias mutation and projection refresh while the compatibility app mutex is held.
- Verified the slice with `cargo test` in `apps/kernel`: 332 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 session-runtime ownership update

- Moved session create/default-agent bootstrap and session config updates onto the owned `CompatibilityRuntimeState` session, agent, attachment, terminal, history, and projection stores when runtime-owned state is available. These session-runtime paths no longer wait for the compatibility app lock in the normal router-owned configuration, with app-lock fallback kept only for no-owned-state legacy tests.
- Fixed session creation responses to return the refreshed focused-agent session after default-agent bootstrap, so session/focus projections are populated from the authoritative post-bootstrap state.
- Verified the slice with `cargo test` in `apps/kernel`: 331 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 runtime integration bridge update

- Restored the daemon integration suite after direct facade retirement by routing the stale test-facing session, prompt, terminal, provider-output, and structured-runtime-state helpers through the explicit `KernelSessionService`, `KernelAgentService`, provider-output pump, provider terminal-input, and provider-run actor boundaries instead of the deleted generic local request dispatcher.
- Verified the slice with `cargo test` in `apps/kernel`: 329 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 direct-cutover direction

- Updated the M4.5 plan to stop treating compatibility preservation as ongoing work. The next slices should replace app-backed runtime ports with owned services and delete the old helper path in the same slice once tests cover the owner path.
- Current code has moved more state and side effects behind owned seams: provider launch state, prompt dispatch/abort enqueue paths, structured prompt I/O reads, provider-output fanout/history writes, prompt-activity updates, workflow prompt progression, workflow console helpers, watchdog pumping, and blocked-claim retry now enter explicit runtime boundaries.
- The remaining M4.5 blockers are still the app-lock ports inside `CompatibilityRuntimeState`: session lifecycle/config/alias/agent mutations, prompt submit/cancel/complete settlement and queue advancement, provider PTY spawn/remove/poll/drain and provider-output prompt settlement, workflow service mutation/runtime tools, transport subscription/replay snapshots, and relay peer/lease state.
- Final I/O coordination remains separate. Keep coarse `WorkspaceCoordinator` claims active during the direct cutover, then design file-level claims, port claims, shell/harness sandboxing, coordinator-owned patch application, and transactional rebase/repair after the actor/projection runtime no longer depends on hot `DaemonApp` paths.

## 2026-04-12

### M4.5 kernel runtime refactor progress

- Landed the first substantial kernel responsiveness slices:
  - `KernelCommand` / `KernelEvent` envelopes and command routing
  - bounded `EventLog` replay with explicit replay-gap behavior
  - session snapshot projection metadata
  - bounded interactive routing and inbound WebSocket admission
  - safe command-id retry handling with in-flight fanout and conflict rejection
  - typed CLI replay-gap handling and user-visible refresh notice
  - local IPC compatibility routing through the same command normalization path
- Moved structured provider submit, abort, and output polling through provider-run actors.
- Added runtime slot tombstones/generations so cleanup racing with slow provider I/O cannot restore stale runtime state.
- Removed provider-family global locks from structured output polling while provider I/O is in progress.
- Fixed the websocket integration harness port race by reserving listeners through server startup.
- Added responsiveness/race coverage for slow history, provider catalog, shell capability, provider launch, structured submit/cancel/poll, slow consumers, replay gaps, duplicate command ids, and runtime cleanup races.
- Introduced `KernelSessionService` and moved session attach, detach, end, delete-by-ref, focus/cycle, and terminal resize behavior behind that service while keeping `DaemonApp` as a compatibility facade.
- Introduced `KernelAgentService` and moved prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, and cancellation finalization behind that service while keeping `DaemonApp` as a compatibility facade.
- Added `AgentRuntime` per-agent mailboxes for prompt submit/cancel admission so agent prompt commands no longer wait behind the generic interactive queue.
- Added `SessionRuntime` per-session mailboxes for attach, detach, focus/cycle, resize, end, and delete admission so session UI/lifecycle commands are isolated from the generic interactive queue and from unrelated sessions.
- Added session mailbox deregistration after successful end/delete so closed sessions do not leave stale mailbox registrations behind.
- Added projected session lane-key lookup for session delete and detach admission, so delete-by-ref and detach-by-attachment can route to the right session mailbox from warmed projection state before falling back to the compatibility app lock.
- Added `DaemonHealthProjection` snapshots for session/agent command mailboxes, provider runtime operation lanes, projected session counts, active/queued prompt counts, and provider-catalog cache state exposed through `GetDaemonHealth`.
- Added a session-owned focused-agent projection shared by `SessionRuntime`, `AgentRuntime`, and the router's agent-lifecycle response path, so untargeted prompt submit/cancel routing can resolve the focused agent without taking the compatibility `DaemonApp` lock once focus is warmed by session commands or local agent spawn/destroy responses.
- Added the first shared session projection store. `GetSessionState`, successful `ResolveSession`, and warmed `ListSessions` now serve projected data without taking the compatibility `DaemonApp` lock; the router refreshes the projection from session-bearing responses and list responses. List responses also hydrate the per-session projection map, so follow-up state and successful resolve reads can return from projection without a separate compatibility-store read.
- Added the first agent runtime projection store. Session projection refreshes now materialize per-agent active and queued prompt counts, direct agent/session-scoped reads are available, and `GetDaemonHealth` uses those counts as the canonical prompt-work health source while mirroring them into the legacy session-projection counters for compatibility.
- Extended the agent runtime projection with each agent's front queued prompt and made local/remote queue advancement inspect that projected candidate before falling back to compatibility session queue reads.
- Updated detach prompt cleanup to republish session and agent-runtime projections after active/queued prompt removal, preventing stale queue-front reads during follow-up advancement.
- Moved provider idle settlement's active-prompt status check to the agent-runtime projection first, with compatibility session inspection retained as fallback when no projected active prompt is warm.
- Moved agent-runtime prompt projection publication into the per-agent mailbox execution path for prompt submit/cancel, so the mailbox runtime now updates its own active/queued prompt read model while compatibility session projections remain mirrored.
- Routed kernel `CompletePrompt` through the per-agent mailbox, resolving the active prompt owner from warmed session projection before falling back to compatibility session state and publishing completion state into the agent-runtime projection from that mailbox worker.
- Moved `CompletePrompt` owner lookup to the agent-runtime active-prompt projection first, so completion routing can stay off the compatibility app lock even when the warmed session projection is stale.
- Moved kernel `CancelActivePrompt` owner lookup to the same agent-runtime active-prompt projection resolver before per-agent mailbox dispatch, with mailbox execution still enforcing attachment/session and active prompt validation.
- Removed the duplicate router-side agent-runtime projection refresh for `PromptSubmitted`; the router still refreshes the session projection from the response while the agent mailbox owns the agent-runtime prompt projection.
- Extended untargeted prompt routing to use warmed session projection focused-agent state before falling back to the compatibility app lock when the dedicated focused-agent projection is cold.
- Added a shared session-history projection. `GetSessionHistory` can now load from disk using a warmed session snapshot without taking the compatibility `DaemonApp` lock, and repeated warmed transcript reads are served from memory while successful history appends keep the warmed projection current.
- Added a shared provider-run projection. Warmed `GetProviderRun` reads now return without taking the compatibility `DaemonApp` lock, and launch/start/finish/fail/park/resume/ended lifecycle updates refresh the warmed projection.
- Added a warmed provider-process projection. Repeated `ListProviderProcesses` reads now return without taking the compatibility `DaemonApp` lock, while provider-run and session lifecycle changes invalidate the projection so teardown-safety metadata is not served stale.
- Added prompt lifecycle publication into the shared session projection. Prompt submit, complete, cancel, dispatch failure, and queue advancement now update warmed prompt-state snapshots so `GetSessionState` can reflect those transitions without taking the compatibility app lock.
- Removed redundant router-side session snapshots for prompt complete/cancel. Those paths now rely on prompt lifecycle projection publication, keeping follow-up warmed `GetSessionState` reads projection-first without an extra compatibility-store refresh.
- Trimmed router-side session snapshots for non-state terminal control commands: `PollRuntimeNotices` and `ResizeTerminal` no longer perform post-response session projection snapshots, while `PumpTerminalOutput` still does because provider output pumping can settle prompt state.
- Added a TTL-bound provider-catalog projection. Warmed `GetProviderCatalog` reads now return without taking the compatibility app lock, and provider logout/configuration changes invalidate the projection.
- Fixed projection correctness gaps: provider-process projection now stores a canonical unfiltered snapshot and refreshes after teardown, warmed OpenCode `GetProviderRun` no longer bypasses selection-sync side effects, relay reconfiguration invalidates provider catalog projection state, and agent lanes are removed on agent/session cleanup.
- Added warmed session-projection reads for agent and workflow inspection: `ListAgents`, `ListWorkflows`, `ResolveWorkflow`, `ListWorkflowRuns`, `GetWorkflowRun`, `ListWorkflowWatchdogs`, and `ListQueuedWorkflowPrompts` can now return without taking the compatibility app lock once the session projection is warm.
- Added transport health projection counters for kernel websocket pressure: active connections/subscriptions, incoming requests, emitted events, replay gaps, inbound overload rejections, outgoing queue overflows, and slow-consumer closes are now exposed through `GetDaemonHealth`.
- Added a workspace coordination health baseline: active worktree claims and same-workspace worktree collisions are now reported from the warmed session projection through `GetDaemonHealth`.
- Added initial `WorkspaceCoordinator` enforcement for explicit file-writing capabilities. `EditFile` and `StoreTransferredFile` now acquire scoped worktree write claims, reject overlapping same-workspace/worktree writes with a retryable workspace-claim conflict, publish active operation claims through daemon health, and release claims on operation completion.
- Removed provider prompt lifecycle worktree claims. Active local provider prompts no longer acquire whole-worktree claims, so independent sessions can dispatch prompts in the same workspace/worktree; explicit write capabilities and workflow node dispatch remain claim-coordinated.
- Promoted workspace claims into the workflow scheduler. Claims now expose `read`/`write` mode metadata, workflow node dispatch acquires an exclusive `workflow_node_dispatch` write claim before provider submission, blocked nodes move to `BlockedOnWorkspaceClaim`, and claim release retries blocked workflow nodes instead of failing temporary contention.
- Clarified the claim strategy after review: current claims should remain a coarse safety/scheduler layer while M4.5 finishes actor/projection ownership. Deeper I/O coordination, including file-level claims, port claims, harness sandboxing, coordinator-owned patch application, and automatic patch rebase loops, is intentionally deferred to the final coordination slice.
- Removed the duplicate `AgentRuntimePromptStateStore` shadow after review. `AgentRuntimeProjectionStore` is now the single warm prompt-state read model for active-owner routing, queue-front preview, daemon health, and projection-first reads while `PromptStateOwner` is the mutation authority and compatibility session state remains the mirror.
- Changed structured provider submit/abort/output-poll/selection-sync enqueue failures to propagate as daemon errors instead of being logged and swallowed. This keeps prompt dispatch cleanup, claim release, notices, and retryable failures on the normal error path when a provider actor does not accept work.
- Added a per-provider-run structured output return buffer so globally drained background output still comes back from the later direct pump for that provider run, without delaying terminal fanout.
- Introduced `PromptRuntimeState` inside the compatibility session mirror. It is now the only writer for per-agent active/queued prompt state and the legacy session-level prompt/scheduler projections, while serialization remains flattened to the existing wire fields.

## 2026-04-13

### M4.5 kernel runtime refactor progress

- Added `PromptStateOwner` as the kernel write owner for per-agent active prompts and queued prompt backlogs. Prompt submit, complete, cancel, cancellation finalization, dispatch-failure cleanup, queue advancement, detach cleanup, provider settlement, and workflow prompt submission now mutate the owner first and then mirror into compatibility `RuntimeSession` prompt fields.
- Demoted `PromptRuntimeState` to the flattened compatibility mirror/projection boundary. It still preserves the existing wire shape for active prompt, queued prompts, per-agent prompt states, and scheduler state, but it is no longer the hot prompt lifecycle authority.
- Removed projection-based prompt submit admission as an authority. Agent-runtime projections still provide warm queue-front previews and health/read models, but stale projection state cannot force an otherwise idle prompt owner to queue.
- Added regression coverage that deliberately corrupts the compatibility session mirror and verifies completion still succeeds from the prompt owner.
- Promoted `PromptStateOwner` from a private compatibility-app sidecar to a cloneable kernel service shared with `AgentRuntime`. Active-prompt owner resolution and complete queue-front preview can now consult the owner without taking the app lock when a session projection is warm, and regression coverage locks the stale-mirror/no-app-lock path.
- Moved `PromptRuntimeState` into `session/prompt_runtime.rs` as a dedicated compatibility prompt mirror boundary. `RuntimeSession` still flattens and forwards it for wire compatibility, but scattered prompt mutation is no longer embedded in the shared session type.
- Hardened `WorkflowRuntime` lane admission and projection refresh. Warmed missing-session workflow mutations now fail from `SessionStateProjectionStore` without creating a workflow lane or waiting on the compatibility app lock, and workflow workers refresh session/agent-runtime projections directly from session-bearing workflow responses before falling back to a compatibility snapshot.
- Hardened `SessionRuntime` projection publication. Session mailbox workers now publish session-bearing create/config/alias/end responses directly, remove deleted-session projections at the mailbox boundary, and only fall back to compatibility snapshots for responses that do not carry enough session state.
- Trimmed prompt-completion app-lock work in `AgentRuntime`. The agent mailbox now consumes the session projection published by the prompt lifecycle service instead of taking a second compatibility session snapshot after completing a prompt.
- Deferred kernel prompt-submit side effects out of the agent-mailbox acknowledgement path. User-prompt history appends now run through spawned blocking persistence with projection refresh after success, and remote relay prompt submit now returns a dispatch object that is spawned after owner mutation; remote dispatch failure cancels the active prompt, refreshes projections, and records a notice.
- Added projection publication after workflow runtime-tool calls. Direct MCP/relay runtime-tool mutations now republish the session and agent-runtime projections after recording the tool call, so workflow turn acknowledgements and output submissions do not leave warmed workflow inspection reads stale when they bypass the router workflow lane.
- Routed relay-client daemon/workflow requests through `CommandRouter` as `KernelCommandSource::RelayClient`. Proxied relay clients now share the same actor admission, projection refresh, and overload behavior as local IPC/kernel transport requests, regression coverage verifies relay list requests warm the shared session projection, and workflow validation requests no longer die at the relay transport gate.
- Consolidated workflow command handling behind the workflow-runtime boundary. `WorkflowRuntime` workers now invoke the explicit workflow request handler instead of the generic local compatibility handler, and local IPC delegates workflow requests to that same handler so workflow mutation logic is not doubled while `DaemonApp` remains the compatibility mirror.
- Consolidated session command handling behind the session-runtime boundary. `SessionRuntime` workers and local IPC now delegate lifecycle, focus, resize, notice, config, alias, end, and delete commands to one explicit session request handler, removing the duplicate session mutation implementations from the actor and local API paths.
- Consolidated agent prompt command handling behind the agent-runtime boundary. Local IPC, the legacy interactive fallback, and `AgentRuntime` now share one explicit agent request handler for prompt submit, completion, and cancellation while prompt mutation still goes through `KernelAgentService` and `PromptStateOwner`.
- Closed the legacy generic interactive fallback. The fallback lane now accepts only explicit session/agent commands and rejects unsupported commands immediately instead of re-entering the full local API or waiting on the compatibility app lock.
- Routed agent lifecycle mutations through the session runtime lane. `SpawnAgent` and `DestroyAgent` now normalize as interactive session-scoped commands, share the explicit agent request handler with local IPC, and publish session/focus projections from the runtime path instead of entering the normal local fallback.
- Routed runtime MCP through `CommandRouter`. The MCP HTTP server now binds from the router-owned config projection, authenticated local workflow runtime tools dispatch through the router, and forwarded relay workflow runtime tools also enter through the router instead of locking `DaemonApp` directly from transport handlers.
- Added explicit router handlers for relay configuration and remote-machine registry mutations. `ConfigureRelay`, `ApproveRemoteMachine`, `ForgetRemoteMachine`, and `RenameRemoteMachine` now invalidate provider-catalog projections from the router path instead of falling through the generic local compatibility request handler.
- Removed the normal/background generic local compatibility fallback from `CommandRouter`. Every non-interactive request now has an explicit router branch: warmed projection reads return early, cold session/provider/capability paths are named, workflow requests enter `WorkflowRuntime`, and provider auth/login/logout no longer wait on the app lock before running provider-side work.
- Split the remaining router cold-read/provider-sync paths away from the public local API facade. `CommandRouter` now calls named session/list/resolve/state/agent-list and provider-run helpers directly, leaving `DaemonApp::handle_local_request` out of production router dispatch.
- Ran the post-slice live drill set: daemon smoke harness, focused-agent multi-agent prompt routing, shared-endpoint multi-agent prompt routing, workflow progression without terminal pumps, downstream workflow scheduling, join-node scheduling, workflow workspace-claim retry, and CLI workflow graph/outline drill catalogs all passed.
- Removed the unused direct complete-and-auto-advance prompt mutation API from `RuntimeSession` and `PromptRuntimeState`, leaving completion on the kernel lifecycle path that reconciles against the agent-runtime queue-front preview before explicit queue advancement.
- Aligned direct compatibility complete/cancel owner resolution with the agent runtime rule: prefer the focused agent only when it is active, otherwise resolve the single active agent and reject ambiguous multi-active ownership.
- Narrowed compatibility prompt mutation visibility. `RuntimeSession` prompt mutators are now private to the session module tree, and provider dispatch failure cleanup now calls back into `KernelAgentService` instead of reaching into `SessionService` directly.
- Moved public session creation and default-agent bootstrap behind `KernelSessionService`, leaving `DaemonApp::create_session` as a compatibility facade instead of a direct lifecycle owner.
- Added daemon-health projection invariant reporting for session/agent-runtime prompt drift, with regression coverage that detects stale agent-runtime queue-front and queued-count projections.
- Routed public `CreateSession` through the session runtime mailbox boundary, including projection and focused-agent publication for the created session, so creation is not rejected behind the generic interactive lane.
- Collapsed `CreateSession` response construction into one compatibility helper shared by local API and session runtime dispatch, avoiding duplicate create/logging components while the runtime migration is still in progress.
- Added `docs/ops/IMPLEMENTATION_INVARIANTS.md` as the explicit M4.5 gate for ownership, projection refresh, cleanup, overload, health, and tests before final I/O coordination starts.
- Recorded the full A+ sequence for the rest of M4.5: prompt ownership, session ownership, projection correctness, workflow hardening, provider/terminal hardening, hot app-lock removal, docs/invariant lock, and final I/O coordination last.

### Remaining M4.5 work

- Move `KernelSessionService` session state into the new `SessionRuntime` mailbox owner, then finish removing the remaining prompt claim/mirror side effects from the shared app lock.
- Expand actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots so remaining provider/read models no longer require synchronous compatibility-store access.
- Keep current workspace claims bounded until actor/projection ownership is complete; return to file-level scopes, port claims, harness enforcement, and transactional mutation/rebase semantics in the final I/O-coordination slice.
- Retire remaining hot request paths that depend on `Arc<Mutex<DaemonApp>>`.
- Run live multi-agent and workflow drills only after all non-I/O ownership/runtime slices above are complete.

## 2026-03-31

### Kernel transport hardening follow-up

- Landed resumable kernel WebSocket transport hardening for the TypeScript CLI: durable event ids, resumable subscribe, reconnect/resubscribe, heartbeat events, and bounded slow-consumer handling.
- Added layered coverage for the hardened transport:
  - TypeScript client transport contract tests
  - daemon kernel-WebSocket integration tests
  - live forced-disconnect and slow-consumer drills
- The remaining transport drills are narrower now:
  - deeper live replay/catch-up validation during active streaming output
  - long-idle heartbeat/liveness validation
- Extracted CLI live-event application and transcript-history seams so incremental pushed-event behavior and reattach catch-up can be tested directly instead of only through manual PTY runs.

### Manual multi-agent session runtime slice

- Landed the first real M4 runtime slice instead of keeping agent handling as footer/chrome-only plumbing.
- Added daemon-owned top-level agent runtime services under `apps/kernel/src/agent/`.
- Direct prompt submission now targets `focused_agent_id`.
- Provider runs are now associated with top-level agents, and the daemon parks/resumes runs as focus changes or the session returns to idle.
- Session history entries now carry `agent_id`, so provider output, notices, and user prompts can be partitioned by agent in the local runtime.
- The TypeScript CLI now supports `individual` and `split` multi-agent response modes plus visible per-agent transcript panes/previews.
- Added shared domain and Prisma updates for focused agents, agent-owned provider runs, and prompt queue targeting.

### Docs alignment update

- Updated roadmap/status docs to reflect that manual multi-agent session runtime is now in progress and no longer just planned plumbing.
- Updated local-running/protocol notes so they describe focused-agent prompt routing, agent-scoped history/provider-run ownership, and the current split-pane CLI behavior.
- Re-sequenced the spec/roadmap around an OpenCode-first development cycle: close one provider deeply first, then polish the CLI, then add multi-platform clients, and only after that expand provider support.

### Known follow-up from current code state

- The OpenCode-backed multi-agent runtime path is not fully stable yet.
- The current daemon integration suite still reports failures around:
  - provider-run launch health checks in the OpenCode event-stream path
  - delayed local-response handling through the local transport
- The current split-pane CLI is still a first slice centered on the primary transcript plus up to two auxiliary panes.

## 2026-03-30

### CLI TUI repaint skill

- Added `docs/CLI_TUI_REPAINT_SKILL.md` as a repo-native repaint playbook for future agents working on OpenTUI/JVX visual update bugs.
- Captured the main lesson from split-pane focus bugs: proactive multi-pass repainting and child-renderable rebuilds matter more than only changing parent pane colors.

## 2026-03-29

### Multi-agent docs alignment update

- Reviewed the current daemon and TypeScript CLI agent plumbing after reproducing that focused-agent changes currently affect footer/chrome state more than actual runtime routing.
- Updated `README.md`, `docs/spec-v1.md`, `docs/ARCHITECTURE.md`, `docs/PROTOCOL.md`, `docs/RUNNING_LOCAL.md`, `docs/ROADMAP.md`, and `docs/ops/TASKS.md` so they distinguish three things clearly:
  - current single-agent-effective runtime behavior
  - already-landed session-agent metadata/focus plumbing (`/agent ...`, `Ctrl+A`, focused-agent state)
  - the intended next milestone: manual multi-agent sessions with per-agent context/history and split-pane CLI rendering before workflow automation
- Reframed the roadmap so manual multi-agent session execution is the next step ahead of daemon-scheduled workflow topology work.

## 2026-03-22

### CLI transcript highlighting update

- Added `docs/CLI_TRANSCRIPT_HIGHLIGHTING_PLAN.md` to define transcript syntax highlighting as an M3 TypeScript CLI subphase separate from LSP.
- Implemented markdown-aware assistant/reasoning transcript rendering in the TypeScript CLI.
- Implemented syntax-highlighted fenced code blocks in the TypeScript CLI transcript using OpenTUI parser/code rendering infrastructure.

## Historical Milestone Archive

Older M0-M14 milestone implementation notes and validation history now live in [PROGRESS_LOG_MILESTONE_ARCHIVE.md](PROGRESS_LOG_MILESTONE_ARCHIVE.md). Keep this active progress log focused on the current validation window and recent operational follow-up.
