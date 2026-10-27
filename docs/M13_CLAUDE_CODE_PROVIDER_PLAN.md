# M13 Claude Code Provider Plan

## Status

M13.1, M13.2, M13.3, and the local M13.4 workflow path are implemented as
of 2026-05-13.

Achieved:

- Added the kernel-owned Claude Code structured-stdio provider adapter.
- Added launch-time Chariox permission/plan-mode mapping to Claude Code
  `--permission-mode`.
- Added Claude catalog exposure with known CLI aliases plus defensive
  `~/.claude.json` model-cache discovery when Claude Code writes model options.
- Added stdout/stderr stream-json runtime parsing for session/model metadata,
  assistant deltas, reasoning deltas, usage, terminal failures, and turn
  completion.
- Added focused unit tests and the `claude-provider:drill` live drill.
- Added M13.2 text/base64 attachment transmission into Claude user content,
  including opaque attachment fallback references.
- Added M13.3 Claude resume state with serialized `claude_session_id`,
  provider-session-id launch mapping, and protocol version 27.
- Added Claude restart/resume for changed Chariox model/effort/execution config.
- Added Claude abort recovery by interrupting the stream-json process, restarting
  with `--resume` when a Claude session id is known, and settling Chariox
  cancellation state.
- Added Claude runtime MCP launch config through `--mcp-config` and
  `--strict-mcp-config`, keeping MCP wiring inside the provider adapter.
- Verified local workflow parity with a Claude workflow node using a granted
  provider MCP tool plus Chariox runtime MCP workflow output submission.
- Added Claude Code to CLI waiting-room/backend selection, command-center
  provider selection, provider preference persistence, and backend model
  filtering.
- Normalized OpenCode's upstream provider catalog into the single Chariox
  backend provider `opencode`, so `openai` is no longer surfaced as a connected
  top-level provider. Codex remains the top-level OpenAI-backed Chariox provider.
- Fixed local-drill relay startup when a persisted Cloud relay profile exists:
  explicit `CHARIOX_RELAY_URL` / `CHARIOX_RELAY_TOKEN` now take precedence and do
  not get replaced by automatic Cloud relay token refresh during daemon startup.
- Verified the remote machine Claude path through a local relay: home and worker
  daemons registered in the same scoped relay realm, the waiting room surfaced
  the worker kernel with Claude/Codex/OpenCode, a remote Claude agent spawned on
  the worker machine, completed one prompt, and accepted cancellation of a second
  prompt.
- Added native Claude TUI hidden context delivery through the
  `UserPromptSubmit` hook `additionalContext` path, including local launcher
  responses and worker-kernel responses for remote/slice native TUI execution.

Verified:

- `cargo test --manifest-path apps/kernel/Cargo.toml claude --lib`
- `cargo build --manifest-path apps/kernel/Cargo.toml --bin chariox-kernel`
- `node --check apps/cli/scripts/live-claude-provider-drill.mjs`
- `node apps/cli/scripts/live-claude-provider-drill.mjs --timeout-ms 180000 --keep-artifacts-on-failure`
- `node apps/cli/scripts/live-claude-provider-drill.mjs --scenario attachment --timeout-ms 180000 --keep-artifacts-on-failure`
- `node apps/cli/scripts/live-claude-provider-drill.mjs --scenario resume --timeout-ms 180000 --keep-artifacts-on-failure`
- `node apps/cli/scripts/live-claude-provider-drill.mjs --scenario selection --timeout-ms 180000 --keep-artifacts-on-failure`
- `node apps/cli/scripts/live-claude-provider-drill.mjs --scenario abort --timeout-ms 180000 --keep-artifacts-on-failure`
- `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario mcp-echo-workflow --providers claude --provider-model claude=sonnet --poll-limit 120 --poll-interval-ms 2000`
- `node apps/cli/scripts/live-claude-provider-drill.mjs --scenario selection --model sonnet --effort low --timeout-ms 180000 --keep-artifacts-on-failure`
- `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario validated-increment-chain --providers claude,codex,opencode --provider-model claude=sonnet --provider-model codex=gpt-5.2 --provider-model opencode=opencode/gpt-5.2 --poll-limit 180 --poll-interval-ms 2000`
- `cargo test --manifest-path apps/kernel/Cargo.toml opencode_backend_catalog --lib`
- `pnpm --filter @chariox/cli run lint`
- `pnpm --filter @chariox/cli run build`
- `node --test apps/cli/dist/provider-catalog.test.js apps/cli/dist/waiting-room.test.js apps/cli/dist/provider-command-catalog.test.js apps/cli/dist/command-center.test.js`
- Live kernel catalog probe returned connected providers
  `["claude","codex","opencode"]`, Claude models
  `["claude-opus-4-7","claude-sonnet-4-6","haiku","opus","sonnet"]`,
  default `sonnet`, and a single OpenCode backend with 40 OpenCode-namespace
  models.
- `cargo test --manifest-path apps/kernel/Cargo.toml local_daemon_protocol --lib`
- `cargo test --manifest-path apps/kernel/Cargo.toml env_relay_config_takes_precedence_over_persisted_cloud_relay_profile`
- `cargo check --manifest-path apps/kernel/Cargo.toml`
- `cargo build --manifest-path apps/kernel/Cargo.toml --bin chariox-kernel`
- `cargo build --manifest-path apps/relay/Cargo.toml --bin chariox-relay`
- `node apps/cli/scripts/live-remote-machine-runtime-drill.mjs --provider claude --provider-model claude=sonnet --timeout-ms 180000`

Known verification gap:

- Claude Code model catalog discovery is best-effort. The subscription CLI path
  exposes aliases through `claude -p --model ...`, but does not expose a stable
  machine-readable model listing command comparable to provider APIs. Chariox
  publishes known aliases/full IDs and also reads Claude Code's
  `additionalModelOptionsCache` / `additionalModelCostsCache` from
  `~/.claude.json` defensively when those caches exist. Cache shape changes must
  be handled by updating the parser and tests.
- OpenCode all-provider validation must use a locally working OpenCode model
  namespace. `openai/gpt-5.2` failed on this machine with `Token refresh failed:
  401`; the same Claude/Codex/OpenCode workflow passed with
  `opencode/gpt-5.2`.
- Remote machine Claude validation now passes through a local scoped relay. The
  remaining remote workflow drill no longer fails at relay discovery; it reaches
  workflow invocation and Claude provider launch, but
  `live-remote-workflow-runtime-drill.mjs --provider claude --provider-model claude=sonnet`
  timed out with the workflow node still `Running` and only prompt echoes
  recorded. That is a workflow/runtime-MCP Claude follow-up, not a relay target
  registration blocker.
- Auth/status UX, larger artifact helper surfaces, native Claude TUI, slice
  support, and workspace live sync remain out of scope for this implementation slice.

Claude Code should become a first-class Chariox provider at the same runtime level
as Codex and OpenCode. The initial integration must use the local Claude Code CLI
and its existing user login/subscription state. It must not use Anthropic SDK/API
key flows for normal Chariox provider runs.

## Architecture Fit

Chariox's existing boundary remains the governing design:

- Clients talk only to the kernel through the existing local/remote kernel
  protocol.
- The kernel owns sessions, agents, prompts, terminal fanout, provider runs,
  workflow state, artifacts, and durable state.
- The provider adapter owns provider-specific launch, stdin/stdout protocol,
  resume identity, permission mapping, event parsing, and auth probing.
- Transport code owns only kernel/client or relay packet movement. Claude Code
  provider behavior must not be added to relay transport or client transport.
- Runtime MCP remains the provider-facing extension/control surface when a
  milestone needs runtime tools.
- The relay remains transport-only and must not inspect Claude prompts, Claude
  output, provider session ids, artifacts, or provider auth state.

Claude Code does not expose the same local HTTP/WebSocket app-server style that
Codex and OpenCode expose. The best fit is a kernel-owned structured stdio
adapter:

```text
Chariox CLI / web / native client
  <-> kernel local or relay-backed protocol
  <-> kernel provider runtime
  <-> Claude adapter
  <-> claude -p --input-format stream-json --output-format stream-json
```

The Claude Code process is an adapter-owned child process. It is not a client
transport, not a relay participant, and not a Cloud concern.

## Explicit Non-Goals

- Do not implement workspace live sync enforcement in the first Claude Code provider
  milestones. Leave it for a later coordinated-I/O hardening milestone.
- Do not require `ANTHROPIC_API_KEY` or Anthropic SDK setup for normal Claude
  provider use.
- Do not add Claude-specific behavior in Chariox CLI prompt dispatch, relay
  routing, or web/native client protocol code except for UI/catalog display of
  kernel-owned provider metadata.
- Do not make Cloud proxy Claude Code runtime traffic.

## Provider Surface

Use the installed `claude` CLI in non-interactive streaming mode:

```text
claude -p \
  --input-format stream-json \
  --output-format stream-json \
  --verbose \
  --include-partial-messages \
  --replay-user-messages
```

Additional launch flags are adapter-owned:

- `--model <model>` for explicit model selection.
- `--effort <level>` for Chariox variant/effort selection.
- `--resume <session-id>` for provider session resume.
- `--permission-mode <mode>` for Chariox execution/permission mapping.
- `--mcp-config <json>` when runtime MCP or artifact tools are bound.
- `--strict-mcp-config` when Chariox needs to prevent unrelated project/user MCP
  inheritance for a specific run.

Normal subscription-backed runs should preserve Claude Code OAuth/keychain
authentication. The adapter should avoid `--bare` for normal runs because bare
mode skips OAuth/keychain reads. The adapter should remove `ANTHROPIC_API_KEY`
from Claude provider process env by default unless a future explicit API-key
profile mode is added.

## Permission And Mode Mapping

Chariox already tracks an agent execution mode and permission level. Claude Code
must receive the closest native permission mode at launch and when turn-scoped
config is supported.

Initial mapping:

| Chariox config | Claude Code mode | Notes |
| --- | --- | --- |
| `execution_mode = Plan` | `--permission-mode plan` | Provider should analyze and present plans without code execution. |
| `execution_mode = Build`, `permission_level = Required` | `--permission-mode default` | Claude asks for native approvals when needed. |
| `execution_mode = Build`, `permission_level = Yolo` | `--permission-mode bypassPermissions` plus `--allow-dangerously-skip-permissions` | Matches current Chariox yolo semantics. Use only in trusted local workspaces. |

If Claude Code supports live control messages for permission/model changes in
the active stream, the adapter can later implement
`supports_turn_scoped_execution_config()`. Until verified, start with launch-time
mapping only and force a provider restart/resume for changed execution config.

Provider-native permission prompts are still surfaced out-of-band. The adapter
must not infer approval state from a missing stdout marker.

## Code Ownership

Planned module boundaries:

- `apps/kernel/src/provider/claude.rs`: executable resolution, launch planning,
  auth commands, static catalog helpers, and env policy.
- `apps/kernel/src/provider/claude_runtime.rs`: child process state, stdin
  submission, stdout/stderr drain, event parser, turn completion, abort, resume
  state, usage/model extraction.
- `apps/kernel/src/provider/run_actor.rs`: dispatch branches for Claude submit,
  abort, terminate, and output polling, matching current Codex/OpenCode actor
  ownership.
- `apps/kernel/src/provider/service.rs`: runtime binding enum and provider-run
  metadata application.
- `apps/kernel/src/provider/types.rs`: Claude resume state, if serialized state
  requires a new field.
- `apps/kernel/src/local/provider_requests.rs`: provider catalog/auth integration
  only.
- `apps/kernel/src/runtime/state/provider_runtime.rs`: generic launch request to
  provider resume-state mapping only.
- `apps/kernel/src/transport/*`: no Claude-specific code.
- `apps/relay/*`: no Claude-specific code.
- `apps/cli/*`: only generic provider display/native launcher work where the
  kernel protocol already requires it.

If adding `claude_session_id` to serialized `ProviderResumeState` changes the
local daemon protocol shape, bump `LOCAL_DAEMON_PROTOCOL_VERSION`, update
protocol snapshot/hash tests, and add a focused drill.

## Milestones

### M13.1 Basic Local Claude Structured Provider

Status: complete for local Chariox-client structured prompt I/O.

Goal: launch Claude Code from the kernel, submit one prompt, stream deltas, and
settle the turn from Claude's terminal result.

Work:

- Add `claude` to the provider registry and catalog.
- Add launch planning for `claude -p --input-format stream-json
  --output-format stream-json`.
- Add Claude runtime state with child process stdin/stdout/stderr ownership.
- Parse `system`, `stream_event`, `assistant`, and `result` messages enough to:
  - capture provider session id,
  - emit assistant text deltas,
  - emit reasoning deltas when present,
  - mark prompt completion,
  - surface terminal failures.
- Add launch-time permission/plan-mode mapping.
- Add unit tests for launch args, env removal, session id capture, text delta
  parsing, and result completion.

Live drills:

- `claude-local-echo`: spawn daemon, launch Claude provider, ask for a short
  deterministic response, verify streamed terminal output and prompt completion.
- `claude-plan-mode`: launch an agent with plan mode, ask for a change plan,
  verify no file changes and a completed turn.
- `claude-permission-required`: launch build/default mode and run a harmless
  prompt that requires file inspection; verify provider-native approval flow does
  not break kernel completion.
- `claude-yolo-smoke`: launch yolo mode in a disposable worktree and verify the
  expected permission mode is used.

### M13.2 Early Artifact Transmission

Status: complete for inline text/base64 attachments and opaque fallback
references. Runtime MCP-backed artifact helpers remain deferred until a later
artifact/tooling hardening slice.

Goal: after basic streaming is proven, support Chariox prompt attachments and
artifact references for Claude turns before remote/native work.

Work:

- Convert Chariox `PromptAttachment` records into Claude stream-json user content
  when supported.
- For local files/artifacts, prefer explicit text blocks or path references that
  preserve kernel artifact ownership.
- Add runtime MCP only as needed for artifact read/download helpers. This is not
  workspace live sync enforcement.
- Ensure artifacts remain kernel-owned and relay-opaque.
- Add tests for text artifact attachment, binary/opaque artifact fallback, and
  missing artifact diagnostics.

Live drills:

- `claude-text-artifact`: attach a small text artifact, ask Claude to summarize
  it, verify summary references artifact contents.
- `claude-file-attachment`: attach a workspace file, ask Claude to identify a
  known marker, verify streamed answer and no client-side file reads.
- `claude-artifact-relay-opaque`: route a remote client through the relay while
  the kernel hosts Claude locally; verify relay traffic remains normal kernel
  encrypted payload traffic and artifact content is not relay-owned.

### M13.3 Resume, Abort, And Selection Updates

Status: complete locally. Remote validation is tracked in M13.5.

Goal: make Claude runs behave like persistent Chariox provider runs.

Work:

- Add `claude_session_id` resume state.
- Support resume by provider session id on provider restart.
- Implement prompt abort:
  - first try Claude stream control interrupt if verified,
  - otherwise terminate and restart with `--resume`.
- Reconcile model/effort from Claude messages when observable.
- Decide whether turn-scoped model/permission updates can use live control or
  need restart/resume.

Live drills:

- `claude-resume`: complete one prompt, terminate provider process, resume by
  provider session id, ask a follow-up that depends on previous context.
- `claude-abort`: start a long-running prompt, cancel through Chariox, verify
  kernel prompt cancellation and provider process recovery.
- `claude-selection-update`: change model/effort through Chariox, verify either
  live application or restart/resume behavior and correct run metadata.

### M13.4 Workflows And Multi-Agent Parity

Status: complete for the local workflow MCP/runtime path. Mixed-provider and
remote workflow drills remain follow-up validation once the remote relay target
bootstrap issue is resolved.

Goal: Claude participates in freeform and workflow runtime at the same level as
Codex/OpenCode, without workspace live sync guarantees yet.

Work:

- Add Claude to workflow provider mapping.
- Verify workflow runtime MCP tools are usable from Claude turns.
- Ensure workflow output validation and ack tools behave like other providers.
- Add Claude to substitute-provider failure rules.
- Add mixed-provider workflow drill coverage.

Live drills:

- `claude-workflow-validated-increment`: single Claude node emits validated
  workflow output.
- `claude-codex-opencode-chain`: three-node workflow, one provider each, with
  structured handoff.
- `claude-workflow-cancel-resume`: cancel a Claude node turn, resume/retry, and
  verify workflow state remains coherent.
- `claude-substitute-after-auth-failure`: force Claude auth/launch failure and
  verify configured substitute activation.

### M13.5 Remote Claude Code

Status: adapter support is expected to follow existing remote provider launch
paths because Claude is now in the provider catalog/runtime stack. Live drills
are currently blocked before provider use by remote relay target bootstrap
failure.

Goal: use Claude Code from remote Chariox clients and remote worker kernels while
preserving the relay architecture.

Work:

- Ensure remote clients can launch and interact with a Claude provider run hosted
  by a kernel exactly as they do with Codex/OpenCode.
- Ensure remote worker kernels can advertise Claude availability through existing
  waiting-room/provider catalog projection.
- Keep provider process and provider auth local to the worker kernel.
- Ensure relay target selection uses heartbeat freshness and never treats stale
  Claude-capable machines as online.
- Add remote failure diagnostics and substitute activation classification for
  Claude launch/auth failures.

Live drills:

- `remote-cli-to-local-claude`: remote TUI attaches through relay to a local
  kernel-hosted Claude run, submits a prompt, receives deltas, and completes.
- `remote-worker-claude`: home kernel dispatches an agent prompt to a remote
  worker kernel with Claude installed/logged in.
- `mixed-remote-providers`: one session with Claude, Codex, and OpenCode runs
  across local/remote kernels; verify prompt routing and provider run metadata.
- `stale-claude-target`: make a Claude-capable worker stale; verify it is not
  selected for provider launch.

### M13.6 Provider Auth And Command Catalog Polish

Goal: Claude appears cleanly in provider status, login/logout, model catalog, and
provider command surfaces.

Work:

- Implement `claude auth status` parsing.
- Implement `claude auth logout`.
- For login, prefer a host-local instruction to run `claude auth login`; only
  launch interactive login when the caller is local and attached.
- Add static Claude command catalog entries where useful.
- Add static model catalog with aliases and full model ids; keep dynamic model
  discovery only if Claude exposes a stable local command.

Live drills:

- `claude-auth-status`: logged-in and logged-out status detection.
- `claude-auth-login-hint`: remote caller receives host-local login guidance,
  not a proxied provider auth flow.
- `claude-logout`: logout invalidates provider catalog cache and status changes.
- `claude-command-catalog`: command center surfaces Claude command entries
  without scraping interactive help.

### M13.7 Native Claude TUI

Status: implemented for the current native TUI contract. Claude Code exposes a
rich interactive TUI, background sessions, `claude attach`, Remote Control, and
hooks, but does not expose the same local provider-server protocol shape that
Codex/OpenCode expose. Chariox therefore uses a kernel-owned PTY plus hook
events rather than a provider app-server proxy.

Goal: add a native Claude Code TUI mode comparable to native Codex/OpenCode TUI
work, while the kernel remains session authority.

Work:

- Add `chariox claude [session-ref]` native launcher.
- Mark runs with `client_interface = native_tui`.
- Ensure Chariox clients treat model/effort controls as provider-controlled for
  native TUI runs.
- Validate the hook-assisted interactive architecture:
  - launch `claude` in a Chariox-owned PTY with a temporary settings overlay;
  - register `UserPromptSubmit`, `Stop`, `StopFailure`, `PermissionRequest`,
    `PreToolUse`, `PostToolUse`, `ConfigChange`, and `SessionEnd` hooks;
  - use `UserPromptSubmit` to observe native prompts and add Chariox hidden
    prompt context through `additionalContext`, not visible plain prompt text;
  - use `Stop`/`StopFailure` and transcript tailing to settle Chariox turn state;
  - use `PermissionRequest`/`PreToolUse` to surface native approval state to
    Chariox when possible.
- Validate Chariox-client prompt injection into a native Claude TUI:
  - first attempt PTY input injection while the TUI is idle;
  - separately investigate whether background sessions expose a stable local
    reply API beyond `claude attach`;
  - do not rely on Remote Control for Chariox runtime traffic, because it routes
    through Anthropic services and is not a Chariox relay/kernel path.
- Treat `claude -p --input-format stream-json --output-format stream-json` as
  the structured Chariox provider path, not the native TUI path.
- Preserve one provider run per native TUI agent.

Live drills:

- `native-claude-new-session`: start native Claude TUI from Chariox, verify kernel
  creates session, agent, provider run, and output fanout.
- `native-claude-existing-session`: attach native Claude TUI as a new top-level
  agent in an existing session.
- `native-claude-hook-observation`: submit from the Claude TUI, verify
  `UserPromptSubmit` reaches Chariox, the prompt appears in Chariox history, and
  `Stop` returns the badge to idle.
- `native-claude-hidden-context`: verify Chariox prompt injection is delivered
  through hook `additionalContext` or launch prompt flags without appearing as
  visible prompt text in the native TUI.
- `native-claude-chariox-client-prompt`: submit from Chariox client and verify
  the native Claude side receives the prompt and the full response turn without
  corrupting the TUI input state.
- `native-claude-permission`: trigger a native Claude permission prompt, verify
  the native TUI behaves as raw Claude Code, and verify Chariox can observe or
  proxy the approval state when the hook data supports it.
- `native-claude-attachments`: validate native TUI attachments and Chariox client
  attachments separately; expect Chariox-to-native binary/image injection to need
  file-path materialization unless Claude exposes a running-session attachment
  API.
- `native-claude-detach-reattach`: detach/reconnect client without losing kernel
  provider-run state.

Research notes:

- Claude Code interactive TUI has no documented local app-server or
  provider-owned websocket endpoint equivalent to `codex app-server` or
  `opencode serve`.
- Claude Code hooks are the strongest integration seam for native TUI mode:
  `UserPromptSubmit` exposes submitted prompt text before processing and can add
  hidden context; `Stop`/`StopFailure` mark turn completion; tool and permission
  hooks expose provider-native execution decisions.
- Background sessions and `claude attach` prove Claude Code can detach and
  reattach a native TUI to a provider-owned local session, but the documented
  shell surface exposes attach/log/stop/respawn/remove, not a stable
  machine-to-session prompt API.
- Remote Control proves Claude Code can split controller and local execution,
  but routes through Anthropic infrastructure. It is not a Chariox runtime
  transport and should not be used for kernel/relay session traffic.

### M13.8 Slice Support

Goal: support Claude in slice workflows without moving provider authority out of
the kernel.

Work:

- Define how Claude provider runs are selected for slice execution.
- Ensure slice display endpoints and generated workspace views remain kernel
  records, not Claude adapter records.
- Support artifact/context transmission into slice Claude turns.
- Verify mixed Claude/Codex/OpenCode slice runs preserve provider boundaries.

Live drills:

- `claude-slice-basic`: create a slice, dispatch Claude, verify slice output and
  display endpoint metadata.
- `claude-slice-artifact`: attach slice artifact context to Claude and verify
  streamed answer.
- `mixed-provider-slice`: run Claude/Codex/OpenCode in one slice scenario and
  verify no provider-specific transport assumptions leak into slice code.

### M13.9 Workspace live sync Hardening Later

Goal: only after Claude local/remote/workflow/native/slice paths are stable,
bring Claude into workspace live sync enforcement.

Work:

- Apply the macOS workspace write fence to Claude child processes.
- Deny or restrict native write tools where Claude exposes stable controls.
- Bind Workspace Live Sync managed tools through runtime MCP.
- Verify direct native writes fail and Chariox Workspace Live Sync artifact writes succeed.

Live drills:

- `claude-workspace-live-sync-text`: managed read/write/edit artifact workflow.
- `claude-workspace-live-sync-native-write-denied`: direct file write attempt leaves no
  forbidden workspace mutation.
- `mixed-workspace-live-sync-all-providers`: Claude, Codex, and OpenCode complete the
  workspace live sync drill in one run.

## Implementation Checkpoints

Commit and push after each meaningful improvement:

- planning doc landed,
- adapter registration and launch planning,
- basic stream parser and runtime binding,
- first local live drill passing,
- artifact transmission,
- resume/abort,
- remote Claude,
- workflow parity,
- auth/catalog polish,
- native TUI,
- slice support,
- later workspace live sync.

Every functional milestone must add or extend at least one live drill before the
milestone is considered complete.
