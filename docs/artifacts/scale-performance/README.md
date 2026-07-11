# 500-agent efficiency validation

Validation date: 2026-07-11  
OSS branch: `codex/scale-efficiency-500` at `fd5f3f558`  
Cloud branch: `codex/scale-efficiency-500` at `faf94560`

This directory preserves the final release-mode evidence for `docs/SCALE_PERFORMANCE_PLAN.html`. All live drills used isolated worktrees, temporary state roots, and empty port bands. Provider child-process capacity is reported separately from Arroba orchestration, as required by the plan.

## Outcome

The Arroba-owned scale gates pass for 500 concurrently active agents distributed across ten worker kernels and for a single kernel rendering 500 synthetic agents. Read paths use immutable projections; provider work is readiness-driven; history and durable state use bounded batched writers; terminal and relay state is sharded; browser hydration is visible-first; large trees are virtualized; and Cloud inventory writes are coalesced.

The original repeated-session/history CPU problem is removed from the hot path. Session and waiting-room readers clone immutable projection handles instead of cloning full runtime graphs. Event persistence now applies bounded backpressure before serialization can accumulate, and command replay retention is bounded by estimated heap footprint as well as disk size.

## Three consecutive frontend matrices

Each release pass covered ten cases: 20×50, 50×20, 100×10, single-session 200, single-session 500, slow client, background tab, disconnected client/replay, retention, and mixed active/idle 100×10.

| Pass | Result | Peak kernel RSS | Peak kernel CPU | Worst reconnect p95 | Worst hydration p95 | Worst long task | Long-task aggregate |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | 10/10 | 349 MB | 69.8% | 990 ms | 2,401 ms | 71 ms | 131 ms |
| 2 | 10/10 | 349 MB | 68.4% | 855 ms | 2,445 ms | 93 ms | 197 ms |
| 3 | 10/10 | 354 MB | 69.7% | 965 ms | 2,429 ms | 77 ms | 195 ms |

The responsiveness gate bounds maximum and aggregate long-task duration (100 ms and 250 ms) instead of requiring a scheduler-sensitive count of zero. Task counts remain diagnostic. All latency, batching, request-count, DOM, WebSocket, CPU, and memory gates passed.

The final visual artifact, `single-500-terminal.png`, shows 500 sidebar agents with two mounted panes, one completed prompt, 499 idle agents, and a ready kernel. The full UI retained 1,109–1,150 DOM nodes in the 500-agent cases rather than mounting 500 terminal panes.

## Distributed 500-active gate

Three consecutive release runs created ten worker kernels with 50 leased/running agents each. Every run maintained 500 remote leases and 500 running provider runs, sampled prompt/output/completion routing on every worker, and used ten shared synthetic provider processes so the measurement isolates Arroba orchestration.

| Pass | Spawn | Launch | Prompt accepted | Sample completion | Peak process RSS | Peak process CPU |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 9,437 ms | 3,431 ms | 33 ms | 33 ms | 341 MB | 68.4% |
| 2 | 8,400 ms | 3,456 ms | 36 ms | 36 ms | 395 MB | 55.1% |
| 3 | 9,476 ms | 3,549 ms | 35 ms | 443 ms | 381 MB | 31.0% |

Every pass terminated all 12 owned roots and descendants with no forced or remaining processes.

## Sustained stream and reconnect pressure

Three consecutive 30-minute runs sustained approximately 1 MiB/s (1,887,436,800 bytes each). Append latency and orchestration resource use remained bounded:

| Pass | p95 | p99 | Peak RSS | CPU p95 | Cleanup |
|---|---:|---:|---:|---:|---:|
| 1 | 88 ms | 163 ms | 815.8 MB | 13.3% | clean |
| 2 | 58 ms | 66 ms | 555.2 MB | 11.9% | clean |
| 3 | 81 ms | 341 ms | 851.8 MB | 11.4% | clean |

The reconnect storm used 32 independent clients over five concurrent reconnect cycles while one slow subscriber received 4,096 × 8 KiB records. Thirty-one healthy subscribers stayed responsive; reconnect p95 was 281 ms; only the slow lane closed; target queue saturation stayed at zero; peak kernel RSS was 397.5 MB; and cleanup required no forced termination.

## Persistence and correctness

The full OSS validation passed 1,952 kernel unit tests plus 56 focused CLI/runtime integration tests, and the relay passed 74 tests. The Cloud suite passed 3,287 tests, including 3,186 web tests and 66 API tests. Release builds passed in both repositories.

Three live persistence drills stopped and restarted the kernel and verified restored session, agent, extension grants, provider run, workflow/run state, operational history, and recall. Focused checkpoint tests also cover crashes before, during, and after entity checkpoint boundaries.

Cloud's top-level lint command remains blocked by pre-existing baseline policy failures unchanged from `origin/main`: two files exceed the 1,000-line cap (`session-snapshot-controller.test.ts`, 1,099 lines; `extensions-controller.ts`, 1,093 lines), and the style linter reports two undefined legacy tokens (`--focus-ring`, `--color-info`). Type checking, builds, and tests pass; this branch introduces none of those lint failures.

## Provider parity

The transcript-only live sample intentionally avoids a background observer that could mutate active prompt state.

- Codex: passed all 20 assistant markers, 20 tool markers, and the final marker; native transcript captured.
- Claude: passed all 20 assistant markers, 20 tool markers, and the final marker; native transcript captured.
- OpenCode: the client/runtime path was exercised, but the configured Zen account returned `Insufficient balance`, while the configured OpenAI OAuth account returned `Token refresh failed: 401`. These are external provider-account limitations, not Arroba orchestration failures. The drill now classifies balance, credits, billing, token-refresh, unauthorized, and 401 messages correctly even when OpenCode exits with status 0.

Per the plan assumptions, provider quotas and credentials are outside the Arroba orchestration capacity guarantee and are reported separately rather than weakening the scale gate.

## Evidence index

- `frontend-matrix-pass-{1,2,3}.json`: compact ten-case release summaries.
- `distributed-500-pass-{1,2,3}.json`: full distributed orchestration reports and cleanup audits.
- `stream-soak-pass-{1,2,3}.json`: raw 30-minute stream metrics.
- `reconnect-storm.json`: slow-subscriber and reconnect-isolation report.
- `restart-persistence-pass-{1,2,3}.log`: live stop/restart verification records.
- `single-500-terminal.png`: final visual validation.

Temporary provider transcripts are deliberately not committed because they can contain provider-native prompt and account metadata.
