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

The next milestone is the matching Cloud/browser harness and cross-repository contract parity, followed by the first real durable prompt-lifecycle slice in the kernel.
