# Arroba Roadmap

## Status

Planning roadmap derived from `docs/spec-v1.md`.

## 1. Roadmap Goals

- deliver a stable daemon-centered v1 runtime
- ship a fully functioning local daemon + CLI path before expanding orchestration breadth
- preserve native provider UX while adding orchestration value
- keep server lightweight with strict security boundaries
- keep the runtime compatible with multi-agent workflow execution, structured handoffs, and isolated branch worktrees

## 2. Milestone Structure

- M0: Foundations
- M1: Core Session Runtime
- M2: End-to-End Local OpenCode Baseline
- M3: Local Capability Surface and Provider Expansion
- M4: Multi-Agent Workflow Runtime
- M5: Remote Access and Web Surfaces
- M6: Provider Switching, Memory, and Agent Extensions
- M7: v1 Stabilization and Launch

## 2.1 Workflow Rollout Within v1

Multi-agent workflow execution remains part of v1, but it is no longer the immediate implementation priority.

Rollout priority:

- first deliver a single-agent local daemon + CLI path with one provider and live terminal streaming
- then expand local capability surface and additional providers
- then deliver workflow runtime foundations and concrete multi-agent execution
- hierarchical topology remains later than circular topology inside the workflow milestone

## 3. Milestones

## M0 - Foundations

Status:

- completed on 2026-03-16
- delivered workspace scaffolding, baseline CI, shared domain types, Prisma schema baseline, and Rust daemon/server bootstrap packages

Outcomes:

- monorepo/workspace setup and baseline CI
- initial shared domain model for user/machine/daemon/session/provider run
- developer docs baseline (`spec-v1`, architecture, protocol, roadmap)

Exit criteria:

- repository can build/lint/test baseline packages
- docs provide a coherent implementation target

## M1 - Core Session Runtime

Outcomes:

- daemon process lifecycle
- session lifecycle (create/attach/detach/end)
- PTY manager for provider runs
- multi-attachment support with daemon-owned prompt queueing and canonical config propagation
- parked provider run support with one active run at a time
- workflow-compatible runtime foundations so later multi-agent graph execution can be added without redesigning session/provider/worktree ownership

Exit criteria:

- local client can run native provider in managed session
- multiple clients can attach without breaking terminal behavior
- implemented runtime surfaces do not block later workflow graph execution, node-scoped provider runs, or explicit worktree assignment

## M2 - End-to-End Local OpenCode Baseline

Status:

- completed on 2026-03-18
- delivered a real local daemon IPC path, a minimal local CLI, a real `opencode` adapter, and end-to-end prompt/output streaming through the daemon
- first iteration continues to exclude provider login flows because OpenCode can run without login by default

Outcomes:

- OpenCode provider adapter wired through the daemon
- local CLI client attached to daemon-managed sessions
- single-agent prompt submission from an input field
- live terminal/output streaming from the active OpenCode run back into the CLI as output appears
- stable session creation/attach/launch/prompt/output loop for one local user on one machine
- no workflow mode, no remote relay, no web app, and no provider switching in this milestone

Exit criteria:

- local CLI can create or attach to a session, launch OpenCode, submit a prompt, and stream output in real time
- daemon remains the authority for session and PTY/provider-run lifecycle during that flow

## M3 - Local Capability Surface and Provider Expansion

Status:

- next implementation priority after M2 closure
- currently in the late OpenCode stabilization, TypeScript CLI hardening, and persistent-session-management phase
- transcript code-highlighting is now part of this same M3 stabilization phase, using terminal-native markdown/code rendering rather than LSP semantic coloring
- explicit persistent session management now exists: delete semantics, no-session CLI state, and session id/alias resolution are in the baseline
- richer OpenCode event rendering is now in the baseline too
- next on the TypeScript CLI path is broader client-side integration coverage, then slash-command dispatch and capability wiring
- slash-command dispatch and capability wiring follow after that client/runtime path is solid

Outcomes:

- upgrade OpenCode from the M2 PTY bootstrap path to a structured local server/session/event adapter
- add a project-wide logging/debugging system with one machine-local log root and correlated logs across daemon and client processes
- add explicit persistent session management with daemon-owned delete semantics and a reusable unattached CLI state
- add syntax-highlighted markdown/code rendering in the TypeScript CLI transcript for provider responses
- replace numeric session ids with commit-like ids and optional aliases, including unique-prefix resolution
- harden the primary TypeScript local CLI after the Rust-to-TypeScript client migration
- shell command capability
- directory tree + file view/edit capabilities
- screenshot capture capability
- git/worktree inspection capability
- file transfer + attach-transferred workflow
- daemon-owned slash-command dispatch for Arroba capabilities
- Claude Code and Codex provider support after the OpenCode baseline is solid
- workflow-compatible local capability design so later multi-agent execution can reuse the same surfaces

Exit criteria:

- capability failures remain isolated from the terminal lane
- multiple supported providers can run through the same daemon-managed local CLI model
- local slash-command UX is usable enough to drive capabilities without a web surface
- OpenCode prompt lifecycle no longer depends on PTY-idle heuristics
- detached sessions remain resumable until explicit deletion, and deleting the current session returns the CLI to a no-session state instead of forcing process exit

## M4 - Multi-Agent Workflow Runtime

Outcomes:

- structured completion and handoff contracts suitable for multi-agent workflow scheduling
- circular workflow topology delivered first
- worktree-isolated parallel branches where required
- daemon-owned workflow scheduling, routing, barriers, and aggregation state
- explicit distinction between Arroba-managed top-level agents and provider-native subagents

Exit criteria:

- multi-agent workflow execution works locally through the daemon without breaking the single-agent path
- workflow concurrency and worktree safety rules are enforced centrally

## M5 - Remote Access and Web Surfaces

Outcomes:

- server relay and discovery flows
- machine registry and presence
- session-scoped E2E encryption for user-generated in-transit payloads
- operational metadata storage boundaries enforced
- web client and relay-backed remote attachment path

Exit criteria:

- remote clients attach reliably via relay
- relay operates without requiring plaintext user content
- local CLI and web clients share the same daemon/protocol semantics

## M6 - Provider Switching, Memory, and Agent Extensions

Outcomes:

- provider adapter abstraction hardened
- canonical control operations:
  - `attach_file`
  - `request_memory_update`
  - `request_compaction_summary`
- provider installation/auth-state probing with native CLI login reuse
- provider version probing plus shipped command catalogs for supported provider versions
- best-effort `/agent` completion on unsupported provider versions with explicit warnings
- transfer package generation for provider switch/machine reassignment/resume
- dual memory model implementation:
  - short-term memory
  - long-term memory
- user-triggered Arroba context compaction flow (`/compact`)
- agent-scoped extension registry for skills, MCPs, command packs, and related provider assets
- daemon-managed MCP runtime with per-agent binding and visibility

Exit criteria:

- daemon can perform memory-refresh inquiry without using terminal prompt path
- provider switching works without depending on provider-private hidden state
- extension binding and MCP visibility are enforced per top-level agent

## M7 - v1 Stabilization and Launch

Outcomes:

- reliability hardening and compatibility testing across providers
- operational telemetry and failure diagnostics
- user/admin docs for setup, security expectations, and limitations
- release process and upgrade guidance
- completion of the v1 workflow rollout, with circular topology delivered earlier and hierarchical topology completed later in v1

Exit criteria:

- v1 release checklist complete
- known non-goals and follow-up items documented

## 4. Cross-Cutting Workstreams

- Project-wide observability and debugging pipeline
- Provider compatibility matrix and adapter conformance
- Extension compatibility matrix and projection rules per provider
- UX quality for slash-command completion, status, and transfer transparency
- Security/privacy review and threat modeling
- Performance targets for PTY throughput and capability latency
- Generic workflow-engine compatibility: directed-graph scheduling model, structured handoff/completion contracts, worktree isolation, and aggregation/barrier semantics

## 5. Risks and Mitigations

- **Risk:** Provider behavior variance across CLIs.
  - **Mitigation:** strict adapter contracts, shipped versioned command catalogs, custom-command discovery where supported, and explicit best-effort warnings on unsupported versions.
- **Risk:** Provider login drift or expired local sessions.
  - **Mitigation:** reuse provider-native login flows, probe structured auth state before launch, and surface local reauthentication guidance without storing provider credentials.
- **Risk:** Provider-local extension files leak across agents or workflows.
  - **Mitigation:** keep a daemon-owned extension registry, bind per top-level agent, and use provider config-root/worktree isolation when materializing provider views.
- **Risk:** Memory drift or stale long-term memory.
  - **Mitigation:** user review/edit/remove controls + explicit refresh reasons.
- **Risk:** Overgrowth of control surface in v1.
  - **Mitigation:** keep canonical control operations minimal and versioned.
- **Risk:** Relay trust expansion.
  - **Mitigation:** maintain session-E2E transport and metadata minimization.

## 6. Post-v1 Candidates

- richer workflow automation and policy hooks
- broader messaging-client integrations
- advanced machine migration orchestration
- optional content persistence with explicit governance
