# M14 Split Native TUI Plan

## Status

Planned. This milestone captures the deferred work to split provider-native TUI
control from provider-server execution after the local native Codex/OpenCode TUI
spikes proved the value of provider-native UX under Arroba session authority.

## Goal

Let users keep the provider TUI they already know while Arroba keeps its runtime
advantages: shared sessions, remote machines, relay attachment, history,
workflows, vault, artifacts, and multi-client observation.

The key capability is to run the provider server near the execution environment
and the provider TUI near the user:

```text
native provider TUI
  <-> local Arroba native-TUI proxy
  <-> local kernel
  <-> relay
  <-> remote or slice kernel
  <-> Arroba provider-server proxy
  <-> provider server/app-server
```

This is intentionally different from exposing provider server ports directly.
Provider endpoints stay loopback/private on the machine that owns provider
credentials, workspace access, MCP configuration, and tool execution.

## Naming

Working name: **Split Native TUI**.

User-facing commands should start explicit and can later gain sugar:

```text
arroba codex-server --session <session> --agent <agent>
arroba codex-tui --session <session> --agent <agent> --server-kernel <kernel-ref>
arroba opencode-server --session <session> --agent <agent>
arroba opencode-tui --session <session> --agent <agent> --server-kernel <kernel-ref>
```

Later shorthand can be added once the architecture is stable:

```text
arroba codex <session> --server slice:<name>
arroba opencode <session> --server machine:<name>
```

## Architecture

Split the current native-TUI wrapper into explicit roles:

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

Provider endpoints must remain private to the owning machine. Relay remains
transport-only and must not inspect provider frames, prompts, outputs, artifacts,
or provider credentials.

## Prompt And UI Policy

- Prompt injection is applied only on the server-bound/kernel-bound path.
- Provider TUIs receive redacted user-visible traffic.
- Arroba-side parameter editing is disabled for Split Native TUI agents.
- Provider-native parameter changes can be observed and recorded when the
  provider protocol exposes them, but Arroba should not present competing
  controls.
- Status badges derive from Arroba agent turn state, not from provider TUI
  process liveness.

## Implementation Plan

1. Add a provider-native bridge abstraction below clients and above provider
   adapters.
2. Add Codex split roles:
   - server role starts `codex app-server`;
   - TUI role starts `codex --remote`;
   - bridge tunnels JSON-RPC frames and preserves hidden-prompt redaction.
3. Add OpenCode split roles:
   - server role starts `opencode serve`;
   - TUI role starts `opencode attach`;
   - bridge tunnels HTTP and streaming responses.
4. Integrate split roles with provider runs and active Arroba agents.
5. Disable Arroba-side provider parameter controls for split/native-TUI agents.
6. Add local, remote, slice, and workflow live drills.
7. Clean up spike-only native-TUI code after the split bridge is proven.

Any serialized protocol shape added for bridge open/send/close/health must bump
`LOCAL_DAEMON_PROTOCOL_VERSION`, update protocol snapshot/hash tests, and add a
focused drill.

## Live Drills

- `codex-split-local`: local TUI and local provider server, prompts both ways,
  full response visible in native TUI and Arroba CLI, badge returns to idle,
  hidden prompt not visible in native TUI logs.
- `codex-split-remote`: local TUI with provider server on remote/slice worker;
  verify tool/file execution occurs on the server machine.
- `codex-split-two-agents`: two Codex native TUI agents in one Arroba session,
  no prompt/output cross-contamination, independent badge transitions.
- `opencode-split-local`: same local drill using `opencode attach` and
  `opencode serve`.
- `opencode-split-remote`: local OpenCode TUI with server on remote/slice worker,
  including streaming output and attachment materialization on the server side.
- `split-native-workflow`: workflow with one local native agent and one
  split-native remote/slice agent; run workflow scenarios outside publication.
- `split-native-permissions`: native provider permission prompt remains usable
  and Arroba state remains coherent.
- `split-native-attachments`: attachments from Arroba client and provider TUI
  route to the provider-server machine with supported MIME handling.
- `split-native-provider-commands`: provider-native commands from the provider
  TUI execute normally; Arroba clients either do not expose them or return a
  clear unsupported error.
- `split-native-failure`: kill TUI, server, and relay during turns and verify
  no silent prompt loss or session corruption.

## Open Questions

- Whether Claude Code can join this milestone through background sessions,
  remote-control surfaces, or a future provider-local attach protocol.
- Whether provider-server and provider-TUI roles need independent kernel
  identities for multi-machine auditing.
- Whether bridge frames should be generic byte streams or provider-shaped
  protocol envelopes from the start.
