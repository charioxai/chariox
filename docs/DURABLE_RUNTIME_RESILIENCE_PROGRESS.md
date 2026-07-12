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

## Milestone 4: Durable Prompt Delivery Acknowledgements

Kernel-owned prompt envelopes now carry a private durable delivery phase: `accepted`, `dispatching`, or `delivered`. Local, remote, workflow, queued-promotion, and structured-provider paths persist `dispatching` before crossing their external side-effect boundary. PTY/native bridge and remote lease paths persist `delivered` only after the write or relay acknowledgement succeeds; structured providers persist it only after the provider actor acknowledges submission.

Structured acknowledgements include the provider-native resume identity and update both the provider run and agent runtime profile before the prompt becomes `delivered`. Claude runs receive a UUID v4 `--session-id` before launch when they do not already have a resume identity, OpenCode keeps its bound session identity, and Codex captures its thread identity as soon as submission returns. The delivery metadata remains absent from public prompt serialization and survives compact prompt events, snapshots, and entity checkpoints.

Focused tests cover private-state round trips, exact-prompt phase transitions, local dispatch acknowledgement, provider resume-state persistence, and Claude session-id generation and argument sanitization.

This milestone records enough evidence to classify restart uncertainty but does not yet change restart behavior. Startup reconciliation must inspect the durable phase and provider-native transcript/session state before deciding whether to dispatch, resume observation, or surface a provider-specific limitation. It must never blindly resend a `dispatching` or `delivered` prompt.

## Milestone 5: Non-Destructive Restart Reconciliation

Kernel bootstrap now clears only ephemeral attachment and active-provider pointers. It preserves active prompt queues, active and prepared workflow runs, node states, turn envelopes, and scheduler authority for the async runtime reconciler. Explicit daemon shutdown keeps the former interruption behavior through a separate shutdown-only path.

After the relay connector starts, the runtime scans every preserved active prompt. Local prompts durably recorded as `accepted` are redelivered through the normal provider path even though their originating terminal attachment no longer exists. Remote prompts either resume projection draining from their acknowledged worker run or replay the same home prompt identity through the existing leased-agent idempotency path. Local `dispatching`, `delivered`, and legacy prompts remain active and are not blindly resent.

Focused coverage proves bootstrap retains running and prepared workflow state, shutdown still interrupts work, accepted local prompts progress to `delivered`, and uncertain prompts retain their exact identity and phase without a duplicate dispatch.

The next recovery slice must reconcile uncertain local prompts against provider-native transcripts and session identities, then issue a durable continuation operation when the original provider turn was delivered but its observation was interrupted. Provider launch failures must also retry without completing the preserved prompt.

## Milestone 6: Transcript-Aware Crash Continuation

Restart reconciliation now scans official Codex, Claude, and OpenCode native transcripts, preferring the exact worktree and matching both the original prompt and a deterministic recovery-operation anchor. It restores provider-native session identity before relaunch and sends an idempotent hidden continuation operation instead of repeating a turn that may already have executed. A `dispatching` prompt with no provider identity or transcript evidence is redelivered only after the bounded transcript scan; a `delivered` prompt without identity remains preserved for later reconciliation. Synthetic `dev-stub` runs retain deterministic original-prompt replay for crash drills.

Recovery operation generation and phase are private durable prompt metadata. Provider launch or continuation failure resets the operation to `accepted` and leaves the original prompt, workflow run, node run, and scheduler authority active for retry. The startup reconciler retries pending transcript discovery for up to five minutes.

Kernel clients now detect a missing control response, reconnect, and replay the same request and command ids within the existing retry window. Socket-generation guards prevent stale close, error, or heartbeat callbacks from tearing down a replacement connection. The kernel command-result cache therefore turns response loss into exactly-once command execution plus replayed acknowledgement.

Provider PTY input uses a bounded per-process writer pump. App-lock paths enqueue without blocking, while delivery-critical prompt dispatch waits for write confirmation outside the global authority lock with a bounded deadline. This prevents provider backpressure from freezing unrelated history, projection, and control reads.

Focused tests cover transcript matching, durable continuation generations, launch-failure preservation, response-loss replay, stale socket callbacks, bounded PTY queues, and lock-free confirmed writes. Full validation passed with 2,053 kernel tests and 813 kernel-client tests. Both local `SIGKILL` drills preserve the same active prompt, provider run identity, running workflow/node/scheduler state, grants, transcript, and recall history across restart.

Provider limitation: official-provider continuation depends on a provider-native session id or discoverable local transcript. When neither exists, Arroba preserves delivered work instead of risking duplicate execution; dispatching work is redelivered only when the transcript scan finds no execution evidence. Remote provider recovery continues through the leased-agent idempotency path and remains part of the distributed live-drill gate.
