# MP-08 / MP-10 / MP-11 — evals phase 1, round 3

MP-08 / MP-10: tokens (input, cached input, output and reasoning) are the
primary cost metrics. Round 3 uses the coordinator-approved official Harbor
same-host placement, with the existing product-linked Codex profile registered
through the real TUI. The 10-task Terminal-Bench smoke precedes 89 fresh full
tasks. Claude/OpenCode adapters remain available; their owner-side lease grants
are still pending. Local benchmark results do not establish managed parity.

MP-08 / MP-10: the existing SWE Verified Mini full result remains **44/50
(88%)**, zero scorer errors. It used product source `82a444a77`, kernel SHA
`f25b48a1...`, official Codex 0.159.3 / `gpt-6.1-sol` / low effort. It consumed
18,013,227 input tokens, including 16,588,544 cached tokens, plus 111,474 output
tokens, of which 8,110 were reasoning. Solver wall time was 4,950.44 seconds.
Applying the dated proxy mapping produces **proxy USD 5.6229604–17.8119658**
for the full run. This is a conservative interval, not an invoice or an exact
billing quote. Original round-2 receipts and runtime identities stay unchanged.

## MP-08 / MP-10 — dated proxy price mapping

The editable [proxy mapping](../apps/cli/scripts/benchmarks/evals-phase1/proxy-prices-2026-10-06.json)
records its source URL, observation date (2026-10-06), units and assumptions.
The exact public API model exists, so the closest mapping is an identity:

| Provider model | Public API proxy model | Short input / cached / write / output per million | Long input / cached / write / output per million |
| --- | --- | --- | --- |
| `gpt-6.1-sol` | `gpt-6.1-sol` | proxy USD 2 / 0.10 / 2.50 / 10 | proxy USD 4 / 0.20 / 5 / 15 |

Source: [official OpenAI API pricing](https://developers.openai.com/api/docs/pricing),
observed 2026-10-06. Reasoning is included in output once. The native harness
omits context-band and cache-write counts; these remain **unknown**, not zero.
The report takes the minimum/maximum band and bounds unreported write tokens
between zero and non-cached input. Its conservative additive envelope includes
base non-cached input plus possible write charges; it does not assert the
provider's billing partition. The lower endpoint is a bound, not an assertion
that writes were zero. No point estimate is emitted while categories are
unknown. Subscription, infrastructure, tool, Fast-mode and regional charges
are outside this proxy. Every plotted dollar value is labeled **proxy**.

## MP-08 / MP-10 / MP-11 — round 3 real-path admission

The first official Harbor attempt failed before container startup because the
builder had no Compose plugin. A SHA-256-verified, pinned Compose v2.39.4 plugin
is now scoped to the lane's Docker CLI tooling. The unchanged pinned official
Bookworm task then reproduced the native loader failure through Harbor runtime
preflight, before provider launch. These attempts have no benchmark score.
A Bullseye native build failed first on its own memory cap, then at linking:
the frozen source uses `posix_spawn_file_actions_addclosefrom_np`, introduced
after Bullseye. Host resource floors remained intact. Its owned compiler
container was removed. A hash-bound public ELF loader/library bundle now keeps
the frozen source and official task images unchanged; real-path validation is
in progress. ELF packaging changes have separate original/packaged hashes.
The real Ubuntu admission passed the official verifier (reward 1) with visible
input193504 / cached171392 / output5734 / reasoning368 and cleanup true.
Official Harbor initializes writable mount targets with mode 0777. Chariox
correctly rejected the profile directory because it was accessible to other
users. The adapter restores mode 0700 on the supplied linked directory before
ordinary product linking; no credential contents or files below it are touched.
The RED directory/status/screenshot receipts and corrected 0700/Evals registration
are retained. Git and a terminal font are ordinary runner prerequisites in slim
images. The native Bookworm task reached Codex, whose `cyber_policy` rejection
is retained as a provider failure. Rejections can arrive before native counters
exist; these are scored by the official verifier with usage unknown, not zero.
Reports retain known token/proxy subtotals and count unmeasured tasks; a campaign
with missing counters has no exact token total or bounded total proxy quote.
Its plot marks the measured proxy lower bound with the total upper bound unknown.
Settled provider failures proceed to official verification and remain in the denominator; transport,
quota and accounting admission failures still stop the campaign.
These diagnostic admissions are separate from smoke
and fresh full results.
The serial campaign retains exact task and harness pins, official verifier
results, real TUI/relay/kernel/provider evidence, fresh quota checks, resource
samples and exact owned-resource cleanup. Missing tasks never become zeros or
a full accuracy score. MP-11 signal guards reject system/invalid PIDs before
harness subprocess signaling. This preparation and RED evidence do not close
an MP acceptance item.

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
Both the base and candidate now have real encrypted self-host relay runs.
Base435 completed its provider turn and failed the real TUI usage projection.
Candidate product source `82a444a77f1456bf3fe75f0782ea4ee7a4d9cf78`, kernel hash
`f25b48a1f1985bf7ec1fed12732eacb83f227feb1773be57a727d1398e4cd294`,
completed with visible input18346 / cached12288 / output9 / reasoning0, matching
the native official cumulative counters exactly. Both cleaned up. The candidate
kernel tree is identical to built source `41a3ee9f6`; its actual TUI was rebuilt
for the session-notice visibility fix. Earlier candidate turns exposed the
collapsed usage notice; the fix places usage on the existing keyed session
notice path. Failed driver-expansion attempts remain RED evidence. The live
receipt proves one fresh, single-agent turn, not resume/delegation/leased parity
or matching USD. Pairing-bootstrap failures without Cloud grants never reached
a provider turn and are not accounting-red evidence.

Historical round-2 exact pricing remains explicitly unavailable when the official harness lacks a field
required by the exact dated price table. Codex0.159.3 supplies cumulative input,
cached input, output and reasoning, but no cache-write/context-price-band
breakdown for the supported `gpt-6.1-sol` model. The runner retains known tokens
with null cost; it never invents zero cache writes or selects a band. Round 3 supersedes the exact-dollar gate with primary tokens and the explicit
bounded proxy mapping above; the native exact quote remains unavailable.

## MP-08 / MP-10 — archived round 2 campaign state

The exact89 /50 task pins below remain unchanged. Codex smoke10 precedes each
full run, with serial tasks and fresh plan-exhaustion checks. Terminal-Bench
profile placement is awaiting a coordinator choice between the official Harbor
same-host volume with normal product linking and a managed worker binding.
SWE-bench's solver checkouts use the approved local profile directly. The real
CLI process cwd and explicit workspace/worktree flags bind each clean pinned
instance checkout. `swe_campaign.py` preserves the exact denominator, serializes
tasks, binds campaign provenance, checks resources and stops on fresh quota or
incomplete cleanup. The first interrupted task attempt is retained with its
solver edits, unscored; it was not reused as a clean base. A subsequent clean-base
admission failure is retained separately. Valid smoke and full tasks use separate fresh checkouts; no smoke outcomes
are reused in the full denominator.
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
price, plus shared client formatting/protocol checks. The prescribed Node client suite passes1117/1117. Rust accounting/persistence/aggregation checks pass12/12, and focused protocol
snapshot checks pass183/183. The real kernel and relay build passed under the
shared compile slot with two jobs and kernel debug symbols disabled. Targeted
CLI notice/session checks pass14/14; CLI typecheck/build pass. These establish
component behavior; the separately recorded real numeric drill establishes its
limited live scope. Official SWE smoke completed10/10 and resolved10/10, with0 evaluator errors,
all solver cleanuptrue, no remaining run containers, and all10 newly pulled task
image tags removed by exact identity after confirming no container used them.
Solver wall time totals842.50s;0/10 tasks have an authoritative USD quote. This
is a local smoke reproduction, not a full score or MP acceptance. The actual
CSV/summary/plot is generated from frozen campaign and official scorer receipts;
missing USD produces no cost/accuracy point.
The historical0-run diagnostic CSV/plot is not a zero-cost/zero-accuracy result.

MP-08 / MP-10: the full SWE Verified Mini baseline completed 50 independent
solver turns and officially resolved 44/50 (88%), with six unresolved, zero
scorer errors, zero empty patches and zero unstopped task containers. The scorer
guard removed only its 50 newly pulled, unused task image tags. The unchanged
official v4.1.0 evaluator run is
`chariox-evals-full-2c6c95cdf4534b82b62bc50470886fa2`. Every task used frozen
product source `82a444a77` and kernel SHA `f25b48a1...`; subsequent allocation
guard commits are not relabelled as that runtime. All 50 real TUI usage captures
match the final native Codex input/cache/output/reasoning counters, and all
runtime cleanup completed. Solver wall sum is 4950.44s, mean 99.01s, median
94.80s and nearest-rank p95 is 191.08s. No task has an authoritative USD quote.
This is a local unpriced Codex baseline; ordinary/managed/provider parity and
matching dollar acceptance still require their own drills.

The six unresolved tasks remain in the denominator. Post-scoring analysis
identifies patch defects: django-12193 fixes the array caller but leaves
CheckboxInput mutating attrs; django-12273 leaves saved inheritance links when
copying with pk=None; sphinx-7590 encodes incorrect C++ literal IDs;
sphinx-7748 adds a singleton signature continuation; sphinx-7985 emits an extra
existing-file link result; sphinx-11510 adds include-read instead of extending
the expected source-read behavior. Official failed-test IDs, patch hashes and
test-log hashes are retained externally. No scored patch was retried after
seeing verifier results.

MP-11: a wider allocation audit corrected stale current-version 435 assertions,
publication defaults and the public-provider drill stamp to 448. Fail-first
Node/CLI/publication checks were retained. Focused 10 Rust guards, three actual
public-provider boundary checks, 14 Node guards, five CLI guards and four
boundary-script checks pass. Historical released snapshots and clients'
unchanged minimum versions retain their original identities. These checks
establish their focused security/protocol scope; the complete narrowed MP-11
parity matrix is not established by this baseline.

MP-08 / MP-10 / MP-11: Terminal task environment inventory exposed a native
loader blocker before any scored task: the host-built kernel requires
GLIBC 2.38/2.39 in Bookworm. Building the same frozen source in the project's
pinned Rust 1.88 Bookworm image produced a kernel with maximum GLIBC 2.34;
the actual native protocol probe changes from RED (exit 1) to GREEN (exit 0,
448). That binary still fails Bullseye (GLIBC 2.31). Bullseye bootstrap package
failures and missing-compiler exit 101 were retained; GPG-verified archived
packages repaired and verified the prerequisites. Its next build remained
queued and was settled before acquiring the shared compile slot, with no build
verdict. The public Codex executable's Bullseye version probe passed; bundled
zsh failed to load. Whether the actual task path selects that zsh is unverified.
No official task image was changed and no profile was mounted into these
builders. These are component probes; the real Terminal task flow requires
approved profile placement or a managed worker binding.

MP-11: all 60 completed solver checkouts were removed after preserving the
predictions and inventorying protected paths, including ignored files. The
single protected-name match was generated bytecode for tracked public Django
source, not an inspected credential. The interrupted checkout was also removed after proving its final diff exactly
matched its preserved, unscored patch. Own finished Bookworm and failed/queued
Bullseye compiler profiles and the exact builder containers were removed.
Public runtime binaries, frozen source pins, receipts and the interrupted
attempt's evidence remain available for replay. Shared caches, provider
profiles, protected reviewer state and other lanes were untouched.


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
