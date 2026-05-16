# AGENTS.md

## Project Map

Arroba spans two main repositories:

- `arroba`: open-source runtime. It owns the kernel, relay, local/remote TUI CLIs, provider adapters, iOS work, and planned Android client foundations.
- `arroba-cloud`: hosted control plane and web app. It owns browser auth, Cloud waiting room, hosted relay token issuance, staging deployment, and the web CLI UI.

## Runtime Architecture

The kernel is the runtime authority. It owns sessions, agents, provider runs, workspaces, worktrees, prompt history, terminal events, and state transitions.

Agents run through provider CLIs or adapters launched by the kernel. Arroba orchestrates them but must not become the provider credential store or provider-internal state owner.

The relay is transport only. It admits scoped connections and routes encrypted packets. It must not become a session authority and must not inspect prompts, outputs, workspace data, provider payloads, or session history.

Primary connectivity:

- Local TUI CLI: `local TUI <-> kernel <-> agent`
- Remote/orphaned TUI CLI: `remote TUI <-> relay <-> kernel <-> agent`
- Web CLI: `browser <-> hosted relay <-> kernel <-> agent`; Cloud is bootstrap/control plane only and must not proxy runtime terminal traffic.
- iOS client: native client planned to use the same kernel/relay protocol surfaces.
- Android client: planned, same architecture as iOS.

Cloud may authenticate users, issue relay tokens, select relay targets, and display waiting-room/control-plane state. Cloud must not fork kernel runtime behavior.

## Native Provider TUI Contract

Native provider TUI mode (`arroba codex`, `arroba opencode`, `arroba claude`) must reuse the normal Arroba runtime paths. For local native TUI, provider prompts enter the kernel through the same prompt path as Arroba clients, and Arroba-origin prompts go through the kernel-managed provider run so the provider TUI observes the same turns. For remote native TUI, provider TUIs and Arroba TUIs attach to the home kernel session; the home kernel dispatches to the worker through the existing leased-agent relay protocol; the worker kernel uses the existing provider adapter/server path. For slice-backed native TUI, provider TUIs still attach to the home kernel session; the slice is only the home-managed worker execution environment selected by `slice_ref`. Do not add a parallel prompt, permission, attachment, history, or relay authority path for native TUIs. See `docs/PROTOCOL.md` section `3.3.2 Native TUI Agents` and `docs/ARCHITECTURE.md` section `5.3.1 Native TUI Client Interface`.

For native TUI MCP/skills, keep standard home-worker and slice behavior distinct. Standard home-worker does not install or copy MCPs/skills across machines; the user/operator must make matching capabilities available on the worker. Slice-backed native TUI may transfer home skill packages to the child worker because the home kernel manages that execution environment. See `docs/PROTOCOL.md` section `3.3.2 Native TUI Agents` and `docs/M14B_NATIVE_TUI_VALIDATION_PLAN.md` for the current validation matrix.

Claude native TUI hidden prompt context must use the `UserPromptSubmit` hook `additionalContext` bridge, not visible PTY prompt injection; see `docs/PROTOCOL.md` section `3.3.2 Native TUI Agents`.

Native TUI permission prompts must resolve through one kernel-owned `RuntimeInteraction` projected to every Arroba TUI in the session; provider-native approval replies should route back to that interaction when the provider seam allows it.

## Protocol Change Rule

When changing `LocalDaemonRequest`, `LocalDaemonResponse`, relay terminal events, browser/kernel terminal transport semantics, or any serialized protocol shape that a CLI or app depends on:

1. Increment the shared local daemon protocol version in OSS.
2. Update protocol snapshot/hash tests so CI fails if the protocol shape changes without a version bump.
3. Update the web/native minimum supported protocol version only when that client depends on the new behavior.
4. Add or update a focused drill that exercises the changed protocol behavior.

Do not merge protocol shape changes without the version bump and test update.

## Implementation Rules

- Keep core behavior below clients, in kernel services and shared protocol contracts.
- Do not implement behavior only in the web app or only in the TUI unless explicitly marked temporary.
- Prefer one shared protocol path across local TUI, remote TUI, web, and native clients.
- Hosted Cloud relay runtime should use the Caddy-fronted `wss://` relay URL for browser, kernel, remote TUI, and kernel-to-kernel remote-agent connections. Local and self-hosted relay setups may keep using `ws://`.
- Use heartbeat freshness for relay target selection; stale targets must not be treated as online.
- Preserve local/dev/self-host compatibility where practical, but fail loudly when hosted Cloud configuration violates the runtime architecture.
- Be lean, don't over engineer and delete all old/unnecessary code along the way.
- Always clean up temporary drill artifacts, orphaned provider processes, and large build outputs you no longer need before handing work back.

## Provider-Native Permission Visibility

Native provider permission prompts are surfaced to the user out-of-band through Arroba runtime interactions. Do not infer that no approval prompt appeared just because a shell/tool result lacks `approval requested` or `approved` metadata. The result visible to the agent normally contains only the provider tool execution outcome, such as stdout/stderr, exit code, and status after the user has already answered the prompt.
