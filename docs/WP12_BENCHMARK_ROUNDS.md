# MP-08 / MP-10 — WP-12 benchmark audit amendment

Owner decision: 2026-10-02, recorded by the coordinator for benchinv. This
amends the benchmark requirements in BROWSER_COMPUTER_USE_END_TO_END_PLAN.md
and the interpretation of the frozen planaudit REMAINING.md. It does not
rewrite or relabel earlier acceptance evidence.

| MP-08 / MP-10 audit item | Current requirement |
| --- | --- |
| BENCH-INVENTORY, BENCH-RULES, BENCH-EXCLUSIONS | Refresh all maintained relevant public benchmarks; pin rules and justify capability-based exclusions. Research preparation may proceed before functional acceptance. |
| BENCH-RUNNER, BENCH-BASELINE, WP-12 round 1 | After functional acceptance, one complete, valid, passing run on every included benchmark, regardless of ranking. Use production kernel, Browser Controller, Computer path, and official provider harnesses. Required seeds/repetitions and the full official evaluation set apply. |
| BENCH-FIRST, WP-12 round 2 | Optimize toward verified public first place under comparable rules later. Preserve correctness and rerun affected regression gates. |
| WP-12 dependency | Scored campaigns still wait for all applicable local and managed functional gates. |
| WP-12 and program/rollout exit | Neither benchmark round blocks closing or merging Browser, Computer control, or Path-1 VMs (MP-01 through MP-11). Other functional, security, exact-head review, resource, and cleanup gates remain mandatory. |
| WP-13 dependency on WP-12 | Benchmark ranking and round-1 completion do not block functional rollout/default acceptance or merge. Rollback-window and default-validation gates still apply. |

A passing campaign run means a valid complete execution and valid scoring with
retained outcomes, evidence, and cleanup. Unsuccessful agent task outcomes
remain in the score denominator; incomplete infrastructure runs, smoke tests,
subsets, modified graders, or incomparable environments cannot be reported as
complete. No numerical minimum task-success score is supplied by this owner
decision; obtain an explicit threshold before imposing one. Public submission
and verification follow organizer rules, with official scores distinguished
from local results. This amendment closes no MP item and asserts no public
benchmark run or score.
