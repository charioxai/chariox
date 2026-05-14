# M14 Split Native TUI Plan

## Status

In progress. The first implementation slice removed the normal attached Claude
Code native TUI path's dependency on GNU `screen` by running it through a direct
PTY helper, while keeping `--detached-screen` as a drill-only mode. The Claude
native TUI drill passes with two native TUIs plus one Arroba observer CLI,
bidirectional prompt/response observation, and `IDLE -> working tone -> IDLE`
badge transitions.

The second implementation slice added protocol v32 `SendTerminalInput`, a
session/attachment-scoped base64 PTY input request. The third slice bumped the
protocol to v33 so terminal input can optionally target a provider run directly;
this is required for multiple remote-rendered native PTYs in one Arroba session.

Current M14 implementation status:

- Claude Code has a kernel-owned `native_tui` managed PTY launch path plus
  `arroba claude --remote-rendered`, which streams the target kernel PTY locally,
  forwards input/resize, records native prompts through Claude hooks without
  re-dispatching them, injects Arroba-origin prompts through the PTY with hidden
  context in `UserPromptSubmit`, and completes active turns from Claude stop
  hooks.
- Codex and OpenCode wrappers have an internal `--server-in-kernel` mode that
  launches the provider server through the target kernel-owned provider run and
  attaches the local native TUI through the existing local proxy. This validates
  the provider-server ownership boundary on same-host/relay drills. The remaining
  product gap for true different-machine split mode is the private loopback
  tunnel between the TUI-side proxy and the server-side provider endpoint.
- On 2026-05-14, `node apps/cli/scripts/live-remote-native-tui-drill.mjs
  --providers opencode,codex,claude --keep-artifacts-on-failure` passed. The
  drill launches two native TUIs plus one Arroba observer CLI in one Arroba
  session for each provider, validates prompts from Arroba and native TUIs,
  validates native output visibility without cross-agent contamination, and
  verifies native-agent badges transition from idle to working/thinking and
  back to idle.

This milestone captures the remaining work to split or remotely render
provider-native TUIs after the local native Codex, OpenCode, and Claude Code TUI
paths proved the value of provider-native UX under Arroba session authority.

## Goal

Let users keep the provider TUI they already know while Arroba keeps its runtime
advantages: shared sessions, remote machines, relay attachment, history,
workflows, vault, artifacts, and multi-client observation.

The ideal capability is to run the provider server near the execution
environment and the provider TUI near the user:

```text
native provider TUI
  <-> local Arroba native-TUI proxy
  <-> local kernel
  <-> relay
  <-> remote or slice kernel
  <-> Arroba provider-server proxy
  <-> provider server/app-server
```

Codex and OpenCode can use this literal split because they expose local
structured provider transports that can be proxied. Claude Code does not expose a
documented local app-server/native-TUI attach protocol. For Claude Code, the
milestone target is therefore functional parity through a remotely rendered PTY:

```text
local Arroba TUI
  <-> local kernel
  <-> relay
  <-> remote or slice kernel
  <-> kernel-owned PTY running Claude Code native TUI + Arroba hooks
```

The Claude Code TUI runs on the execution machine, but the local user sees and
controls that native TUI through an Arroba PTY stream. This is equivalent to
running Claude Code over SSH, except Arroba owns session linkage, hidden prompt
injection, history, artifacts, workflow scheduling, and multi-client
observation.

This is intentionally different from exposing provider server ports directly.
Provider endpoints stay loopback/private on the machine that owns provider
credentials, workspace access, MCP configuration, and tool execution.

## Naming

Working umbrella name: **Distributed Native TUI**.

Provider-specific submodes:

- **Split Native TUI**: provider TUI and provider server are separate processes
  connected through an Arroba bridge. Applies to Codex and OpenCode.
- **Remote-Rendered Native TUI**: provider TUI runs on the execution machine in
  a kernel-owned PTY and is streamed to the user. Applies to Claude Code until a
  supported provider attach/server protocol exists.

User-facing commands should start explicit and can later gain sugar:

```text
arroba codex-server --session <session> --agent <agent>
arroba codex-tui --session <session> --agent <agent> --server-kernel <kernel-ref>
arroba opencode-server --session <session> --agent <agent>
arroba opencode-tui --session <session> --agent <agent> --server-kernel <kernel-ref>
arroba claude-pty --session <session> --agent <agent> --machine <machine-ref>
```

Later shorthand can be added once the architecture is stable:

```text
arroba codex <session> --server slice:<name>
arroba opencode <session> --server machine:<name>
arroba claude <session> --machine <name>
```

## Architecture

Split the current native-TUI wrappers into explicit roles where the provider
supports it:

- `native-tui`: launches the local provider TUI and exposes a local loopback
  endpoint for it.
- `native-server`: launches the provider server/app-server near the workspace
  and registers the Arroba provider run for the target session/agent.
- `native-bridge`: tunnels provider protocol traffic through kernel/relay
  between the TUI-side proxy and server-side proxy.
- Kernel remains the session, agent, provider-run, workflow, history, artifact,
  and status authority.

Provider-specific transport:

- Codex: tunnel websocket JSON-RPC frames between `codex --remote` and
  `codex app-server`.
- OpenCode: tunnel HTTP requests/responses and streaming bodies between
  `opencode attach` and `opencode serve`.
- Claude Code: do not tunnel an unsupported provider protocol. Launch Claude
  Code in a kernel-owned PTY on the execution machine, install Arroba hook
  settings there, stream PTY output/input through kernel/relay, and use hooks
  plus transcript observation for prompts, output, hidden prompt context, and
  permission mapping.

Provider endpoints must remain private to the owning machine. Relay remains
transport-only and must not inspect provider frames, prompts, outputs, artifacts,
or provider credentials.

Remote-rendered PTY requirements:

- The remote machine does not need GNU `screen`, tmux, or a graphical display.
- The worker kernel owns the PTY directly, using the same class of pseudo-
  terminal support as other kernel PTY flows.
- The worker sets `TERM=xterm-256color` or equivalent, propagates rows/columns
  from the local client, streams output, accepts input, and handles resize.
- `screen` remains acceptable for local drills, but product code should not
  require it.

## Prompt And UI Policy

- Prompt injection is applied only on the server-bound/kernel-bound path.
- Provider TUIs receive redacted user-visible traffic.
- Claude Code receives hidden Arroba instructions through the `UserPromptSubmit`
  hook `additionalContext`; only redacted prompt text is typed into the PTY.
- Arroba-side parameter editing is disabled for Split Native TUI agents.
- Provider-native parameter changes can be observed and recorded when the
  provider protocol exposes them, but Arroba should not present competing
  controls.
- Status badges derive from Arroba agent turn state, not from provider TUI
  process liveness.

Artifact policy:

- Arroba-origin attachments are materialized on the provider-server or
  remote-rendered PTY machine before the prompt is delivered to the provider.
- Prompt text sent to the provider TUI is rewritten to reference machine-local
  artifact paths that the provider can read.
- Provider-native attachments are captured when the provider protocol or
  transcript exposes enough metadata. For Claude Code this must be validated
  separately because native prompt attachments may appear only as transcript
  content or local file references.
- Managed-I/O guarantees apply only when Arroba launches the provider execution
  process behind the relevant runtime boundary. Externally launched provider
  endpoints remain outside the managed-runtime guarantee.

## Implementation Plan

1. Add a provider-native transport abstraction below clients and above provider
   adapters.
2. Add a generic kernel/relay tunnel for provider-local byte streams and
   provider-shaped bridge frames.
3. Add a generic remote PTY streaming surface:
   - open/close PTY session;
   - send input;
   - receive output;
   - resize;
   - report exit/liveness;
   - bind PTY to session/agent/provider-run ownership.
4. Add artifact materialization for native-provider delivery:
   - home artifact to worker artifact transfer;
   - stable worker-local session artifact paths;
   - prompt reference rewriting;
   - MIME metadata preservation where supported.
5. Add Codex split roles:
   - server role starts `codex app-server`;
   - TUI role starts `codex --remote`;
   - bridge tunnels JSON-RPC frames and preserves hidden-prompt redaction.
6. Add OpenCode split roles:
   - server role starts `opencode serve`;
   - TUI role starts `opencode attach`;
   - bridge tunnels HTTP and streaming responses.
7. Add Claude remote-rendered native TUI:
   - worker role starts Claude Code in a kernel-owned PTY;
   - worker role writes temporary hook settings and context files;
   - local role streams PTY output and user input;
   - hook events submit native-origin prompts to the home session;
   - Arroba-origin prompts write hidden context and inject visible text into the
     PTY;
   - transcript observation appends provider output and completes turns.
8. Integrate split/remote-rendered roles with provider runs and active Arroba
   agents.
9. Disable Arroba-side provider parameter controls for distributed native-TUI
   agents.
10. Add local, relay, remote-machine, slice, artifact, permission, workflow, and
    failure live drills.
11. Clean up spike-only native-TUI code after the bridge and PTY paths are
    proven.

Any serialized protocol shape added for bridge open/send/close/health must bump
`LOCAL_DAEMON_PROTOCOL_VERSION`, update protocol snapshot/hash tests, and add a
focused drill.

## Live Drills

Every provider must pass the common behavioral matrix unless a limitation is
explicitly documented in the provider notes below.

Common assertions:

- one distributed native-TUI agent can create a new Arroba session;
- `arroba <provider> <session>` can attach a second distributed native-TUI agent
  to that session;
- one observer Arroba CLI sees both agents;
- prompts submitted from provider-native UI are visible in Arroba history and
  observer CLI;
- prompts submitted from Arroba CLI are visible to the provider-native UI;
- full provider responses reach both Arroba history and observer CLI;
- no cross-agent prompt/output contamination with two agents;
- agent footer badges move `IDLE -> working/thinking tone -> IDLE`;
- hidden Arroba instructions reach the provider server/runtime and are not
  visible in provider-native UI output;
- native-TUI agents reject Arroba-side provider/model/permission mutation with a
  clear provider-controlled message;
- session cleanup leaves no stale provider runs, PTYs, tunnel sessions, or
  temporary artifact roots.

### Codex

- `codex-split-local-basic`: local TUI and local app-server through the bridge,
  prompts both ways, hidden prompt redaction, badge transitions.
- `codex-split-local-two-agents`: two Codex split agents in one Arroba session,
  no contamination, independent app-server threads.
- `codex-split-relay-same-host`: native TUI connects through relay to the home
  kernel while provider app-server stays local to the kernel. **Pass on
  2026-05-14** via `live-remote-native-tui-drill.mjs --providers codex`.
- `codex-split-remote-machine`: local TUI, provider app-server on a remote
  worker kernel; verify shell/tool/file execution occurs on the worker.
- `codex-split-slice`: local TUI, provider app-server inside a slice; verify
  slice workspace and history attribution.
- `codex-split-attachments`: Arroba-origin image/file attachment materializes on
  the server machine and reaches Codex; Codex-native attachment metadata is
  captured when emitted by the native protocol.
- `codex-split-permissions`: provider-native shell/edit approval appears in
  Arroba clients, can be answered from Arroba, and the native TUI remains
  coherent.
- `codex-split-provider-commands`: native `thread/*` commands from Codex TUI
  tunnel to app-server; Arroba CLI command surface remains intentionally
  unsupported or explicitly routed.
- `codex-split-workflow`: workflow with one local native Codex and one split
  remote/slice Codex agent; run the workflow validation scenarios outside
  publication.
- `codex-split-reconnect`: restart relay and local TUI bridge during idle and
  during a turn; verify no silent prompt loss.
- `codex-split-server-crash`: kill app-server during idle and during a turn;
  verify provider-run state, prompt settlement, and user-facing error.

### OpenCode

- `opencode-split-local-basic`: local TUI and local `opencode serve` through the
  bridge, prompts both ways, hidden prompt redaction, badge transitions.
- `opencode-split-local-two-agents`: two OpenCode split agents in one Arroba
  session, independent provider sessions and no contamination.
- `opencode-split-relay-same-host`: native TUI connects through relay while
  provider server remains local to the home kernel. **Pass on 2026-05-14** via
  `live-remote-native-tui-drill.mjs --providers opencode`.
- `opencode-split-remote-machine`: local TUI, `opencode serve` on a remote
  worker kernel; verify shell/tool/file execution occurs on the worker.
- `opencode-split-slice`: local TUI, provider server inside a slice; verify
  slice workspace and history attribution.
- `opencode-split-streaming`: long streaming response crosses the tunnel without
  chunk loss or premature turn completion.
- `opencode-split-attachments`: Arroba-origin file/image attachment materializes
  on the server machine and reaches OpenCode; provider-native attachment
  metadata is captured when emitted by OpenCode HTTP/SSE traffic.
- `opencode-split-permissions`: native permission prompt maps to Arroba
  interactions and response routes back to OpenCode.
- `opencode-split-provider-commands`: provider-native commands submitted from
  OpenCode TUI execute normally on `opencode serve`.
- `opencode-split-workflow`: workflow with one local native OpenCode and one
  split remote/slice OpenCode agent; run workflow validation scenarios outside
  publication.
- `opencode-split-reconnect`: restart relay and TUI bridge during idle and
  during a turn; verify prompt and event reconciliation.
- `opencode-split-server-crash`: kill `opencode serve` during idle and during a
  turn; verify clear failure state and no session corruption.

### Claude Code

- `claude-remote-rendered-local-basic`: Claude Code runs in a kernel-owned PTY
  on the local machine without GNU `screen`; local Arroba client streams and
  controls the PTY; prompts both ways, hidden prompt redaction, badge
  transitions.
- `claude-remote-rendered-local-two-agents`: two Claude Code PTYs in one Arroba
  session, no prompt/output contamination, independent transcript offsets.
- `claude-remote-rendered-relay-same-host`: local Arroba client connects through
  relay to a kernel-owned Claude PTY on the same host. **Pass on 2026-05-14**
  via `live-remote-native-tui-drill.mjs --providers claude`.
- `claude-remote-rendered-remote-machine`: local Arroba client controls Claude
  Code running on a remote worker kernel; verify shell/tool/file execution
  occurs on the worker.
- `claude-remote-rendered-slice`: local Arroba client controls Claude Code
  running in a slice PTY; verify slice workspace, transcript, and history
  attribution.
- `claude-remote-rendered-headless`: run on a host with no `screen`, tmux, or
  graphical display; verify PTY allocation, TERM, resize, input, output, and
  exit handling.
- `claude-remote-rendered-prompt-injection`: hidden Arroba instructions are
  delivered through `UserPromptSubmit` `additionalContext` and do not appear in
  visible PTY output or native prompt text.
- `claude-remote-rendered-attachments-arroba-origin`: Arroba-origin attachments
  transfer to the worker and prompt references are rewritten to worker-local
  paths before injection into the PTY.
- `claude-remote-rendered-attachments-native-origin`: user adds a file/image
  through the Claude native TUI; validate whether Claude transcript/hooks expose
  enough metadata to register the artifact with Arroba. If not, document the
  limitation and fail with a clear unsupported state.
- `claude-remote-rendered-permissions-native-only`: provider permission prompt
  appears and can be answered in the streamed native TUI; Arroba state remains
  coherent.
- `claude-remote-rendered-permissions-arroba-bridge`: if Claude hook output can
  answer permission events reliably, map the permission request to Arroba
  interaction UI and route the answer back through the hook. If not, keep the
  native-only permission contract and document the gap.
- `claude-remote-rendered-provider-commands`: slash/provider commands typed in
  the streamed Claude TUI execute normally; Arroba CLI does not expose competing
  Claude-native commands unless a stable command protocol appears.
- `claude-remote-rendered-workflow`: workflow with one local native Claude and
  one remote-rendered Claude agent; run workflow validation scenarios outside
  publication.
- `claude-remote-rendered-reconnect`: reconnect local Arroba PTY viewer during
  idle and during a turn; verify PTY/session state does not lose prompt
  settlement.
- `claude-remote-rendered-worker-crash`: kill Claude process and worker kernel
  during idle and during a turn; verify provider-run failure, prompt settlement,
  and reconnect UX.

### Cross-Provider

- `distributed-native-mixed-session`: one Codex split agent, one OpenCode split
  agent, and one Claude remote-rendered agent in one Arroba session; targeted
  prompts, no contamination, independent badge transitions.
- `distributed-native-mixed-workflow`: workflow graph spanning Codex, OpenCode,
  and Claude distributed native-TUI agents; validate node outputs and final
  workflow result outside publication.
- `distributed-native-artifact-roundtrip`: one Arroba-origin attachment is sent
  to all three providers, and any provider-native artifact metadata that can be
  observed is registered back into Arroba history.
- `distributed-native-relay-recovery`: restart relay with mixed distributed
  native agents attached; verify clients reconnect and no provider turn is
  silently duplicated.
- `distributed-native-history-restart`: restart home kernel after completed
  distributed native turns; verify history, provider-run records, agent
  identities, and artifact records restore.

## Open Questions

- Whether Claude Code will expose a supported local attach/server protocol in
  the future. Until then, Claude parity is functional through remote-rendered
  PTY, not architectural split parity.
- Whether provider-server and provider-TUI roles need independent kernel
  identities for multi-machine auditing.
- Whether bridge frames should be generic byte streams or provider-shaped
  protocol envelopes from the start.
- Whether Claude native-origin attachments expose enough stable metadata to
  register them as Arroba artifacts.
- Whether Claude permission hook output is stable enough for Arroba-side
  approval responses, or whether Claude permissions must remain native-TUI-only
  in distributed mode.
