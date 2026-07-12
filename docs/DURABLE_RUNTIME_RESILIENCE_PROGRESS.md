# Durable Runtime Resilience Progress

## Baseline

- OSS worktree: `arroba-runtime-projection-fix`
- OSS branch: `codex/durable-runtime-chaos-foundation`
- OSS starting `main`: `afc233de00cca623d7641610c54d7a78a8ade0fc`
- Cloud worktree: `arroba-cloud-resilience-validation`
- Cloud branch: `codex/durable-runtime-chaos-cloud`
- Cloud starting `main`: `bdb254e65e12875b7f36a3eb11224118a7920182`

Both branches were created from their repository's current `origin/main`. The implementation must not add or restore a hard source-file line cap.

## Delivery Order

1. Build deterministic chaos primitives and executable invariants first.
2. Make kernel command acceptance, idempotency, event ordering, and restart reconciliation durable one vertical slice at a time.
3. Project the same resumable event contract into TUI and web clients with cursor replay and snapshot fallback.
4. Extend coverage through interactions, configuration, workflows, terminal streams, remote leases, slices, and collaboration.
5. Run real local, slice, Hetzner, collaborator, web, and TUI fault drills with resource and screenshot evidence.

When a provider, environment, or orchestration problem blocks a drill, compare the existing related live drill and its preserved artifacts before changing runtime behavior. Commit and push whenever a coherent, verified milestone lands in either repository.

## Milestone 1: Deterministic Foundation

The OSS foundation supplies a seeded PRNG, fake clock, programmable transport faults, generation-aware process death, stale-callback suppression, replay bundle validation, and convergence invariants. The default scenario covers repeated drops, delay, duplication, reorder, partition/reconnect, process death/restart, cursor monotonicity, snapshot recovery, exactly-once execution, bounded queues, authority, and cleanup. It is registered in the runtime resilience matrix and shared validation suite.

## Milestone 2: Browser Chaos And Contract Parity

The Cloud foundation drives the production browser relay event handler through deterministic reverse-decrypt ordering, duplicate event replay, relay binding replacement, stale callbacks, route partition/reconnect, and snapshot fallback. The handler now serializes frames per subscription and rejects callbacks from superseded relay key generations. OSS and Cloud publish the same `arroba.drill.chaos_contract.v1` manifest, and both cross-repository gates can require exact replay-schema, fault-kind, and invariant parity. Cloud staging enables that requirement before running distributed evidence.

## Milestone 3: Durable Prompt Admission

Every kernel-owned prompt-state mirror now appends an acknowledged `session.prompt_state.updated` durable event. Its compact internal payload includes hidden system context and command operation metadata without exposing either through the client session protocol. Local and remote prompt submission attach a stable command id and request fingerprint before admission; replay checks restored prompt state before attachment validation, history writes, provider dispatch, or remote delivery. A matching retry returns the original prompt identity and queue disposition, while a reused operation id with another fingerprint fails. Snapshot and entity-checkpoint payloads retain the private prompt envelope, so compaction does not discard it.

Focused coverage proves an accepted queued prompt, hidden context, operation key, and one-item queue survive a full kernel bootstrap and that the same command replays once even though its old attachment was reconciled. Prompt-focused tests cover local, remote, workflow, queue, projection, and provider-settlement paths.

This milestone does not yet make an in-flight provider turn automatically resumable. The next slice must add durable delivery phases and an outbox/reconciler for active prompt dispatch, steer/cancel/clear receipts, provider launch/resume state, and uncertain side effects before restart reconciliation can stop cancelling active work.
