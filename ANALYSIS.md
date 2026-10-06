# MP-08/MP-10 — WebVoyager round 3 failure analysis

Analysis and targeted benchmark fixes; this diagnostic does not close an MP item or establish a leaderboard rank. MP-11 applies to signal/observation/credential protections, not an exact-blob review of all benchmark source.

Baseline: **179 wins / 615 valid / 17 invalid**, out of 632 eligible tasks (11 frozen exclusions). Eligible win rate 28.32%; valid-only 29.11%. Source controller `fda30571dc26f3ee88da4c5835f1730220d85f6a`, runtime fingerprint `75ca3fea817039531370b5b3649ee330d9c3598cfa394e4bdde41598fe05a95d`, protocol 423, image `sha256:e9790158dcb0942208dcae403ba03de2e87223af6bace073c64e817da1d6a69b`. Host Rust/relay binaries originate from `164c35016`; this is unsigned development evidence. Harness branch starts at `7e8c42450bdd42929b656f78c762d3af864e3416` (r2next lineage), not current OSS main.

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

Rerun in progress. An optional protected-screenshot variant is a separate observation-setting decision; this measured variant remains browser-only. Same gpt-6.1-sol HIGH solver, LOW official-prompt substitute CLI judge, 15 mutating actions/80 Browser calls/600 s, controller, image and no-logins/read-only restrictions. The only changes are the disclosed recovery prompt and final passive capture retries. Fresh lane-owned kernel/relay/Rooms; resource guard 16 GiB MemAvailable/10 GiB disk (stronger than builder 12 GiB floor). No leaderboard submission.

## MP-08/MP-10/MP-11 reproducibility and cleanup

Evidence root: `/root/.codex/evidence/browser-resume-20260930/wvanalysis/`. `SOURCE_PLAN_REVIEW.json`, `FAILURE_TAXONOMY.json`, `SAMPLE.json`, fail-first log and 23 focused GREEN tests are retained. Default frozen prompt compared across all 643 tasks. New runtime roots reject cross-lane paths and path traversal. Original source/evidence are read-only. No Rust compilation, shared process/container/image/cache cleanup, Cloud contact or signing key transfer. Final resource and owned-cleanup receipts will accompany the run.
