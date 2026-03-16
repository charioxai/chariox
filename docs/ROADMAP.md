# Arroba Roadmap

## Status

Planning roadmap derived from `docs/spec-v1.md`.

## 1. Roadmap Goals

- deliver a stable daemon-centered v1 runtime
- preserve native provider UX while adding orchestration value
- ship memory-aware context transfer with minimal provider coupling
- keep server lightweight with strict security boundaries

## 2. Milestone Structure

- M0: Foundations
- M1: Core Session Runtime
- M2: Capability Surface
- M3: Control Lane and Memory Management
- M4: Remote Access and Security Hardening
- M5: v1 Stabilization and Launch

## 3. Milestones

## M0 - Foundations

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
- multi-attachment support + controller/observer model
- parked provider run support with one active run at a time

Exit criteria:

- local client can run native provider in managed session
- multiple clients can attach without breaking terminal behavior

## M2 - Capability Surface

Outcomes:

- shell command capability
- directory tree + file view/edit capabilities
- screenshot capture capability
- git/worktree inspection capability
- file transfer + attach-transferred workflow
- schedule metadata + daemon execution baseline

Exit criteria:

- capabilities callable from overlay/palette
- capability failures isolated from terminal lane

## M3 - Control Lane and Memory Management

Outcomes:

- provider adapter abstraction hardened
- canonical control operations:
  - `attach_file`
  - `request_memory_update`
  - `request_compaction_summary`
- dual memory model implementation:
  - short-term memory
  - long-term memory
- transfer package generation for provider switch/machine reassignment/resume
- user-triggered Arroba context compaction flow (`<reserved character for arroba commands>compact`)

Exit criteria:

- daemon can perform memory-refresh inquiry without using terminal prompt path
- transfer package is deterministic, inspectable, and non-fatal on unsupported control responses

## M4 - Remote Access and Security Hardening

Outcomes:

- server relay and discovery flows
- machine registry and presence
- session-scoped E2E encryption for user-generated in-transit payloads
- operational metadata storage boundaries enforced

Exit criteria:

- remote clients attach reliably via relay
- relay operates without requiring plaintext user content

## M5 - v1 Stabilization and Launch

Outcomes:

- reliability hardening and compatibility testing across providers
- operational telemetry and failure diagnostics
- user/admin docs for setup, security expectations, and limitations
- release process and upgrade guidance

Exit criteria:

- v1 release checklist complete
- known non-goals and follow-up items documented

## 4. Cross-Cutting Workstreams

- Provider compatibility matrix and adapter conformance
- UX quality for command palette, status, and transfer transparency
- Security/privacy review and threat modeling
- Performance targets for PTY throughput and capability latency

## 5. Risks and Mitigations

- **Risk:** Provider behavior variance across CLIs.
  - **Mitigation:** strict adapter contracts + degradation rules.
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
