# Durable Runtime Resilience Progress

## Baseline

- OSS worktree: `chariox-runtime-projection-fix`
- OSS branch: `codex/durable-runtime-chaos-foundation`
- OSS starting `main`: `afc233de00cca623d7641610c54d7a78a8ade0fc`
- Cloud worktree: `chariox-cloud-resilience-validation`
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

The Cloud foundation drives the production browser relay event handler through deterministic reverse-decrypt ordering, duplicate event replay, relay binding replacement, stale callbacks, route partition/reconnect, and snapshot fallback. The handler now serializes frames per subscription and rejects callbacks from superseded relay key generations. OSS and Cloud publish the same `chariox.drill.chaos_contract.v1` manifest, and both cross-repository gates can require exact replay-schema, fault-kind, and invariant parity. Cloud staging enables that requirement before running distributed evidence.

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

Provider limitation: official-provider continuation depends on a provider-native session id or discoverable local transcript. When neither exists, Chariox preserves delivered work instead of risking duplicate execution; dispatching work is redelivered only when the transcript scan finds no execution evidence. Remote provider recovery continues through the leased-agent idempotency path and remains part of the distributed live-drill gate.

## Milestone 7: Disposable Worker Runtime Recovery

The first real same-host matrix run exposed that worker lease backing sessions were being journaled as prompt authority without a matching durable session lifecycle. A worker restart then replayed an orphan `session.prompt_state.updated` event and exited with `SessionNotFound`, leaving the home kernel unable to repair the stale lease.

Leased backing sessions are now explicitly ephemeral in the shared session store. Their prompt state and agents are excluded from durable events, snapshots, and checkpoints, while normal user sessions and hidden publication sessions remain durable. Restore also ignores orphan prompt events defensively so older worker journals cannot prevent startup. The home kernel remains the sole durable authority and recreates worker execution state through the existing lease-refresh and idempotent prompt path.

Focused tests cover orphan prompt replay and snapshot exclusion. The websocket reconnect drill now waits for the client-side `transport_resumed` event instead of racing the server-side second subscribe. Provider-thread drills fail immediately on terminal provider errors rather than polling until the global timeout.

The repaired matrix scenarios pass: websocket drop resumes from event id 1; home restart preserves the original leased-agent id; worker restart and both-kernel restart each acquire one fresh lease and complete the post-restart prompt. Codex worker thread transfer also passes with provider session identity preserved. OpenCode could not be validated beyond Chariox launch/error projection: OpenCode Zen reports insufficient balance, and the OpenAI-backed OpenCode profile reports OAuth refresh `401`. Both are external account failures and now surface in seconds.

## Milestone 8: Canonical Two-Client Workflow Projection

The Cloud web terminal now renders and resolves workflow actions from the same merged projection of current session state, workflow snapshots, and publication state. Destructive actions no longer resolve against a narrower session-only list than the one shown to the user. The local web-plus-TUI drill passed 43 synchronized transitions covering Freeform prompt submission, queue, steer, clear, cancel, configuration changes, interaction popups, workflow creation, edits, parameters, run state, and reciprocal control. The Hetzner web-plus-TUI drill passed all 32 distributed transitions with low-delay convergence and screenshot evidence.

Preserved evidence:

- `.artifacts/runtime-resilience-goal/local-two-client-projection-fix-1`
- `.artifacts/runtime-resilience-goal/hetzner-two-client-cb28b8d7`

Cloud staging now accepts an external relay for this matrix and reconciles workflow action projections through the shared resolver. Non-detached drill children are always cleaned up. Cloud's complete workspace suite, lint, and 224-check validation gate passed; the web suite reported 3,244 passing tests.

## Milestone 9: Distributed Recovery Drills

The Hetzner collaborator matrix passed relay restart, collaborator reattachment, worker restart with a fresh lease and provider run, and home-owned grant/revoke authority in 27.9 seconds. Cleanup found no run-owned process or temporary root. The worker used about 84 MB RSS, the relay about 12 MB, and the host retained about 4.8 GB free disk during the drill. Evidence is in `.artifacts/runtime-resilience-goal/hetzner-collab-c145b0eb6`.

The hosted staging second-kernel matrix passed in 32.9 seconds through the normal Chariox prompt path. Its home and worker kernels each remained near 78 MB RSS after isolating the drill `HOME`; using the global `~/.chariox` database had previously caused an avoidable 1.9 GB startup spike. The multi-user hosted drill also passed peer-owned remote-agent home-proxy script, MCP, and connector execution, denied peer grant/revoke, and deleted all four temporary Cloud identities. Evidence is in `.artifacts/runtime-resilience-goal/hosted-second-kernel-managed-fix`.

The local slice lifecycle matrix and real Codex save/restart drill pass. Saved-state cleanup now resets saved state before deleting a slice, so no drill-owned state image survives. The Hetzner, hosted Cloud, and slice fixes were derived by comparing the existing related drills and preserved evidence before changing runtime behavior.

## Milestone 10: Provider Thread Continuity

Provider transfer drills now require an exact response marker that never appears verbatim in the prompt. ANSI normalization remains supported, but arbitrary subsequence matching was removed because a shutdown-flushed prompt echo could otherwise create a false pass. Unexpected provider termination also fails immediately.

Claude isolated workers now receive a temporary Keychain credential export, `.claude.json`, and Claude settings; `claude auth status` must pass before launch. Run-owned provider state is copied from an isolated home to the isolated worker only after the source turn, avoiding the user's 98 GB Codex home and 1.3 GB OpenCode store. Native Claude launches now pass `--resume` for the requested session. Home-managed slices use `/workspace` consistently before and after restart, which keeps provider project-scoped transcript identity stable.

Strict live results:

- Codex isolated worker transfer: `.artifacts/provider-thread-transfer/1783908503626-37288`
- Claude isolated worker transfer: `.artifacts/provider-thread-transfer/1783908555763-40040`
- Codex slice restart: `.artifacts/provider-thread-transfer/1783908220981-22884`
- Claude slice restart: `.artifacts/provider-thread-transfer/1783907968000-10688`

All four preserve the provider session id, exact second-turn recall, and an active provider run through the assertion point. Worker runs preserve the exact workspace; slice runs preserve `/workspace`. Worker-side provider teardown now runs before kernel termination, followed by delayed idempotent temporary-root removal. Slice cleanup erases temporary Codex and OpenCode auth copies from retained artifact roots while preserving non-secret provider state, and removes the external Claude credential root. Post-run audits found no matching provider process, credential file or root, drill container, or saved-state image.

Claude credentials are currently valid and both Claude drills pass. OpenCode remains externally blocked: Zen reports insufficient balance and the OpenAI OAuth profile fails refresh with `401`. Chariox correctly projects both failures and cleans up immediately; same-thread continuation cannot be claimed until one OpenCode account path is restored.

The final OSS gates pass: 2,056 kernel unit tests plus integration and documentation tests, 1,346 CLI tests, 70 server tests, 13 shell tests, lint, formatting, and all 630 validation-suite checks.
