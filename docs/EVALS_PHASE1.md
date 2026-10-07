# MP-08 / MP-10 / MP-11 — evals phase 1, round 5

MP-08 / MP-10: input, cached input, output and reasoning tokens are the primary
cost metrics. These are local Codex baselines using official provider execution
and unchanged official scoring. Claude/OpenCode adapters are ready for the
coordinator's owner-side leases. Local results cover their stated execution
scope; ordinary/managed parity requires its own matrix cells.

## MP-08 / MP-10 — official results

| Benchmark | Official result | Solver wall seconds | Campaign wall seconds | Proxy USD interval |
| --- | ---: | ---: | ---: | --- |
| SWE Verified Mini, fresh full | 44/50 (88%) | 4,950.44 | Not recorded | 5.6229604–12.1132338 |
| Terminal-Bench 2.0, fresh smoke | 7/10 (70%) | 1,698.69 | 2,224.54 | 1.2024776–2.5021632 |
| Terminal-Bench 2.0, fresh full | 65/89 (73.03%) | 13,089.14 | 40,780.97 Harbor job wall; 53,438.58 elapsed | 9.4442332–19.6523254 (88 measured tasks; 1-task total unknown) |

| Benchmark | Input tokens (includes cached) | Cached input tokens | Output tokens (includes reasoning) | Reasoning tokens |
| --- | ---: | ---: | ---: | ---: |
| SWE full 50 | 18,013,227 | 16,588,544 | 111,474 | 8,110 |
| TB2 smoke 10 | 4,429,809 | 4,185,216 | 29,477 | 2,790 |
| TB2 full 89 (88 measured; 1 unknown) | 32,243,711 | 30,257,152 | 244,540 | 31,237 |

MP-08 / MP-10: The full campaign completed all 89 official tasks: 65 passes and 24 failures, no harness errors, complete cleanup. Three are settled provider failures: the official verifier still passed two, and scored `vulnerable-secret` 0 after a Codex `[cyber_policy]` rejection with no counters, so full-89 token and proxy totals stay unknown; the TB2 token row and proxy interval are the 88-task measured subtotal. The full 89 tasks are fresh attempts; no smoke
outcomes are reused. Official verifier outcomes decide the score, including
provider failures. Transport/account admission failures remain distinguishable
from scored failures. No scored task is retried after verifier feedback.
Solver wall sums the scored end-to-end turn receipts, including native runtime
startup and TUI interaction; it is not inference-only time. Harbor job wall sums
every retained task and archived attempt, including setup, verification and
shared compile-slot waiting. Elapsed time also includes idle pauses: 11,827
seconds from the authentication block to the owner-repaired resume, 112 seconds
before the interrupted-runner resume, and the earlier QEMU setup correction.

MP-08 / MP-10: external reports retain task CSVs, summaries and the
`cost-vs-accuracy.png` comparison. The plot places provider tokens per task (input plus output) first, alongside
proxy cost per task and official accuracy, with context-band/cache-write
uncertainty visible. Cached input and reasoning are already included once. An unmeasured
counter keeps the campaign total unknown; known subtotals and unknown-task
counts are reported separately. Such a plot marks a lower bound and states
that the total upper bound is unknown.

MP-08 / MP-10 / MP-11: evidence root is
`/root/.codex/evidence/browser-resume-20260930/evals/round3/`.
The final TB2 summary, task CSV and comparison plot (TB2 full, TB2 smoke, SWE
full) are in `../round5/tb2-full-report/`. `tb2-full-native-equality.json` binds
native counters, visible TUI usage and cleanup to all 88 measured receipts; the
unmeasured policy rejection is settled and cleaned up. The plot marks TB2 full
tokens and proxy cost as lower bounds. The earlier blocked report stays in
`tb2-full-blocked-report/`. Fresh smoke evidence is in `tb2-smoke-v2/` and its
report in `tb2-smoke-report-v2/`. The repriced SWE report is `swe-full-report-v3/`.
`full-runner-provenance.json` records the frozen adapter hashes. Review-local
runtime evidence is in `review-local-accounting/`; paired-accounting attempts
and the preserved rejection/deadline comparisons remain separate diagnostics.
Screenshots and captures stay outside this repository. The final reporting
source changes do not relabel earlier native receipts: those retain their
version-1 proxy, original adapter identity and frozen kernel. Offline final
version-2 reports use their unchanged measured counters.

## MP-08 / MP-10 / MP-11 — authentication block and resume

MP-08 / MP-10: the first `train-fasttext` attempt stopped at `profile_link`
(normal linked Codex account refresh, fixed `401`/`unauthorized` classes) before
task session/agent/prompt creation or verification. The admission-only probe
reproduced it. After the owner re-logged acct-686, `--resume-admission`
archived that receipt in `admission_attempts`. It accepts only a pre-prompt seam
with no prompt, usage, verifier or leftover resources. Quota and setup guards
still reject it. The first resume invocation stopped on the runner's own
memory reserve while queued for the shared compile slot, before Harbor started.
The reserve now stops only a started Harbor job, the lane-local slot launcher waits for
task headroom, and `--resume-interrupted` continued with no unrecorded Harbor
job. All 83 scored receipts were preserved and none was rerun. The six
remaining tasks ran once: `train-fasttext`, `tune-mjcf`,
`winning-avg-corewars` and `write-compressor` pass; `video-processing` fails;
`vulnerable-secret` is a settled `[cyber_policy]` provider rejection. Its
recorded failure class is `provider_error_unknown` because the classifier
reads error entries and Codex surfaced the policy as a notice; screenshots
show the policy text. Round-5 evidence is
`/root/.codex/evidence/browser-resume-20260930/evals/round5/`.

## MP-08 / MP-10 — dated, editable proxy mapping

The [mapping table](../apps/cli/scripts/benchmarks/evals-phase1/proxy-prices-2026-10-07.json)
records source URL, observation date, units, version and assumptions. The closest
public API model is the same published model:

| Provider model | Public API proxy model | Short input / cached / write / output per million | Long input / cached / write / output per million |
| --- | --- | --- | --- |
| `gpt-6.1-sol` | `gpt-6.1-sol` | proxy USD 2 / 0.10 / 2.50 / 10 | proxy USD 4 / 0.20 / 5 / 15 |

MP-08 / MP-10: source [official OpenAI API pricing](https://developers.openai.com/api/docs/pricing),
observed 2026-10-07. Reasoning is included in output once. Missing context-band
and cache-write fields remain unknown. Unreported writes are bounded between
zero and non-cached input; the lower endpoint does not assert zero writes.
The [official caching guide](https://developers.openai.com/api/docs/guides/prompt-caching)
states that cache writes replace the uncached rate. The version-2 proxy is
`(input - cached - writes) * input_rate + cached * cached_rate + writes * write_rate + output * output_rate`,
bounded over missing writes and short/long context bands. Original version-1
additive receipts remain preserved; final reports reprice their original counters.
MP-08 / MP-10: every dollar figure is labeled proxy; no point quote is emitted with missing
categories. Subscription, infrastructure, tools, Fast mode and regional charges
are outside the mapping. Native exact-dollar reports remain unavailable where
the official harness lacks required fields; the approved proxy replaces that
old exact-dollar gate for these eval reports.

## MP-08 / MP-10 / MP-11 — execution and provenance

MP-08 / MP-10: real Chariox TUI user actions submit each prompt to the real
kernel. Kernel-managed official Codex 0.159.3 runs `gpt-6.1-sol` at low effort.
The TUI attaches through a real encrypted loopback relay. Each task captures
setup, ready, settled and expanded usage screenshots plus terminal/console
logs, native counters, official results, commands and resource samples.
The numeric TUI projection is checked independently of task-scoped accounting.
Only the captured agent/prompt's run records and explicitly intended descendants
contribute to a task. Older tasks and unrelated active agents are excluded.

MP-08 / MP-10 / MP-11: frozen product source
`82a444a77f1456bf3fe75f0782ea4ee7a4d9cf78`, local 448 / peer 91. The coordinator
allocated these versions; this lane allocated none. SWE native kernel SHA
`f25b48a1f1985bf7ec1fed12732eacb83f227feb1773be57a727d1398e4cd294`.
TB2 packaged kernel SHA
`4a4bd16b0f2117dd73b9e8e63b0b20d4803c013132f2d2da18fd2b759e30a8eb`.
Public ELF loader/library packaging preserves the frozen code and records
original/packaged hashes separately. Official task images and verifiers are
unchanged. Compose is pinned to v2.39.4 in lane-owned tooling. Profile access
uses the approved product link and ordinary status refresh; adapters never
read or copy credential payloads. Harbor's writable-mount initialization changes
the profile directory to 0777; setup immediately restores 0700 on that exact
owned directory. Files below it are not altered by that restoration.

MP-08 / MP-10: the initial full adapter is
`abda7679dc88e7088e7769c41a44f40151d6394c`; the setup-only resumed adapter is
`e0e029f0a51f2c96b1db791dcfe63b13aae776cf`; the six final tasks ran with
`9f56b437d` (resume and reserve guards only). External module hashes and per-turn
runner hashes bind both. The task-turn accounting code and proxy mapping are
unchanged across that setup correction. The full benchmark runtime is not relabeled as a later
review kernel. Ordered task IDs and public revisions are in
[inputs.lock.json](../apps/cli/scripts/benchmarks/evals-phase1/inputs.lock.json):

| Input | Revision / identity |
| --- | --- |
| Official Harbor | `c803185a8b7c88c163abe48a22e9cea0bbd95e90` |
| Official Terminal-Bench 2.0, 89 tasks | `2fd12b88aafdd04a52c298e3940bcb189f9766d6` |
| Official SWE evaluator v4.1.0 | `726c5461e2ef52d83cf1ea2107870a8bb3328d57` |
| HAL-selected SWE Verified Mini, 50 tasks | dataset revision `b316c349947c29963fce3f4a65967c9807a4b673` |
| Mini parquet SHA256 | `f9ba19dea78884f1081355d2d8afb671899981f24180aa0c4c1aa14d2c23e855` |

MP-08 / MP-10: serial campaigns refresh genuine product quota state before new
work. Fresh rolling-limit exhaustion pauses the run; an empty purchased-credit
balance does not imply subscription exhaustion. Resume archives the exhausted
attempt, retains its official receipt, preserves scored tasks and creates a
new attempt after refreshing the plan window. Full results retain the exact 89
or 50 denominator. Diagnostic admission attempts are separate.

## MP-08 / MP-10 / MP-11 — RED-capable validation and limits

MP-08 / MP-10: real accounting base 435 completes its provider turn but fails
usage projection; candidate 448 displays counters matching native usage through
the built TUI, kernel and relay. SWE full 50 and TB2 smoke 10 each match all native
counters and clean up. All 88 measured full-campaign tasks have native-counter equality, visible numeric TUI reports and complete owned cleanup; the one unmeasured policy rejection stays unknown.
Real Harbor admission also reproduces native loader failure and the 0777 linked
profile rejection before the corrections. Ubuntu/Bookworm task admission and
Bullseye kernel/zsh loader probes pass afterward; full task receipts establish
the actual covered task environments.

MP-08 / MP-10: separate real unsupported-model and 20-second adapter-deadline
comparisons are RED on frozen base adapter cd0270bf8, which skips verification.
The corrected adapter receives official reward 0 in both: an unmeasured
rejection stays unknown; a deadline uses ordinary TUI Ctrl+C and settles before
verification. Incomplete last-observed counters do not become a final total.
Diagnostic scores are excluded from benchmark denominators. Authentication
and quota failures are not silently counted as ordinary model failures.

MP-08 / MP-10: the first full QEMU Alpine attempt was RED before provider or
verifier execution: live Bullseye security indexes referenced removed package
files (404). An exact same-image installer reproduces that first seam. The
corrected installer uses a signed, dated security snapshot
`20260903T220410Z` through a temporary source list, preserving the image's
configured sources and signature checks; its fresh-container package setup is
GREEN. [Debian bug #1147093](https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=1147093)
records the upstream issue. Setup resume archives the failure and accepts only
explicit absent provider/verifier execution, matching source pins and complete
cleanup. It rejects missing stage fields or any solver/oracle feedback. All
67 already scored full tasks are preserved; none is retried. Both resumed Bullseye QEMU tasks complete the real built
TUI/kernel/relay/provider path, with measured native/TUI counter equality,
ordinary official verification and complete cleanup. Their official task
rewards remain 0; setup admission success does not relabel solver failures.
The Alpine task reports 497,043 input, 466,816 cached, 2,812 output and 95
reasoning tokens over 153.01 seconds of solver wall.
The campaign wall includes the failed setup, correction and lock waiting.

MP-08 / MP-10 / MP-11: reviewer #907 findings have focused fixes: an indexed
prompt/run latest-usage lookup replaces history replay in the output path;
authorized leased projections persist usage at home before settlement;
benchmark attribution uses exact task bindings; quota retry preserves evidence.
Forty-nine focused Python checks pass in the final source. The new solver-wall
regression fails first when pre-prompt admission time is incorrectly counted,
then passes with only scored solver receipts included. The proxy
pricing correction also fails first on both the known-write point and missing-write
upper bound, then passes all 48 checks; repricing real SWE50 and TB2 smoke10
receipts preserves every non-proxy CSV column. Original reports remain archived. Six accounting
report Rust checks and one leased persistence check pass in a source-bound
freshly built/copied test binary. The stale shared-cache candidate test run is
explicitly invalid evidence; retained base RED and fresh GREEN receipts identify
exact source/artifact hashes. These changes add no wire shape. A separate real local turn through the review
kernel, built TUI, loopback relay and official provider passes with native/TUI
equality (18,346 input, 12,288 cached, 11 output, 0 reasoning tokens) and complete
cleanup. That local drill does not establish leased accounting parity.

MP-08 / MP-10 / MP-11: live leased accounting remains blocked before a provider
turn. Same-host product-generated kernels discover and approve one another;
both local profile links authenticate, but normal remote account refresh or
materialization rejects the home profile as unauthenticated. Implicit and
explicit real profile selections reproduce that seam and clean up. The
coordinator must supply a working product-granted home/worker profile binding
or repair that materialization path. No manual credential transfer, fabricated
identity or relay claim is used. The review kernel is source
`5afadbc277914763b1c052b1862a0bd2235494af`, SHA
`ac200905124d7569253f1c47fe88bf7d6f969c626a6e415403d5fb5296c3f21d`.
Its TUI runtime code is unchanged from the retained 82a client build; one
excluded test-only change is recorded. Passing source tests do not close this
live cell. Claude/OpenCode still await the coordinator's owner-side leases;
placement-agnostic adapters remain ready. Resume/delegation/managed/provider
parity requires its own behavioral matrix and current security-anchor reviews.
Non-security per-blob review is outside the narrowed MP-11 scope.

MP-11: evidence stays outside Git. Only exact owned containers, networks,
process identities and unused newly created image tags are removed. Unknown
volumes and other lanes' resources are retained. Signal helpers reject unsafe
PIDs and verify owned descendants' birth identities. Floors are 9 GiB
MemAvailable and 10 GiB root free, with larger guard margins and one shared Rust
compile slot. Host source builds held the slot. Starting after the already
launched `portfolio-optimization` task, the external Harbor launcher also holds
it across each task and verifier, covering task-internal Rust compilation.
Earlier task-internal compile exclusion was not recorded. The launcher change,
original official entry hash and acquisition events are retained in
`harbor-compile-slot-provenance.json` and `harbor-compile-lock-events.jsonl`;
o benchmark source, official task or scorer changes accompany this guard.
Campaign wall includes lock waiting. Own unused compiler output is removed after protected-path and
active-process inventory. Shared caches, profiles, reviewer infrastructure,
private assets and other lanes remain untouched. No pushes, GitHub CI,
deployments, staging changes or public submissions are performed by this lane.
