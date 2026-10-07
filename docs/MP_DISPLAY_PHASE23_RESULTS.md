# MP-08/MP-10/MP-11 — display phase23 FINAL: performance RED, real live BLOCKED

Final report 2026-10-07T00:07:52.802818+00:00. Local commits only; no push, PR, GitHub CI, merge, deployment or Cloud/hosted-relay contact. This is a supplementary component campaign, not acceptance.

## MP-08/MP-10 resulting implementation

Protected native motion now hands an immutable leased raster to the private Python encoder, which masks only its COW mapping before input/output checks. Node does not materialize or spool a full raster for motion. Exact snapshots and bounded crop reads remain masked; fallback holds its lease until materialization finishes. PNG CRC remains checked through native zlib; the unshared/CDP fallback remains supported. No serialized client/protocol shape changed: local 447, relay 90.

Native exact preparation uses a 50 ms quiet deadline, while lossy/CDP verification remains 300 ms. The first 30 ms candidate mistakenly treated 30 Hz gaps as stops: 86 full exact captures during 10 s of video. Fail-first cadence check and the real component timing trace reproduced it; 50 ms eliminates those motion reads. Complete native exact bases reuse admitted contiguous patches, and a fitting whole PNG skips redundant tile preparation.

## MP-11 supervisor crash and current review

Coordinator 22:45 P2 mapped to commit `3b60b0798`: native capture pools and Xauthority now live in the existing kernel-owned private display packet root. Descriptor-pinned cleanup admits only fixed slot names, private owner/type/link/size bounds, without following links, recursing unknown contents or reading authority bytes. Partial pools are reclaimable. Durable Chromium profile/tabs remain separate.

Exact base drill: 49,152,000 capture bytes plus Xauthority remained after SIGKILL; encoder spool itself was reclaimed. Final exact ELF: all capture/authority files and packet directories disappeared before harness parent removal; durable profile remained. Six focused cleanup checks include link/mode/size/unknown/partial negatives. Allocation audit: SysV capture is IPC_RMID-marked; X resources follow owned Xvfb; encoder COW mappings close on every request; encoder spool/stripe packets and now native pool/authority are kernel-owned; profile/tabs are intentional persistence. Existing managed isolation masks `/tmp`, `/var/tmp` and custom process temporary roots.

Final 210-cycle gate: three 70-cycle runs PASS_LOCAL_COMPONENT, zero violations across 7,614 protected presentations. Separate protected DPR 2 and protected supervisor-crash runs passed. This does not close real-world real live masking acceptance or current semantic security-anchor review. Coordinator must review COW/mask/lease and FD cleanup/browser observation anchors; no non-security exact-blob audit is requested under narrowed MP-11.

## MP-08/MP-10 exact identities and methods

- Required base `7d09b7ae761f4cfe0912ca61eb957be35fb59db0`. Base ELF was built at `c2dbe63f0178c8c410eda1b5f9781ae1b4f91965`; embedded controller assets SHA-match `7d09`. Base branch docs identity is not relabeled as a new ELF build.
- Final kernel ELF build `ab4459858991252f0fbe6845ca126876cdff0d60`, clean candidate; SHA256 `ff21a907086526ccf7754512d1fde8d88a52d6c19f96a1022b694022574b6922`. Client/harness `f4e52111e0b146af9348bb31e55d9a1029a0ad68`; all embedded asset hashes checked, no script override. Both sources include the cleanup fix.
- Preview runs explicitly use source override and have separate receipts; final measurements use embedded assets. Window of four credits, 8 Mbit/s ceiling, software x264, one stripe worker, bundled libyuv for stripe comparisons; CSS 1920×1080 / DPR 1, protected CSS 1280×800 / DPR 2 (2560×1600 raster). Real scoped local relay, headless Chromium, focused MCP probe, actual decoder/canvas; not the Cloud app entry, real provider or physical desktop.
- Same-host upstream Selkies 2 comparison ran sequentially at 8 Mbit/s using the shared fixture. Wheel pages now have 600 sections to avoid reaching the bottom inside 10 s. Earlier 180-section wheel receipts are retained and excluded from before/after claims. Other fixtures remain unchanged.
- Raster timing samples are whole-run sampled self CPU across controller/pixel workers, not additive wall time or motion-window process cores. Motion metrics below exclude profiler runs. Other lanes may compile concurrently; host load/resource samples are retained.
- Presentation is software canvas/rAF, not photon timing. settle_present_ms is the presented frame later proved exact against independent RGB; settle_ms includes expensive screenshot/readback verification. Both are retained. Pixel-change FPS can include codec variation; source workload cadence is stated.
- A wrapper wrote the final build receipt to its parent directory. Its old/new Cargo executable paths were identical: the copied bytes were the correct new ELF under the compile lock and embedded-source guards passed. The initial wrong-ELF diagnosis was incorrect; artifact-copy-audit.json records the correction. Our safe SIGINT interrupted one video leg and skipped two wheel legs; all three reran successfully. Original interruption and erroneous diagnosis receipts remain preserved, not silently relabeled.

## MP-08/MP-10 final component measurements

| Condition | FPS | Pipeline cores | Click P95 ms | Type P95 ms | Exact presentation ms | Verification ms |
|---|---:|---:|---:|---:|---:|---:|
| 1080p canvas | 59.42 | 1.13 | 40.40 | 53.20 | 248.46 | 597.88 |
| 1080p docs | — | — | 56.40 | 36.60 | — | — |
| 1080p scroll30 | 29.84 | 1.04 | 56.40 | 54.20 | 893.57 | 1319.30 |
| 1080p scroll60 | 54.50 | 1.30 | 55.90 | 58.50 | 915.44 | 1303.91 |
| 1080p video | 29.96 | 0.86 | 41.50 | 55.80 | 238.72 | 617.28 |
| 1080p wheel30 | 29.90 | 1.23 | 42.00 | 58.30 | 914.60 | 1375.43 |
| 1080p wheel60 | 7.50 | 1.00 | 42.60 | 55.10 | 966.11 | 1355.44 |
| protected DPR2 canvas | 54.90 | 1.73 | 56.70 | 55.50 | 557.18 | 1533.62 |

MP-08/MP-10 targets remain RED: scroll60 delivered 54.50 fps at 1.30 pipeline cores; wheel60 delivered 7.50 fps at about 1 core. Exact presentation is below 300 ms for canvas/video, while scroll/wheel/DPR 2 miss. Click/type P95 does not stay below 50 ms across conditions. Valid component legs passed pixel, navigation, stale input, takeover and cleanup checks; performance and real live acceptance remain RED/BLOCKED.

ProtectedDPR2 improves from phase22 46.57 fps / 2.34 cores to 54.90 fps / 1.73 cores. Whole-video software: canvas 59.59 fps / 1.09 cores / 267.50 ms settle; scroll60 59.10 fps / 1.35 cores / 877.30 ms; wheel60 8.64 fps / 0.84 cores / 893.20 ms. It does not close parity. Whole-video conversion is through PyAV reformat; converter diagnostics are only emitted for stripes. Initial whole-video legs had an erroneous libyuv expectation; corrected final legs pass with auto conversion.

### MP-08/MP-10 fresh same-host Selkies 2

| Workload | Presented FPS | Pixel-changing FPS | Pipeline cores | Click/type P95 ms |
|---|---:|---:|---:|---:|
| canvas | 61.89 | 59.59 | 0.40 | 47.70/32.40 |
| docs | — | — | — | 31.90/30.00 |
| scroll30 | 61.72 | 61.52 | 0.51 | 37.90/34.10 |
| scroll60 | 61.90 | 61.30 | 0.53 | 48.30/37.20 |
| video | 61.74 | 29.82 | 0.37 | 32.20/30.50 |
| wheel30 | 61.83 | 61.43 | 0.59 | 48.40/35.70 |
| wheel60 | 61.96 | 61.16 | 0.55 | 37.60/36.00 |

Selkies baseline has local transport, no Chariox relay or Vault admission; it is useful parity evidence, not identical security/transport cost. Wheel60 source ends at 74,910 px on the extended page, within its scroll extent. Chariox wheel60 remains a real input/capture/delivery bottleneck; input-driven source retirement/credit cancellation is a follow-up hypothesis, not a proven sole cause. No guard was removed to meet FPS.

### MP-08/MP-10 re-profile

| Whole-run DPR2 sampled function | Exact phase22 seconds | Final seconds |
|---|---:|---:|
| Node readSync | 2.754 | 0.143 |
| Node writeSync | 1.716 | 0.071 |
| PNG CRC (JS before, native now) | 0.022 | 0.002 |

The supplied 1.9 s CRC figure did not reproduce on the exact phase22 snapshot: that source already uses native serial scheduling hints. No 1.9 s CRC saving is claimed. Native handoff regression asserts zero full Node copies/spool files; small masked region reads, exact snapshots, diagnostics and fallback still perform bounded I/O. Final 30 Hz video has zero exact captures during motion and 0.86 pipeline cores; the 30 ms intermediate had 86 captures / 1.15 cores.

## MP-08/MP-10/MP-11 checks, resources and cleanup

Final focused Node suite 123/123, no skips; Python encoder 7/7, stripes 7/7 (explicit libyuv), protection 6/6; exact full-kernel selected Rust 15/15 (six cleanup, three protocol, two process group, three display gate, one transient event). Fail-first mask handoff, native deadline/cadence, exact reuse, fallback lease and cleanup checks retained externally. Full release builds used the builder2 flock, four CPUs/jobs; no CI triggered.

Resource monitoring minimum MemAvailable 26.92GiB, free disk 67.38GiB, above the 12 GiB memory and 10 GiB disk floors. Audited 110 exact disposable state/temp roots and 62 namespaces absent. Per-run owned-process inventories empty; all lane helper/drill processes settled. Temporary base/candidate public checkouts removed. Retain exact public ELF, evidence and owner kit; no shared Cargo/BuildKit/Docker/provider/key/reviewer/other-lane cleanup.

## MP-08/MP-10 owner VAAPI kit

Unsigned public archive: `/root/.codex/evidence/browser-resume-20260930/display/phase23/final50/owner-kit/display-lan-kit.tar.gz` (245336336 bytes). SHA256`5fb7c08fe986a81ca1326222d072df29f9d94df1b60c3d7b7d7f8e5ecd148db2`. ManifestSHA256`c2abca690ee5d2d4b45787933d0ab5d8e2cda0b3c1541cf00cb43d8d31fa15e3`. Source `f4e52111e`, kernel build `ab4459858`; stripped derivativeSHA256`8198042d7e3311b0879876836e7a06ab34ba535044cb0f9b345c85ec6ae29029`, originalSHA retained. Public Node 22.20.0 / Python 3.14.4 / PyAV 16.0.1, loader/libraries, exact libxxhash/libyuv and source/client inventory; no state/credentials/keys. Manifest verification and runtime/CRC/native-library/kernel-enumeration smoke checks pass.

Included `OWNER_VAAPI.md` gives coordinator-run Intel UHD 620 / iHD software-vs-VAAPI whole-video commands at DPR 1/2, protected fallback, and per-case backend/timing/cleanup requirements. run-owner-v23.sh supplies exact SIMD library for stripe gates. Builder nodes identify virtio-pci, not owner Intel hardware; no owner-laptop VAAPI result is claimed. Hardware backend must be proven by successful per-case encoder traces, not requested flags/vainfo/device presence.

## MP-08/MP-10 real live BLOCKED — exact owner action

The lane is prohibited from Cloud staging, hosted relay/Apps contact and owner-laptop execution. Coordinator/owner must supply/run the authorized actual Cloud app revision/entry/flags and real CLI/TUI with the exact kernel, scoped hosted wss bootstrap, official product-linked Codex/Claude/OpenCode accounts, actual desktop DPR 1/2 and Intel laptop. Drive fixed real sites/services, shaped ~8 Mbit/s client uplink with measured ≥60 ms RTT, screenshots/console captures per step and a multi-hour stability session. No account credentials or private keys go into this kit.

| Mandatory real site/condition | DPR1 | DPR2 | DOM coverage/screenshot/input/scroll/hosted network |
|---|---|---|---|
| en.wikipedia.org article | BLOCKED | BLOCKED | unmeasured |
| www.wikipedia.org portal | BLOCKED | BLOCKED | unmeasured |
| github.com repository page | BLOCKED | BLOCKED | unmeasured |
| Google search results | BLOCKED | BLOCKED | unmeasured |
| Major news site | BLOCKED | BLOCKED | unmeasured |
| developer.mozilla.org docs | BLOCKED | BLOCKED | unmeasured |

Full mirroring, screenshot success, relay click/typeP95 ≤150 ms, streamed scroll ≥30 fps, absence of a PNG primary slideshow and multi-hour continuity are not established on those real conditions. No owner exception is assumed. Nothing is ready for staging acceptance.

## MP-08/MP-10/MP-11 evidence index

All evidence under`/root/.codex/evidence/browser-resume-20260930/display/phase23`. Final receipts/profiles/kit in`final50/`; FINAL23.json, summary.log, hot-io-comparison.json, profile-stages23.json, exact test-binary.json, kernel logs and paired screenshots per case. Parent retains base red crash,30ms intermediate, fail-first tests, source-override previews, interruption/correction audit, host-load/resources and owned-cleanup-audit.json.
