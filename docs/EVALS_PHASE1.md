# MP-08 / MP-10 / MP-11 — evals phase 1

Phase 1 is **blocked, not accepted**. The lane has prepared token normalization,
a dated API price table and adapters for two pinned public benchmarks. No provider
baseline or scored task has run. This report makes no Pareto-frontier claim and
closes no MP acceptance item.

## MP-08 / MP-10 / MP-11 — source and measurement scope

The lane started from OSS main `e325afa580d81954e2c179757fc53fa02ed2a2b3`
(local435), which includes G2, Apps Phase 1 and the MP-11 queue. The shared
prebuilt kernel reported local439 and was excluded from source-matched proof.
No local or relay protocol version was allocated or changed by this lane.

`apps/kernel/src/usage_accounting.rs` normalizes the official harness counter
conventions: Codex input includes cached reads and output includes reasoning;
Claude result input excludes cache reads/writes; OpenCode message output
excludes its separate reasoning counter. Missing counters remain unavailable.
Negative, inconsistent and overflowing counters fail closed. Repeated Codex
cumulative updates must be differenced, and OpenCode messages deduplicated,
when this module is wired into the kernel turn lifecycle. That wiring, durable
per-turn attribution, agent/session/delegation-tree totals and a CLI report
remain unimplemented pending the coordinator's protocol allocation.

The version-1 [price table](../apps/kernel/src/usage_accounting/prices-2026-10-06.json)
is dated 2026-10-06 and uses integer nanodollars per token. Quotes use public
standard global API list prices, exclude infrastructure/tool charges and are
**API-equivalent estimates, not subscription invoices**. Reasoning is included
in output exactly once. Exact model names are required. Unknown models,
necessary missing counters, an unspecified context band or unreported cache
write TTL produce an unavailable quote. No rate is guessed from an alias.
Sources: [OpenAI API pricing](https://developers.openai.com/api/docs/pricing)
and [Claude API pricing](https://platform.claude.com/docs/en/about-claude/pricing).

## MP-08 / MP-10 — pinned pilots

The complete ordered task IDs, revisions and HAL parquet byte hash are retained
in [inputs.lock.json](../apps/cli/scripts/benchmarks/evals-phase1/inputs.lock.json).
No random replacement subset is used.

| Pilot / official source | Revision | Full denominator |
| --- | --- | ---: |
| [Terminal-Bench 2.0](https://github.com/laude-institute/terminal-bench-2) | `2fd12b88aafdd04a52c298e3940bcb189f9766d6` | 89 |
| [Harbor harness](https://github.com/harbor-framework/harbor) | `c803185a8b7c88c163abe48a22e9cea0bbd95e90` | 89 |
| [HAL Verified Mini selection](https://hal.cs.princeton.edu/swebench_verified_mini), [dataset](https://huggingface.co/datasets/MariusHobbhahn/swe-bench-verified-mini) | `b316c349947c29963fce3f4a65967c9807a4b673` | 50 |
| [Official SWE-bench Docker evaluator](https://github.com/SWE-bench/SWE-bench) | `02e7a74ffd0b707aab73d203fe87bdc7c76afc8e` | 50 |

The Harbor adapter implements the pinned official `BaseAgent` interface and
uses a real Chariox TUI, a disposable kernel and the selected official provider
harness inside the task environment. A hashed runtime bundle may be installed
there; credentials may not be included. A product-materialized profile is a
separate required input in local mode. Leased mode instead attaches to the
coordinator-granted home task agent bound to the official environment's worker;
provider access transfers through the normal lease, with no builder credential
handling. The SWE adapter admits the exact dataset and clean
instance base, sends only the problem statement to Chariox, and emits standard
SWE-bench predictions. Its scorer invokes the unchanged pinned official Docker
evaluator with a frozen local JSON dataset. Neither adapter has yet been
validated in a real task environment. Fixture/interface checks are preparation,
not benchmark execution or accounting acceptance.

## MP-08 / MP-10 — diagnostics

| Baseline | Pilot | Smoke completed | Full completed | Accuracy | Tokens | API-equivalent USD | Wall time |
| --- | --- | ---: | ---: | --- | --- | --- | --- |
| Codex through Chariox | Terminal-Bench 2.0 | 0 | 0 / 89 | unavailable | unavailable | unavailable | unavailable |
| Codex through Chariox | HAL Verified Mini | 0 | 0 / 50 | unavailable | unavailable | unavailable | unavailable |

The external evidence directory contains a 139-row task-status CSV with blank
measurement fields and a cost-versus-accuracy PNG explicitly marked **no scored
runs**. These are diagnostic status artifacts, not zero-cost or zero-accuracy
results. No task was attempted, so no provider quota exhaustion was proved and
there is no task-level resume point beyond the beginning of each pinned set.
The runner stops only on fresh, active rolling-plan exhaustion; zero purchased
credits and expired cached meter snapshots are insufficient.

## MP-08 / MP-10 / MP-11 — exact blockers and next acceptance

1. The coordinator must allocate a local protocol number. The existing
   serialized `ProviderRunTokenUsage` has total/context counters only. This lane
   stopped before changing that contract, as instructed. After allocation,
   wire official events into durable per-turn history and the common usage
   report, update protocol snapshots and run the base-red/candidate-green drill.
2. The owner/coordinator must identify which registered Codex account maps to
   authorized `acct-686` and provide its product-linkable path. Product status
   lists three authenticated Codex profiles, but none is labelled `acct-686`.
   No credential files were inspected or copied to infer the mapping. The
   coordinator's 12:35 UTC direction says to continue Codex acct-686 locally;
   its product-linkable path is still missing from the lane instructions.
3. The real task environments need the source-matched runtime bundle and a
   profile materialized through documented Chariox product commands. Then run
   the accounting drill with the real built TUI/kernel, a real local relay and
   Codex, retaining per-step terminal/screenshot and log evidence. The drill
   must demonstrate matching harness counters and API-equivalent price on the
   real path before accounting is declared done. Then run the 10-task smoke
   before either full diagnostic set, retaining failures and quota resume state.

The 12:35 UTC coordinator inbox directs Claude/OpenCode cells to use leased
agents from the owner's Mac home kernel. Their worker and access grant are
coordinator-owned; no builder login is requested. Both benchmark adapters now
accept local or leased placement, reject missing/foreign home-worker bindings,
and share the same real TUI prompt path. The builder's allowlisted product status
RPC also reports authenticated Claude and OpenCode profiles. A fresh provider run was not performed, so their current
execution readiness is not established. No new owner login or account creation
is presently indicated by that status observation.

## MP-08 / MP-10 / MP-11 — retained validation and cleanup

Evidence is external under the reserved builder's
`/root/.codex/evidence/browser-resume-20260930/evals/`. It includes dependency and
TUI build logs, component checks, exact task pins, allowlisted account status,
resource observations, and CSV/PNG diagnostics. The initial four Python tests
were red on missing adapter admission code, then green; final admission checks
also cover artifact hashes and signal guards. The initial Rust lock-wait
interruption was exit143 before compilation and is **not** a red test. Rust
normalization/price checks subsequently ran under the compile slot and passed
7/7; the full kernel library `cargo check` also passed. Seventeen Python checks
pass, including local/leased placement admission. The TUI Done-state and provider-specific cache-write regressions failed before
their fixes and pass after. No fail-first provider accounting drill is claimed.

No provider, task container, local relay or deployment was launched for these
pilots. No GitHub action, push, PR, public submission, merge or staging change
was performed. Lane-owned dependency tools and public benchmark input caches
are retained for replay; disposable component fixtures cleaned their temporary
state. Shared reviewer, provider-account and other-lane resources were preserved.
