# MP-08/MP-10 — WebVoyager round 3 failure analysis

MP-08/MP-10/MP-11 update: the owner approved protected screenshot observations for round 4. The completed **vision-allowed** 632-task run is reported in [ROUND4_RESULTS.md](ROUND4_RESULTS.md): **347 wins / 621 valid / 11 invalid, 54.91%**. This document retains the earlier text-only baseline and 60-task diagnostic; its observation-setting question is resolved, and its sample estimate is separate from the measured round-4 result.

Analysis and targeted benchmark fixes; this diagnostic does not close an MP item or establish a leaderboard rank. MP-11 applies to signal/observation/credential protections, not an exact-blob review of all benchmark source.

Baseline: **179 wins / 615 valid / 17 invalid**, out of 632 eligible tasks (11 frozen exclusions). Eligible win rate 28.32%; valid-only 29.11%. Source controller `fda30571dc26f3ee88da4c5835f1730220d85f6a`, runtime fingerprint `75ca3fea817039531370b5b3649ee330d9c3598cfa394e4bdde41598fe05a95d`, protocol 423, image `sha256:e9790158dcb0942208dcae403ba03de2e87223af6bace073c64e817da1d6a69b`. Host Rust/relay binaries originate from `164c35016`; this is unsigned development evidence. Harness branch starts at `7e8c42450bdd42929b656f78c762d3af864e3416` (r2next lineage), not current OSS main.

The largest actionable seam is the benchmark's consent policy: **192 of 453 non-wins (42.38%)** stopped at cookie preferences. The completed paired sample improves from **16 to 28 wins out of 60**, with 11 consent recoveries. This supports the targeted prompt change; it does not establish a new full-campaign score.

## MP-08/MP-10 comparability

The retained upstream [WebVoyager README](https://github.com/MinorJerry/WebVoyager/blob/5a7896738c10bfb8b9edccce6bb0e0411f8ae569/README.md) distinguishes its vision solver from text-only accessibility-tree observations. Its running instructions also recommend manually refreshing outdated Booking/Flights dates. This campaign freezes the original task strings, keeps historical dates, and exposes only first-party text/browser tools to the solver; screenshots go to the judge. The approved LOW native-CLI judge substitutes for the upstream GPT-4V API evaluator. Egress region, no-login restrictions, exclusions, task refresh, model, vision input and evaluator must match before comparing any leaderboard score. The local CLI reports 0.159.3; the baseline does not record its CLI version, so byte-exact provider CLI parity is unproven. No current rank or equivalent-settings superiority is asserted.

## MP-08/MP-10 taxonomy and upside

Every non-win has one primary observed seam. Assignments use the original answer, validity flags, tool trajectory and judge explanation. Class examples are screenshot-reviewed; the 453 exported action/tool-input traces are retained in external `FAILURE_TAXONOMY.json`. Reported blockers do not prove every underlying product cause. Expired dates take precedence over consent because dismissing an overlay cannot make historical availability searchable. Missing data without evidence of removal stays in extraction. The table gives theoretical ceilings, not predicted gains.

| Primary class | Count | % of 453 non-wins | Maximum eligible-rate upside | Practical scope |
| --- | ---: | ---: | ---: | --- |
| site blocked / anti-bot / captcha | 94 | 20.75% | +14.87 pp | 91 anti-bot; 3 mandatory login. No authorized bypass; 0 targeted gain assumed. |
| live-site drift (answer changed) | 40 | 8.83% | +6.33 pp | 34 reported expired dates; 6 removed/unavailable configurations/pages. Dates unchanged; 0 targeted gain assumed. |
| judge disagreement | 0 | 0.00% | +0.00 pp | No independently verified disagreement; 616 stored verdicts match their final explicit label. Semantic candidate review remains bounded; no score adjustment. |
| navigation failure | 229 | 50.55% | +36.23 pp | 192 consent-policy, 19 currency/delivery, 18 element/navigation. Consent gives at most 192 additional wins (+30.38 pp); 34 are also flagged time-sensitive travel; excluding them leaves 158 non-travel consent candidates (+25.00 pp), before further blockers. |
| reading/extraction error | 68 | 15.01% | +10.76 pp | 68 missing/partial/incorrect answers; 34 Wolfram extraction losses include image-only values. Existing protected Computer observations are excluded by the frozen prompt; modality decision needed for much of this. |
| timeout / step budget | 1 | 0.22% | +0.16 pp | 1 self-reported 15-action ceiling; no proven 600 s wall-time stop. Budget remains fixed. |
| harness/runtime error | 17 | 3.75% | +2.69 pp | 17 invalid: 4 final capture, 2 room creation, 1 judge, 7 tool audit, 2 provider settlement, 1 cleanup. Passive capture repair ceiling 4 wins (+0.63 pp); validity recovery alone is not a win. |
| other | 4 | 0.88% | +0.63 pp | 4 prohibited task actions. Restrictions remain; 0 targeted gain assumed. |

## MP-08/MP-10 class verification

- **Site block:** Allrecipes--0 screenshot 6 shows persistent Cloudflare Verifying; trajectory is open/status/text and an honest stop. ESPN has 44 analogous CloudFront 403 losses. Google Search--27 reports Vercel checkpoint. Neither cookie rejection nor retry bypasses these.
- **Drift:** Apple--4 screenshot 12 shows M5/M5 Pro/M5 Max instead of the requested M3 Max. Booking--37 has an October/November 2026 calendar and a March 2024 request; past dates are disabled. Drift is an availability/configuration diagnosis, not proof that an old answer is wrong.
- **Judge candidates:** Amazon--7 claims size 6; the judge reports highlighted 4.5. Final screenshot 25 does not expose the size selector. Google Search--4 top ranks match, but lower-rank disagreement needs more visual proof. ArXiv--42 screenshot 31 has Support-Vector Machine, and both solver and judge acknowledge the exact-title mismatch. No evidence-backed override claimed.
- **Navigation:** BBC News--0 screenshot 11 shows an explicit I do not agree control; the original prompt caused the solver to stop rather than reject tracking. Amazon--3 screenshot 26 shows an undismissed delivery notice and a visible Featured sort; multiple retries did not identify the correct control. Visibility does not prove the kernel hit-test succeeded.
- **Extraction:** Wolfram Alpha--0 screenshot 9 visibly contains 11.2, while the solver receives headings and reports unreadable data. This is a frozen tool-policy/observation gap, not judge disagreement: the final answer actually omits the number. PDF-only ArXiv tasks have an analogous gap.
- **Budget:** Huggingface--22 explicitly stops within the 15-action ceiling while the inspected docs describe the reverse conversion. The row records 54 tool calls, exactly 15 mutating actions and 199.4 seconds; screenshot 37 shows the reverse-conversion documentation. This is a planning/action-budget stop, not a wall timeout.
- **Runtime:** Apple--5 completed 24 tools but strict final capture failed; Google Map--18 through 24 have zero-tool invalid turns. Original RED rows remain invalid. Root causes differ; passive capture retry only addresses the first seam.
- **Other:** Amazon--1 asks to save a product, but the permitted runner is read-only. Huggingface--1 inference submission is similarly out of scope. The rerun does not broaden these actions.

## MP-08/MP-10 highest-leverage fixes

1. **Agent prompt:** explicitly permit privacy-preserving consent rejection and dismissing optional sign-in/informational overlays. 192 direct consent losses make this the largest actionable class. Never accept optional tracking, log in, solve a challenge, or change the task. Implemented as opt-in `recovery-v1`; frozen default preserved.
2. **Agent prompt/tooling:** inspect overlays before retrying obscured/disabled controls, refresh IDs, wait once, read targeted sections, and verify every requested constraint and recency before answering. 18 element failures plus extraction/partial-answer losses are candidates; no invented answer or longer budget. Implemented.
3. **Agent prompt/tooling — observation-setting decision:** the frozen source already provides protected `slice_screenshot` with native MCP image output through the ordinary Room/kernel path. The benchmark excludes it with its blanket Computer-tool ban. Allowing only this read-only observer could address image/PDF losses without product code or a new protocol. This would change the solver observation modality, so the running 60-task comparison keeps the original ban. An optional owner setting question is pending; no raw benchmark CDP image is injected into solver context. No missing product screenshot capability is claimed.
4. **Harness:** retry passive final screenshots up to 3 times, retain every original failure, and require a fresh successful final capture. 4 original invalid rows are candidates. No provider or judge replay; implemented.

The remaining 19 locale/delivery losses are deferred findings. No location is invented and prices are not silently converted. Full tool-result payloads were not retained by round 3, limiting retrospective root-cause proof. Historical dates/removals need a separately disclosed future benchmark setting; this frozen task set remains unchanged.

## MP-08/MP-10 paired 60-task measurement

Deterministic SHA-256 task order, proportional allocation with at least one representative per nonempty subclass, and 16 winning controls. Original sample scores: **16 wins / 54 valid / 6 invalid**. Task IDs, exact task strings, stratum populations/weights, baseline hash and immutable source identity are frozen in external `SAMPLE.json`. This small stratified sample overrepresents rare invalid subclasses; raw sample delta is not a full-population score.

**Completed: 28 wins / 58 valid / 2 invalid**, compared with **16 / 54 / 6** on the identical selected tasks. Eligible sample rate: **26.67% → 46.67% (+20.00 pp)**; 15 gains, 3 lost controls. A successful judge verdict only counts when both harness and judge are valid.

| Original stratum, grouped | Tasks | Before wins | After wins | Before valid | After valid |
| --- | ---: | ---: | ---: | ---: | ---: |
| Winning controls | 16 | 16 | 13 | 16 | 14 |
| Consent-policy navigation | 17 | 0 | 11 | 17 | 17 |
| Other navigation | 2 | 0 | 0 | 2 | 2 |
| Site block / mandatory login | 8 | 0 | 0 | 8 | 8 |
| Live-site drift | 3 | 0 | 0 | 3 | 3 |
| Reading/extraction | 6 | 0 | 1 | 6 | 6 |
| Step budget | 1 | 0 | 1 | 1 | 1 |
| Harness/runtime | 6 | 0 | 2 | 0 | 6 |
| Forbidden task action | 1 | 0 | 0 | 1 | 1 |
| **Total** | **60** | **16** | **28** | **54** | **58** |

Same gpt-6.1-sol HIGH solver, LOW official-prompt substitute CLI judge, 15 mutating actions/80 Browser calls/600 s, controller, image, task strings and no-logins/read-only restrictions. The measured variant remains browser-only. Changes are the disclosed recovery prompt and final passive capture retries; continuation only fences untouched tasks. First 46 attempts use harness `b99ed2079daeb74770818355011f73c8edabc07c`; remaining 14 use `18dcbc705d387be3cafeb7494ba6b4cc8385a52a`. Both harness identities and all original receipts are retained. An optional protected-screenshot variant was not measured; the owner setting question remains unanswered.

The first segment stopped after 46 tasks when both ArXiv--13 and ArXiv--32 judges reached the unchanged 120-second deadline without a verdict. Both solver turns completed; retained failure flags show no quota or unauthorized indication. The underlying provider/network cause is unproven. Cleanup reported `attachment_not_found` on the expired attachment handles, while independent kernel inventories and physical resource checks confirmed their Rooms/slices were gone. Own kernel/relay processes stopped. An independent cleanup settlement removed the disposable identity state and workspace; the 46 original rows remain unchanged, including both invalid controls. Continuation admitted only the 14 never-submitted tasks. No solver retry, judge retry, rescoring or replacement task.

The consent gains are Coursera--32, BBC News--32, BBC News--31, Coursera--27, Google Map--34, Google Search--6, Cambridge Dictionary--16, Cambridge Dictionary--40, Coursera--39, Google Map--2 and Google Map--12. Screenshot/trajectory spot checks establish the relevant seam: Coursera--32 rejects cookies and reaches visibly selected Beginner/1–3 Months filters with the requested count; BBC News--31 passes the overlay and reaches a visible completed match result. Consent recovery still leaves six sampled losses, including subsequent site errors, unavailable travel dates and extraction/constraint problems.

Other gains are Wolfram Alpha--26, Huggingface--22, Google Map--21 and Wolfram Alpha--20. Wolfram--26 uses the existing Plain Text panel for composition; the answer still calls the conversion unverified, and the judge credits the conversion shown in screenshots. Its scored gain therefore does not prove full answer extraction. Huggingface--22 now gives the correct conversion direction within 13 mutating actions and 53 tools, versus the original 15-action stop. The two runtime gains come from fresh valid attempts; unchanged runtime artifacts and one successful run do not isolate a causal repair for those original seams.

Lost controls are GitHub--10 and the two unjudged ArXiv tasks. GitHub's answers both report the monthly price multiplied by 12 and explicitly leave a separate annual subscription price unverified. The LOW judge accepts that caveat in the baseline and rejects it in the rerun. This is evidence of evaluation sensitivity on an ambiguous requirement, not independent proof of the correct annual price. Both verdicts are preserved; no original non-win is retroactively reclassified or upgraded.

Google Search--6 exercises the harness fix: final `Page.captureScreenshot` fails with `CDP_TIMEOUT`, then the second fresh attempt succeeds and the run earns a valid win. Both that error and an earlier passive capture timeout remain recorded. Apple--17 recovers validity but remains a loss. Validity recovery and task completion are reported separately.

Prevalence weighting gives a diagnostic point estimate of **28.32% → 46.09% (+17.76 pp)** over the 632-task population. Rare subclasses have only one observation, controls contain judge noise, and the deterministic sample is small. This estimate is not a measured full-campaign score or a precise forecast. Recorded aggregate solver wall time increases from 3,988.4 to 6,222.7 seconds; it includes formerly invalid attempts with incomplete baseline execution and additional recovery work, so no efficiency improvement is claimed. No leaderboard submission.

## MP-08/MP-10/MP-11 reproducibility and cleanup

Evidence root: `/root/.codex/evidence/browser-resume-20260930/wvanalysis/`. `SOURCE_PLAN_REVIEW.json`, `FAILURE_TAXONOMY.json`, `SAMPLE.json`, class/paired screenshot reviews, fail-first logs, original and continuation campaigns, cleanup settlement, `sample60-continuation/COMPARISON.json` and `FINAL_CLEANUP.json` retain source identities, hashes, commands and results. Default frozen prompt is compared across all 643 tasks. The 26 focused tests cover classification, bounded recovery, admission/replay fences, original-failure retention, paired validity, source drift and cleanup ownership. No skipped tests.

Resource minimum across both segments: **24.64 GiB MemAvailable / 162.96 GiB free disk**, from 6,733 samples. Guard: 16 GiB / 10 GiB, stronger than the builder's 12 GiB memory floor. Both own runtime roots/workspaces are removed; launcher, kernel and relay PIDs are absent; Docker container/volume queries under each exact owner-kernel label return none. Shared provider profile, image, services, other lanes and caches were excluded from cleanup. No Rust compilation, protocol change, Cloud contact, signing key transfer, push, PR, CI or deployment.

The initial campaign command exits 1; the comparison and focused checks exit 0. The continuation campaign records completed60 and its launcher is absent, but the tool session identifier was not retained, so its OS exit status is unavailable; the receipt explicitly records that gap rather than inventing an exit code. Original baseline/selection and first46 result hashes remain unchanged. This diagnostic leaves MP-08/MP-10 acceptance and MP-11 security-anchor acceptance to their broader validation lanes.
