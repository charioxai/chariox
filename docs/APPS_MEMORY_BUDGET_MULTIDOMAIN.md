# MP-08 / MP-10: App-view memory on the multidomain browser

Four-view App-induced p95 RSS was **588.7 MiB on the native kernel browser** and **678.2 MiB inside the Room slice**. Both meet the 768 MiB target for this shared sparse fixture. Historical B1 and broader MP acceptance need additional evidence.

## Method

Base `6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1`; both runs at `4f26b57172a6efaf90841fa6e692fc654a3829d3` (local 443 / peer 86). Test binary SHA-256 `5c205d209549043dfe15cb7b765cff8a41942c8446fd6ea7875ee2554f01901d`. Production kernel `5a2d92578a1a782f3e6689f40b65b640d5352c86bd58be1e41d4b2ace14812f4`; relay `5ca8ef1204f8cbace862871d7adca49e60f9b9a67e84927f5f9dc4400a8f4976`. Unsigned measurement image `sha256:aa2b4769961f9caaffe97e25a1e1ce6dde635ec49e40d7281d85092ff0b90aa6`, source fingerprint `9b8ebcc35273d6ed2051ea038d5b5149d64fe62076f63a41f476f48d8d2fc0ba`. Build/capture commands and inputs are in the evidence receipts.

Ubuntu 26.04.1, kernel 7.0.0-31-generic, AMD EPYC-Genoa, 16 CPUs, 30.59 GiB RAM. Both paths use Chromium 147.0.7727.137 and 1280×800 Xvfb. Native kernel/browser run as UID 65534, controller Node 22.22.1. Room uses the pinned G2 Debian/browser base with current debug runtime/controller, 2 CPUs, default Node 22.23.2. This qualifies neither bookworm nor a release image.

The ignored `runtime::router::tests::user_app_views::memory_budget::app_view_memory_budget_drill` installs four copies of the existing signed MD integration UI with distinct installation origins and fixed ABI echo workers. Native views use `OpenUserAppView`, explicitly `kernel_browser`, with zero sessions/slices. The Room is bound before normal Docker provisioning and uses `OpenAppView`. No provider or owner credentials are used. The fixed worker ceiling is 600 s / 1000 calls; this does not measure production Node worker containment.

Four workers are admitted before the zero-view baseline. After 3 s warmup, idle and interaction windows last 20 s. Interaction observes the document before native click/text/Enter, or Room locator fill/click, then pauses 250 ms per cycle plus RPC latency. Each page must acknowledge more than its initial App-channel call. Native App DOM mirror subscriptions are intentionally denied (MP-11); no mirror observer/cache is installed.

Collect `/proc` VmRSS and smaps_rollup PSS at a target 100 ms. Sum simultaneous process values within each class, then nearest-rank p50/p95. Browser total includes browser, renderers, GPU, utilities, zygotes and crash helpers; controller/kernel/workers are separate. RSS double-counts shared pages; PSS apportions them and can vary with other lanes. Samples outside declared 20 s windows and missing reads remain in raw evidence but are excluded from percentiles. Budget attribution subtracts the minimum complete zero-view baseline, conservatively, rather than the baseline p95.

MP-08 / MP-10: the wrapper requires more than a zero child exit before reporting PASS. Validation must acknowledge all four distinct installations, include the expected native/Room flags, and reach the matching `finished` phase. Baseline idle, four-view idle and four-view interaction each need at least 20 complete positive browser RSS/PSS samples spanning at least 18 s inside their declared 20 s window, with browser and renderer processes present. The final phase must follow those samples. Both validation and phase files are retained; missing or unusable evidence produces RED/nonzero after cleanup. Rust's exact filter running zero tests cannot establish a measurement pass.

## Numbers (MiB)

Cells show p50 / p95. These are full browser totals; the v2 target is **App-induced** browser p95 ≤ 768 MiB in [release budgets](CHARIOX_APPS_RELEASE_BUDGETS.md).

| Placement | Views / activity | n; missing RSS/PSS; after-window | Full RSS | Full PSS | RSS max | App-induced RSS / PSS p95 |
|---|---|---|---|---|---|---|
| native | 0 / idle | 134; 0/0; 0 | 862.86 / 867.98 | 383.13 / 386.79 | 868.31 | — |
| native | 1 / idle | 128; 0/0; 0 | 994.95 / 1005.65 | 410.56 / 414.55 | 1005.67 | 142.86 / 31.43 |
| native | 1 / interacting | 126; 2/2; 6 | 1004.66 / 1025.02 | 417.31 / 433.38 | 1047.84 | 162.23 / 50.26 |
| native | 2 / idle | 123; 0/0; 0 | 1155.69 / 1160.43 | 456.97 / 460.07 | 1160.45 | 297.63 / 76.95 |
| native | 2 / interacting | 126; 0/0; 12 | 1166.08 / 1168.77 | 461.93 / 465.20 | 1169.14 | 305.98 / 82.08 |
| native | 4 / idle | 119; 0/0; 0 | 1437.11 / 1445.18 | 510.23 / 515.46 | 1445.31 | 582.38 / 132.34 |
| native | 4 / interacting | 117; 0/0; 18 | 1449.91 / 1451.46 | 516.37 / 517.62 | 1453.15 | 588.66 / 134.50 |
| Room | 0 / idle | 102; 0/0; 0 | 912.38 / 916.86 | 362.88 / 365.56 | 917.02 | — |
| Room | 4 / idle | 93; 0/0; 0 | 1541.05 / 1564.86 | 455.12 / 471.59 | 1568.46 | 653.48 / 109.12 |
| Room | 4 / interacting | 93; 0/0; 3 | 1575.85 / 1589.57 | 473.38 / 487.55 | 1638.07 | 678.19 / 125.08 |

Per-class simultaneous totals: each cell is **RSS p50/p95; PSS p50/p95**. Per-process distributions, process counts, maxima and censored reads are in each `summary.json`.

| Placement | Views / activity | Browser | Renderers | GPU | Controller |
|---|---|---|---|---|---|
| native | 0 / idle | 246.06/249.10; 150.14/152.29 | 172.89/173.11; 67.78/68.28 | 145.93/147.91; 81.28/82.28 | 67.42/78.15; 24.05/34.77 |
| native | 1 / idle | 254.85/258.01; 154.35/155.55 | 291.64/292.05; 92.39/92.91 | 154.65/156.06; 84.84/86.15 | 72.69/73.77; 26.91/27.91 |
| native | 1 / interacting | 255.14/270.92; 153.89/169.40 | 300.13/301.09; 99.24/100.22 | 155.40/155.57; 85.13/85.25 | 76.97/77.62; 31.13/31.78 |
| native | 2 / idle | 280.84/282.84; 176.52/178.06 | 417.27/418.16; 115.35/115.96 | 163.42/165.12; 88.70/89.58 | 78.30/78.49; 32.45/32.62 |
| native | 2 / interacting | 281.22/281.34; 176.73/176.80 | 426.48/429.33; 119.76/122.73 | 164.10/164.45; 89.08/89.23 | 79.78/83.55; 37.58/76.10 |
| native | 4 / idle | 294.66/298.48; 187.13/189.95 | 666.21/667.50; 152.02/152.91 | 181.66/184.63; 97.74/100.10 | 84.08/84.69; 76.63/77.25 |
| native | 4 / interacting | 294.09/294.83; 186.84/187.18 | 680.95/683.02; 158.32/160.27 | 182.47/183.18; 98.19/98.52 | 88.48/94.15; 81.03/86.71 |
| Room | 0 / idle | 294.78/297.62; 175.22/176.83 | 214.24/214.26; 78.16/78.18 | 72.42/74.12; 22.02/23.05 | 64.72/68.41; 42.38/46.09 |
| Room | 4 / idle | 327.16/337.92; 193.18/200.20 | 771.16/772.91; 147.88/149.88 | 108.47/119.77; 37.55/44.99 | 69.66/69.77; 47.29/47.41 |
| Room | 4 / interacting | 335.04/346.60; 199.63/211.16 | 795.55/798.89; 157.20/161.80 | 109.93/110.49; 38.60/39.23 | 72.64/73.06; 50.27/50.69 |

Four-view renderer distribution (per process, MiB):

| Placement / activity | Processes min–max | RSS p50 / p95 | PSS p50 / p95 |
|---|---|---|---|
| native / idle | 6–6 | 118.03 / 129.39 | 25.81 / 35.52 |
| native / interacting | 6–6 | 124.05 / 130.79 | 28.32 / 36.22 |
| Room / idle | 6–6 | 139.29 / 139.69 | 27.47 / 27.87 |
| Room / interacting | 6–6 | 143.92 / 146.44 | 29.27 / 31.21 |

Minimum baseline RSS/PSS: host 862.797/383.119, slice 911.379/362.466 MiB. Median sampling intervals: host-8 159.2 ms, slice-3 197.1 ms. App-channel calls by view: host-8 [36, 18, 7, 7], slice-3 [22, 22, 22, 22]. Both runs exit 0 and cleanup PASS.

## Decisions and limits

Worst four-view attributable p95 RSS is **588.664 MiB native** and **678.191 MiB in the Room**. No production memory reduction is justified by this fixture.

Four distinct App origins add four renderers to the two-renderer baseline. Native four-view renderer p95 grows from 173.11 to 683.02 MiB RSS; the ~510 MiB increase dominates the 589 MiB attribution. Available renderer status reads have NoNewPrivs=1 / Seccomp=2; namespace depth is 2 native and 4 Room. Native had 6 and Room had 5 missing status reads, retained as censored protection evidence. No sandbox-disabling flags were seen. Four distinct App origins retain separate renderers. The renderer aggregate is the dominant growth; browser/GPU growth is shared, and controller memory is separately reported. Sparse static UI assets provide no evidence of a large cache contributor. Renderer sharing across these origins would weaken installation isolation; hidden-view discarding would change lifecycle behavior. Neither is implemented. The native import-closure fix adds two missing shared protection dependencies; it changes no protocol shape, sandbox flag or admission policy (MP-11).

Historical B1 harness/fixtures and the raw 806 MiB receipt were absent from the frozen tree; a coordinator ask remains recorded. This cannot establish a like-for-like B1 improvement. It excludes animated/large Apps, slow broker consumers, real Node workers, Web/TUI display, Mac and managed Path-1 parity. The OS/Node differences and shared builder load also prevent attributing all placement differences to multidomain alone. No MP item is closed by these runs.

Earlier RED seams are retained: async blocking bootstrap, fixed-worker /tmp, closed peer event receiver, missing native import modules, stale unobserved document, forbidden App mirror, provision-before-Room-binding and invalid Room execution IDs. Initial full-symbol test compilation was aborted after the resource guard observed RAM below 9 GiB; the value at that trigger was not retained. Final builds use 2 jobs / test debug=0 with an 11 GiB buffer; sampler buffer 10 GiB. Final measured floors and exact cleanup are in receipts. No foreign resources/signals or shared caches were used for cleanup.

Evidence: `/root/.codex/evidence/browser-resume-20260930/appsbudget/`: `host-8/`, `slice-3/`, `comparison.json`, build9 and image9 receipts, focused fail-first checks, resource samples and final cleanup inventory. Reproduction command: `python3 scripts/apps-memory/measure.py --binary <captured-test-binary> --output <external-evidence> --topology host|slice --image <pinned-measurement-image>`; host also supplies `--chrome <sandboxed-Chromium-launcher> --library-path <matching-runtime-libraries>`. The shared compile lock, source-fingerprinted image recipe and exact invocation arrays are recorded there.
