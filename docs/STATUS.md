# Delivery Status

Detailed milestone and delivery status for Arroba. The [README](../README.md)
carries a short summary; this file holds the full history so the README stays
oriented toward what Arroba is and how to run it.

## Milestones

- M0, "Foundations" — complete.
- M1, "Core Session Runtime" — complete.
- M2, "End-to-End Local OpenCode Baseline" — complete.
- M3 — largely delivered for the local baseline hardening work.
- M4 — in progress; first manual multi-agent session runtime slice landed in `main`.
- M4.5, "Kernel Runtime Refactor" — in progress alongside remaining M4 stabilization.

## v1 scope

- single-agent sessions
- manually directed multi-agent sessions
- multi-agent workflow execution

## Delivery priority

1. Close the OpenCode-first runtime cycle: capabilities, agent harnessing behavior, and multi-machine session work.
2. Finish the kernel runtime refactor so interactive commands stay responsive while background work, provider I/O, history reads, and replay/reconnect paths run concurrently.
3. Polish the TypeScript CLI as the reference client.
4. Add multi-platform clients such as web and iOS/Android on the same daemon/protocol model.
5. Add more providers such as Claude Code and Codex and harden the generic provider-adapter/protocol shape.

## What the codebase provides

- a pnpm workspace for TypeScript packages
- a minimal Fastify server with a health endpoint
- a shared domain package for workflow-oriented core v1 entities
- a Rust daemon runtime with config/bootstrap wiring, in-memory session lifecycle, shared attachment participation, provider-run orchestration, prompt queueing/config propagation, and PTY-backed terminal fan-out
- a real local daemon IPC surface, a TypeScript OpenTUI local CLI with an OpenCode-inspired transcript/prompt layout, and a working OpenCode baseline path with prompt submission and live streamed output
- a kernel-hosted WebSocket transport for the TypeScript CLI, including pushed events, resumable subscriptions, heartbeat/liveness, and reconnect-friendly behavior
- the M4.5 kernel runtime slices: normalized kernel commands, an event log service with replay-gap handling, a command router, bounded interactive routing, provider-run actor coverage, `KernelSessionService`/`KernelAgentService` lifecycle services, per-agent and per-session command mailboxes, projection-first reads for warmed state, a shared `PromptStateOwner`, and router-boundary routing for daemon/workflow/MCP/relay requests
- a real manual multi-agent session slice in the daemon and TypeScript CLI: agent records, focused-agent prompt routing, per-agent provider-run ownership/history metadata, `/agent ...` management commands, `Ctrl+A` focus cycling, and `individual`/`split` response views
- a Rust compatibility launcher for the TypeScript CLI
- a local daemon smoke harness for managed-session flows
- a Prisma schema aligned with workflow-oriented runtime entities
- baseline CI for TypeScript and Rust checks

## Current caveats

- the OpenCode-backed multi-agent path still needs stabilization, but the current daemon and CLI suites are green
- the split-pane TypeScript CLI is still an initial slice centered on the primary transcript plus up to two auxiliary panes
- the M4.5 ownership refactor is closed: session, prompt, provider process/output, workflow/runtime-tool, and transport/relay ownership plus runtime fallback deletion and dead-code purge are complete; `DaemonApp` remains bootstrap/composition scaffolding, not the command-state owner
- workspace claims are a bounded safety and scheduling layer; M4.6 managed artifact I/O coordinates Arroba-managed provider-session writes; remaining coordination work is port claims, policy commands for unsafe mode, optional integration checks, and post-v1 artifact-specific region models
- generic agent transport is intentionally deferred; OpenCode continues to use its native local HTTP + SSE adapter path

The project specification and architecture remain the primary source of truth
for behavior beyond this bootstrap.
