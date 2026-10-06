# MP-08 / MP-10 / MP-11 — evals phase 1, round 2

Round 2 is active and remains **not accepted**. The coordinator allocated
local daemon448 / relay peer91 and identified the product-linked Codex profile.
The local accounting implementation and real-path drill are being validated;
no scored smoke/full benchmark result is claimed yet.

## MP-08 / MP-10 / MP-11 — round 2 implementation and provenance

Preparation commits are `7e7ae0e46` and `5159dc3b0`. The frozen real base is
`5159dc3b07504c8ede8a59eab82a662da2cfd9b6`, local435 / peer73.
Its source-matched kernel hash is
`0e6e4c423452b39a1df474da5e79982dd5e85e4061473583c5b847a27c271cdd`.
The candidate uses the coordinator allocation448 /91; this lane did not choose
protocol numbers. Snapshot assertions and an accounting wire hash guard cover
request, response and provider usage counters.

The kernel persists prompt/run-bound official usage in operational history
before prompt settlement. The shared `GetSessionUsage` request and real TUI
`/session usage` project session, individual-agent and delegation-tree totals.
Repeated reports replace the same prompt/run; children count once in the
session total. Unmeasured bound prompts remain unavailable. Codex cumulative
thread counters are differenced against the start-of-turn baseline; a fresh
thread starts at zero, while an unknown resumed-thread baseline is unavailable.
Regressing counters do not reuse a prior apparently valid measurement. Claude
final-result normalization is wired, but its leased acceptance and OpenCode
integration are not established by the Codex drill.

acct-686 is linked through the normal TUI `/provider accounts link` flow,
followed by the ordinary `RefreshProviderAccountProfile` operation. Its mapping
comes from benchmini/benchom2w/acct/acct2/acct3 lane notes. Disposable kernels
consume the approved product-materialized profile; no credential file is read,
printed or copied by these adapters. Operator-local profile locations remain
in the external lane status rather than public source.

The base real TUI/kernel/official Codex0.159.3 run completed the exact answer
`ACCOUNTING_REAL_PATH_OK` on `gpt-6.1-sol` with low effort, then failed the
accounting requirement because base435 has no usage-report request. Cleanup
completed. Earlier setup/screenshot failures and the HTTP400 rejection of
`gpt-5.3-codex` are separate diagnostic attempts, not accounting-red evidence.
Evidence is external under the evals round2 directory and retains PTY capture,
per-step terminal screenshots, kernel logs, exact input and result identities.
The candidate live relay/TUI/kernel/provider result is pending.

Pricing remains explicitly unavailable when the official harness lacks a field
required by the exact dated price table. Codex0.159.3 supplies cumulative input,
cached input, output and reasoning, but no cache-write/context-price-band
breakdown for the supported `gpt-6.1-sol` model. The runner retains known tokens
with null cost; it never invents zero cache writes or selects a band. This blocks
matching USD acceptance unless an exactly priceable model/profile or authoritative
counters are supplied. Diagnostic benchmark execution can continue unpriced.

## MP-08 / MP-10 — campaign state

The exact89 /50 task pins below remain unchanged. Codex smoke10 precedes each
full run, with serial tasks and fresh plan-exhaustion checks. Terminal-Bench
profile placement is awaiting a coordinator choice between the official Harbor
same-host volume with normal product linking and a managed worker binding.
SWE-bench's solver checkouts can use the approved local profile directly.
Claude/OpenCode await the coordinator's Mac home to builder worker grant.
No source test, partial task set or unavailable accounting closes an MP item.


## MP-08 / MP-10 — frozen official pilots

Complete ordered task IDs, revisions and dataset byte hash are retained in
[inputs.lock.json](../apps/cli/scripts/benchmarks/evals-phase1/inputs.lock.json).
No replacement subset or modified scoring rule is used.

| Pilot / official source | Revision | Full denominator |
| --- | --- | ---: |
| [Terminal-Bench2.0](https://github.com/laude-institute/terminal-bench-2) | `2fd12b88aafdd04a52c298e3940bcb189f9766d6` |89 |
| [Harbor](https://github.com/harbor-framework/harbor) | `c803185a8b7c88c163abe48a22e9cea0bbd95e90` |89 |
| [HAL Verified Mini dataset](https://huggingface.co/datasets/MariusHobbhahn/swe-bench-verified-mini) | `b316c349947c29963fce3f4a65967c9807a4b673` |50 |
| [Official SWE-bench Docker evaluator](https://github.com/SWE-bench/SWE-bench) | `726c5461e2ef52d83cf1ea2107870a8bb3328d57` |50 |

The [dated version1 price table](../apps/kernel/src/usage_accounting/prices-2026-10-06.json)
uses integer nanodollars at standard global API list prices. Reasoning counts in
output exactly once. Exact model names and required categories are mandatory;
no alias, context band or cache-write TTL is guessed. These are API-equivalent
estimates, not subscription invoices; tool/infrastructure charges are excluded.
Sources: [OpenAI API pricing](https://developers.openai.com/api/docs/pricing) and
[Claude API pricing](https://platform.claude.com/docs/en/about-claude/pricing).

Harbor uses the unchanged pinned official task/verifier, with a custom agent
that drives the production TUI/kernel/provider path. Runtime bundles bind all
built app/dependency/provider files by hash, reject escaping symlinks and require
committed source. Profiles are separate product inputs. SWE-bench receives only
the problem statement and an instance-base solver checkout. Gold/test patches
stay in the separate scorer input; predictions use the standard instance/model/
patch format. The scorer checks the exact clean official evaluator checkout and
all required IDs, then invokes its unchanged Docker evaluation.

## MP-08 / MP-10 / MP-11 — validation scope

Round1 preparation had7 Rust normalization/price checks and17 Python admission
checks. Round2 has18 Python checks, including known tokens with an unavailable
price, plus shared client formatting/protocol checks. The prescribed Node client suite passes1117/1117. Expanded Rust persistence,
aggregation and snapshot checks are pending the shared compile slot. These are
component evidence, not live acceptance. No benchmark task has been scored yet.
The historical0-run diagnostic CSV/plot is not a zero-cost/zero-accuracy result.

All state, logs, screenshots and output stay outside source repositories. Signal
helpers reject system/invalid PIDs and settle only verified owned descendants.
Fresh rolling-limit exhaustion pauses campaigns; stale meters and zero purchased
credits do not. Resource floors are9GiB MemAvailable /10GiB root free. No push,
PR, CI, deployment, public submission or shared-resource cleanup is authorized.


MP-08 / MP-10: scorer admission on originally pinned current evaluator02e7a74f
failed on missing `image`; that evaluator now requires enriched task records.
Before any scored run, the evaluator pin was corrected to official release
v4.1.0 (`726c5461e2ef52d83cf1ea2107870a8bb3328d57`), which admits all50 unchanged
HAL Mini records. This is an explicit infrastructure correction, not a scoring
rule change or relabelled result. The evaluator runs with cache level `instance`
and clean=false, so its global cache cleanup cannot delete other lanes' images.
Admission success alone does not establish a scored task.
