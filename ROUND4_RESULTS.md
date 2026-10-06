# MP-08/MP-10/MP-11 — WebVoyager round 4 (vision-allowed)

MP-08/MP-10: round 4 **vision-allowed** settled 632/632 eligible tasks: **347 wins / 621 valid / 11 invalid**. Eligible win rate: **54.91%**. The original 11 exclusions and all 643 frozen task strings remain unchanged.

| MP-08/MP-10 campaign | Wins | Valid | Invalid | Eligible win rate |
| --- | ---: | ---: | ---: | ---: |
| Round 3, text-only | 179 | 615 | 17 | 28.32% |
| Round 4, vision-allowed | 347 | 621 | 11 | 54.91% |

MP-08/MP-10 paired outcomes: **181 gained / 13 lost**, net +168 wins and **+26.58 percentage points** on the 632 settled paired tasks. Successful judge verdicts count only when harness and judge are both valid. The separate 10-task smoke passed **6 wins / 10 valid / 0 invalid** (exit 0); it is not pooled into these full-run numbers.

## MP-08/MP-10 per-site comparison

| Site | Eligible | R3 text-only wins/valid/invalid | R4 vision-allowed wins/valid/invalid | Unattempted | Paired delta (pp) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Allrecipes | 45 | 0/45/0 | 0/45/0 | 0 | +0.00 |
| Amazon | 39 | 9/39/0 | 12/39/0 | 0 | +7.69 |
| Apple | 43 | 30/42/1 | 30/43/0 | 0 | +0.00 |
| ArXiv | 42 | 34/42/0 | 36/40/2 | 0 | +4.76 |
| BBC News | 42 | 4/42/0 | 21/40/2 | 0 | +40.48 |
| Booking | 40 | 5/40/0 | 8/37/3 | 0 | +7.50 |
| Cambridge Dictionary | 43 | 0/42/1 | 36/43/0 | 0 | +83.72 |
| Coursera | 42 | 6/40/2 | 36/42/0 | 0 | +71.43 |
| ESPN | 44 | 0/44/0 | 0/43/1 | 0 | +0.00 |
| GitHub | 40 | 36/40/0 | 34/39/1 | 0 | -5.00 |
| Google Flights | 40 | 0/39/1 | 6/39/1 | 0 | +15.00 |
| Google Map | 40 | 0/30/10 | 20/40/0 | 0 | +50.00 |
| Google Search | 43 | 15/42/1 | 34/42/1 | 0 | +44.19 |
| Huggingface | 43 | 30/43/0 | 30/43/0 | 0 | +0.00 |
| Wolfram Alpha | 46 | 10/45/1 | 44/46/0 | 0 | +73.91 |

## MP-08/MP-10 deltas by frozen round-3 class

MP-08/MP-10: grouping uses the original round-3 class, including winning controls; a recovered task is not moved into a new baseline class. Detailed subclasses remain in `COMPARISON.json` outside Git.

| Original class | Paired tasks | R3 wins | R4 vision-allowed wins | Gained | Lost | Delta (pp) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| harness/runtime error | 17 | 0 | 10 | 10 | 0 | +58.82 |
| live-site drift (answer changed) | 40 | 0 | 0 | 0 | 0 | +0.00 |
| navigation failure | 229 | 0 | 123 | 123 | 0 | +53.71 |
| other | 4 | 0 | 0 | 0 | 0 | +0.00 |
| reading/extraction error | 68 | 0 | 47 | 47 | 0 | +69.12 |
| site blocked / anti-bot / captcha | 94 | 0 | 1 | 1 | 0 | +1.06 |
| timeout / step budget | 1 | 0 | 0 | 0 | 0 | +0.00 |
| winning controls | 179 | 179 | 166 | 0 | 13 | -7.26 |

## MP-08/MP-10 remaining top-three loss classes

MP-08/MP-10: classes describe reported seams in original answers and traces, not proven causes for every task. Representative screenshot reviews are retained externally; no verdict is rescored.

1. **site blocked / anti-bot / captcha — 102 losses.** Record the exact access blocker; stop at challenges or mandatory login without bypass.
2. **reading/extraction error — 72 losses.** Use fresh protected screenshots for labels/units and targeted text; verify every requested constraint before answering.
3. **live-site drift (answer changed) — 64 losses.** Keep frozen-task losses separate; request a separately versioned fresh-task campaign if dates/listings must change.

MP-08/MP-10: Allrecipes shows Cloudflare verification and ESPN shows CloudFront 403 in reviewed captures. No challenge, login or access bypass was attempted; the cause of the site block is unproven. Frozen historical dates and removed listings remain unchanged. Fresh tasks would require a separately versioned campaign.

## MP-08/MP-10/MP-11 invalids and continuation boundaries

- MP-08/MP-10/MP-11 `ArXiv--12`: posthoc total-tool-call budget; 82/80 total calls; original invalid retained.
- MP-08/MP-10/MP-11 `ArXiv--25`: posthoc total-tool-call budget; 83/80 total calls; original invalid retained.
- MP-08/MP-10/MP-11 `BBC News--16`: posthoc total-tool-call budget; 82/80 total calls; original invalid retained.
- MP-08/MP-10/MP-11 `BBC News--35`: posthoc total-tool-call budget; 84/80 total calls; original invalid retained.
- MP-08/MP-10/MP-11 `Booking--1`: provider_settlement; mutating_actions; original invalid retained.
- MP-08/MP-10/MP-11 `Booking--9`: provider_settlement; mutating_actions; original invalid retained.
- MP-08/MP-10/MP-11 `Booking--33`: cleanup; original invalid retained.
- MP-08/MP-10/MP-11 `ESPN--21`: provider_settlement; original invalid retained.
- MP-08/MP-10/MP-11 `GitHub--16`: room_create; original invalid retained.
- MP-08/MP-10/MP-11 `Google Flights--4`: cleanup; original invalid retained.
- MP-08/MP-10/MP-11 `Google Search--14`: provider_settlement; mutating_actions; original invalid retained.

MP-08/MP-10/MP-11: interrupted segments settled their in-flight peer before continuing only never-admitted tasks. Durable admission receipts, frozen selection/source hashes, unchanged original-row prefixes and recursively verified cleanup proofs fence every continuation. **No solver retry, judge retry, rescoring or replacement task.** Budget/lifecycle/cleanup pauses are retained as RED segment exits; they are not provider quota/auth unless the native solver/judge flags prove it.

MP-11: Booking--33 reported `attachment_not_found` after its Room/slice were already deleted. Independent absence checks settled ownership, while its original invalid remained. Google Flights--4 reported a Room-deletion transport error; restarting its own persisted kernel found the Room already absent. The remaining owned slice was deleted through the normal kernel API, and all owned processes/Docker/state were removed. The receipt-bound settlement used zero provider submissions. Neither finding establishes the transport-failure root cause. ESPN--21 was a native missing-session dispatch failure; the dropped prompt was never resent. GitHub--16 failed Room creation before a provider prompt, with successful owned cleanup.

## MP-08/MP-10/MP-11 setting and exact source identities

MP-08/MP-10: opt-in `recovery-vision-v1` permits privacy-preserving rejection of optional tracking/cookies and dismissal of optional sign-in/informational overlays, refreshes stale element IDs, checks requested constraints, and allows protected `slice_screenshot` image observations through the ordinary Room/kernel path. It permits no login, challenge solving, anti-bot bypass, arbitrary script/HTTP access or prohibited state change. Passive final capture retries are bounded to three fresh attempts. These joint changes are not causally isolated; live-site changes and judge variability can also affect paired controls.

MP-08/MP-10: both rounds use gpt-6.1-sol HIGH solvers, the same LOW official-prompt native-CLI substitute judge (120-second judge deadline), 15 mutating actions, 80 total tool calls, 600-second solver deadline, and at most two concurrent Rooms. Round 4 counts screenshot observations within the unchanged total-call ceiling. Agent execution is at the home kernel with a slice-backed browser, configured Room viewport 1024×768, pinned 2 GiB/1 CPU slices and sandbox compatibility disabled. This does not establish a separate worker-provider or full managed Path-1 parity matrix. Local Codex CLI is 0.159.3; round 3 did not record its CLI version, so byte-exact CLI parity is unproven. This is a local reproduction, not the published WebVoyager model/evaluator setting or a leaderboard result.

MP-08/MP-10/MP-11 source pins:

- Upstream task revision: `5a7896738c10bfb8b9edccce6bb0e0411f8ae569`.
- Controller revision: `fda30571dc26f3ee88da4c5835f1730220d85f6a`.
- Runtime fingerprint: `75ca3fea817039531370b5b3649ee330d9c3598cfa394e4bdde41598fe05a95d`.
- Local protocol: `423`; host kernel/relay origin: `164c35016`.
- Docker image: `sha256:e9790158dcb0942208dcae403ba03de2e87223af6bace073c64e817da1d6a69b`.
- Worktree base: `696019fb4f90b0ff414c920d1561ce5d4d58769c` on `bench/wvanalysis`.
- Smoke harness: `5a9d45dcf8dd4962c763effede36aa84953b10ca`.

| MP-08/MP-10 full segment | Harness commit | Cumulative settled | Child exit |
| --- | --- | ---: | ---: |
| full | `a6fbbbfb4ce23e2b0c96ed8ef4d622e02b241ab0` | 214 | 1 |
| full-continuation1 | `6ff5e9cf4670a9cca6ae22e1e2a32bb4cf9633ba` | 220 | 1 |
| full-continuation2 | `6ff5e9cf4670a9cca6ae22e1e2a32bb4cf9633ba` | 243 | 1 |
| full-continuation3 | `36f323b2c80355799e3996b40881b1143b7b06d5` | 359 | 1 |
| full-continuation4 | `40fb00a507d1528cad8e89b2b8ccd07838fab961` | 425 | 1 |
| full-continuation5 | `40fb00a507d1528cad8e89b2b8ccd07838fab961` | 516 | 1 |
| full-continuation6 | `40fb00a507d1528cad8e89b2b8ccd07838fab961` | 632 | 0 |

MP-08/MP-10/MP-11: unsigned development artifacts were reused. These results are not validation of signed G2, current OSS main, Cloud staging, a display-transport replacement, or the complete MP acceptance matrices. No protocol shape change or protocol number allocation.

## MP-08/MP-10/MP-11 checks, evidence and cleanup

MP-08/MP-10/MP-11: fail-first regressions followed by focused checks cover the observation allowlist, all 643 frozen prompts, quota/auth stop flags, no-replay admission fences, cleanup proof ownership, immutable prior invalids, recursive ancestry and ghost/cycle rejection. The continuation checkpoint passed 45 focused checks; the current ancestry change passed 31 affected checks plus two targeted mutation/ghost/cycle cases, with no skips. Checks are overlapping, not additive. No Rust or full web build was run.

MP-08/MP-10/MP-11: full-run resource minimum was **19.17 GiB MemAvailable / 79.68 GiB free disk**, from 72445 samples. Guard: 16 GiB / 10 GiB, above the builder memory floor. The smoke minimum was 23.36 GiB / 156.62 GiB. Protected screenshot tool calls: 2161; tasks requiring a final passive capture retry: 6.

MP-08/MP-10/MP-11: no host-port collision retries occurred. Recorded tool-name audits contain zero forbidden-tool rows; this does not independently verify the meaning of every click or the complete private-observation protection matrix. Final cleanup independently checked eight runtime roots/workspaces, 26 recorded processes, eight owner-kernel labels, 642 exact container names and 641 exact volume names. Its resource sample was 44.98 GiB MemAvailable / 76.01 GiB free disk.

MP-08/MP-10/MP-11: big evidence is under `/root/.codex/evidence/browser-resume-20260930/wvanalysis/round4/`; final raw rows/comparison are in `full-continuation6/`. Frozen selections, admissions, source manifests, tool/judge audits, original captures, command exit codes, resource samples, stop points and cleanup settlements remain there. `FINAL_CLEANUP.json` independently confirms every owned runtime root, workspace, launcher/kernel/relay process and exact owner-labelled/container-name/volume-name resource is absent. Shared linked profiles, borrowed images, toolchains, caches, keys/backups, other lanes and reviewer services were excluded. Runtime-generated identities and materialized disposable profiles were removed with owned state.

MP-08/MP-10/MP-11 executable commands (options and logs remain in external evidence/state):

```sh
node apps/cli/scripts/benchmarks/webvoyager-round4.mjs <lane-options.json>
node apps/cli/scripts/benchmarks/webvoyager-round4-report.mjs <evidence>/full-continuation6
```

MP-08/MP-10/MP-11: comparison and independent cleanup checks exit 0; full segment child exits are recorded above. Review inbox mapping is in operator-local `PUSH_READY.md`. No public leaderboard submission, push, PR, GitHub comment/CI, merge, deployment, Cloud staging contact, signing-key transfer or shared-service restart. A passing benchmark does not close the broader MP-08/MP-10 plan or MP-11 security-anchor review.
