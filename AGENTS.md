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
- For browser relay runtime, require direct `wss://` browser-to-relay connectivity in hosted environments.
- Use heartbeat freshness for relay target selection; stale targets must not be treated as online.
- Preserve local/dev/self-host compatibility where practical, but fail loudly when hosted Cloud configuration violates the runtime architecture.
