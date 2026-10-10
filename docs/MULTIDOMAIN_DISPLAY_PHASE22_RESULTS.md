# MP-08/MP-10/MP-11 — phase22 FINAL, RED_PERFORMANCE / BLOCKED_REAL_LIVE

Phase22 is not accepted or done. The review leak is fixed fail-first and component masking/exactness checks pass; one-core/50ms targets and all-metric Selkies parity remain RED. Real-app/hosted/GPU/public-site/multi-hour acceptance is BLOCKED by lane resource restrictions. Nothing is staged as accepted.

MP-08/MP-10/MP-11 source: required base f7dff63a24c4319a845206fc95e2ca40d205a5ad; branch md/display-perf; runtime commits 92d99135d then c2dbe63f0178c8c410eda1b5f9781ae1b4f91965. Final report commit changes documentation only. Protocol local 447/relay 90 and client contracts unchanged; no protocol allocation needed. Local [skip ci] commits only; no push/PR/GitHub CI/deployment/hosted contact.

## MP-11 — supervisor reclamation and unchanged protection gate

Actual inbox is the lane review inbox. Phase21 checked the wrong display-perf path; phase22 resolves the unhandled third #893 finding first. Kernel-owned Drop reclaims only exact encoder-six-alphanumeric/raster handoffs, opened through retained directory descriptors with no-follow, private owner/mode, regular-file, single-link and 16,384,000-byte bounds. Unknown contents, links, unsafe modes and oversized files remain untouched; no recursive production cleanup. Empty partial handoffs are reclaimed too.

Valid RED: reclaim-red-valid.log exits 101; reclaim-live-base-valid exits 1 on the phase21 ELF, leaving a 16,384,000-byte raster/root after the owned supervisor is killed. Final exact release ELF/source c2dbe63f0: reclaim-live-final exits 0, raster exists before SIGKILL, remaining packet roots [] before harness cleanup. Kernel descendants settle and both state/temp roots are removed. Earlier live injection after graceful close and an earlier queued Rust run that saw restored source were invalid reproductions; they are retained as such, not counted as RED proof.

Canonical unchanged three 70-cycle gate: 210 cycles, 7,429 protected presentations, zero violations; exact settle, navigation retirement and reference recovery pass. Extra protected DPR 2 has 540 presentations/zero violations. Stripes are the primary motion path; PNG is exact settle/protected fallback. Configured Python tests separately cover full-video input/decoded-output guards. This is fixture regression evidence, not the real-app privacy gate or security-anchor approval.

## MP-08/MP-10 — dominant-stage changes and CPU profiles

Base 1080p kernel/credit work is the largest individual stage; protected DPR 2 controller Node raster work dominates. Whole-run V8 samples show DPR2 readSync 2.378 s, CRC32 1.875s and writeSync 1.577 s. Instruction-only base kernel perf records ownership inventory and Tokio work; it does not justify removing attestation. Avoid unchanged stripe copies by exact in-place XXH-prefilter+memcmp over admitted buffers, then copy only changed codec rows. Private COW mmap permits bounded temporary buffer addresses without exports surviving exchange closure. Native serials replace an extra full-raster CRC as scheduling hints only; exact comparison and both privacy guards remain. Small contiguous equally masked native damage reuses an immutable masked base; all other captures retain full read/masking. Negative-only busy-encoder credits avoid redundant CDP inventory; ready exact patches and retired policies still take the full path. Local empty-credit retry is 8 ms, input still wakes parked slots.

|MP-08/MP-10 profiled motion condition|Kernel|Controller Node|Codec|Native capture|Xvfb|Pipeline component sum|
|---|---:|---:|---:|---:|---:|---:|
|base-profile-1080p|0.49|0.34|0.36|0.12|0.05|1.36|
|base-profile-dpr2|0.24|1.01|0.73|0.23|0.13|2.35|
|final-profile-1080p|0.40|0.28|0.35|0.12|0.05|1.20|
|final-profile-dpr2|0.26|0.91|0.71|0.24|0.15|2.27|

MP-08/MP-10 profiles use Linux 100 Hz role/component ticks over nearest motion sample boundaries; whole-run V8/cProfile traces are separate. Instrumentation overhead is excluded from the final comparison runs. Base/final are both release builds; script-override previews use the old ELF and are explicitly separate. The base ELF was built at e4440628e6b3d4c54086d8ab6af0cadf70ce3049, SHA256 422e7cc1a2b582005bbbcc5b79ab91dc512d0da5f97e46600fa9402e9d82ebd5, with debug sections stripped. Its runtime tree is identical to required base f7dff63a2: that intervening commit changes only the phase21 report and lane/handoff documentation. These results are not relabeled to a new binary source. Shared-builder compilation/load can overlap sequential cases, so no sole-source attribution or hardware guarantee is claimed.

|MP-08/MP-10 final condition|fps|Pipeline cores|Click P95 ms|Type P95 ms|Exact settle ms|
|---|---:|---:|---:|---:|---:|
|1080p DPR1/canvas|59.65|1.12|54.90|54.80|1082.37|
|1080p DPR1/docs|—|0.74|55.30|37.40|1450.74|
|1080p DPR1/scroll30|29.84|1.08|55.90|54.00|1334.62|
|1080p DPR1/video|30.05|0.92|54.50|54.10|1071.61|
|1080p DPR1/wheel30|22.03|1.00|55.90|54.70|1375.42|
|protected DPR2/canvas|46.57|2.34|56.20|72.30|1605.19|

MP-08/MP-10: All final local cases independently reach exact idle fidelity. Canvas exceeds one pipeline core and both geometries miss the 50 ms input target. Scroll30 and wheel30 miss 30 fps. DPR 2 CPU shows no meaningful improvement over the base profile. No latency outliers were discarded. Docs CPU is measured during its click window; it has no continuous motion interval. The 8 Mbps value is a negotiated budget on the local component path, not a shaped hosted uplink.

## MP-08/MP-10 — fresh same-host Selkies software/GPU comparison

MP-08/MP-10/MP-11 — software 1080p DPR 1, 8 Mbps; RED_PERFORMANCE.

|Fixture|Backend|Click P50/P95 ms|Type P50/P95 ms|FPS/content FPS|Pipeline cores|Source/viewer cores|Moving Mbps|Live PSNR dB|Settled exact|
|---|---|---|---|---|---|---|---|---|---|
|docs|Chariox x264|38.60/55.30|34.50/37.40|—/—|0.74|0.20/0.41|—|—|yes|
|docs|Selkies2 x264|30.70/31.80|26.80/30.30|—/—|0.42|0.12/0.92|—|—|no (33.01dB)|
|canvas|Chariox x264|37.50/54.90|23.00/54.80|59.65/59.65|1.12|0.27/0.90|1.36|25.71–27.85|yes|
|canvas|Selkies2 x264|30.30/48.70|29.10/36.10|61.99/59.69|0.43|0.28/0.89|0.62|30.59–30.59|no (41.67dB)|
|video|Chariox x264|37.70/54.50|52.90/54.10|30.05/30.05|0.92|0.22/0.70|0.74|40.92–41.61|yes|
|video|Selkies2 x264|30.60/48.30|29.90/34.20|61.81/29.95|0.40|0.13/0.86|0.53|41.70–41.70|no (41.70dB)|
|scroll30|Chariox x264|45.10/55.90|50.60/54.00|29.84/29.84|1.08|0.27/0.92|3.71|15.22–15.56|yes|
|scroll30|Selkies2 x264|31.30/48.10|29.50/35.00|61.76/61.56|0.50|0.17/0.94|6.79|14.51–14.54|no (30.77dB)|
|wheel30|Chariox x264|24.20/55.90|46.00/54.70|22.03/22.03|1.00|0.45/0.72|2.73|16.75–16.95|yes|
|wheel30|Selkies2 x264|31.50/31.90|29.80/46.30|61.35/49.18|0.61|0.26/0.99|7.38|14.79–14.98|no (30.93dB)|

MP-10 GPU table (owner Intel UHD620/iHD; each entry is BLOCKED, no hardware result claimed):

|Condition|Chariox VAAPI|Selkies VAAPI|Owner action|
|---|---|---|---|
|docs/canvas/video/scroll30/wheel30; latencyP50/P95, fps, cores, GPU/encoder, Mbps, PSNR, settle|unmeasured|unmeasured|Coordinator runs authorized owner laptop and real-app matrix|

MP-08/MP-10 scope: local component fixtures, real sandboxed headed Chromium, encrypted kernel+relay vs upstream direct local WebSocket; not hosted/real-app acceptance. Shared builder load may vary between sequential legs. Linux 100 Hz role-separated ticks; CPU target is pipeline. Canvas/rAF latency is software proxy. Live PSNR includes temporal drift; no frame-aligned codec claim. Full source/pipeline/viewer totals retained in JSON.

## MP-08/MP-10/MP-11 — checks, provenance, cleanup and live blockers

Focused verification: 131 Node checks with zero skips; 7 stripe, 6 encoder, and 6 protection Python checks; 13 focused release Rust checks (4 packet/cleanup, 3 protocol/hash, 2 process ownership, 3 scheduling, 1 transient byte isolation). The extracted production Rust seam's 4 checks are supplementary, not additional kernel acceptance runs. Fail-first perf/stripe/busy-credit/sparse-mask seams are recorded in COMMANDS22.md. Full release compilation uses flock, four jobs and CPU affinity 0–3, finishes in 16m 42s, and exits 0; the artifact is copied under the compile lock. The final campaign exits 0 and runs fresh Selkies2 last.

MP-08/MP-10/MP-11 exact release-test ELF SHA256 3eb9c9d34ece67c03d64150c5b976da1f350da22d9d9fc8c6b0e3ea308d7aa21, source c2dbe63f0178c8c410eda1b5f9781ae1b4f91965, clean tree at build and every final receipt. Final controller assets are materialized from that ELF and hash-verified against committed source; no override. This ELF starts the real kernel/local encrypted relay services with a development opener and shared component presenter in headed sandboxed Chromium. It is not the actual Cloud app/TUI/provider flow. Browser version, flags, source/client hashes, screenshots, console/kernel logs and command paths are bound in each receipt.

MP-08/MP-10 evidence: <lane evidence>/phase22/FINAL22.json, comparison22.json/md, profile-stages22.json, COMMANDS22.md, COORDINATOR_LIVE22.md and screenshots of each step. 55 recorded disposable roots are absent. All baseline owned namespaces/process inventories are empty and removed. Resource monitor: 428 samples, minimum 28.97 GiB MemAvailable / 69.29 GiB free disk; samples for each case retained. Own monitor stopped via marker; reproducible public binary/evidence retained. Shared Cargo, foreign work/processes, BuildKit/Docker, protected keys/credentials and reviewer state were excluded from cleanup.

MP-08/MP-10 coordinator action: authorize/supply actual Cloud app entry/flags and hosted Caddywss (this lane is forbidden to contact Cloud staging or hosted relay/Apps machines), real desktop/DPR2 and owner Intel UHD620/iHD GPU host. Run Wikipedia article+portal, GitHub, Google, news, MDN plus real video/canvas, shaped~8Mbps and measured RTT>=60ms, real official linked providers/TUI where relevant, screenshot/mirroring coverage and click/type/scroll metrics for each condition, plus a multi-hour session. Full mirroring,≤150 ms hosted input P95 and≥30 streamed fps required absent owner exception; local 50 ms/one-core target remains separately RED. Install permitted laptop packages exactly as COORDINATOR_LIVE22.md; no private owner keys/credentials on this builder. Do not substitute mocks or stage this as accepted.

MP-11 current semantic review of observation protection/owned cleanup anchors remains coordinator work; corrected inbox checks are not reviewer approval. Narrowed non-security per-blob review is not an open blocker. FINAL; lane stops after documentation commit.
