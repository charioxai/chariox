# MP-08/MP-10/MP-11 — phase19 protection guard and coordinator handoff

MP-11: the reproduced codec mask failure now has a fail-closed guard and passing component stress evidence. MP-08/MP-10 performance remains RED. Real live acceptance is BLOCKED on coordinator execution/access; none of these fixture runs closes an MP item.

MP-08/MP-10/MP-11 identities: base `b77569f2eef892305129380e2f2a8ee23b84a230`; runtime `0acaf81dbcb543be8da90e17b5ea67737071d518`; navigation verifier `40905d8c0a2cc68370659f0ad703c13c301220ad`. Local protocol447/relay90 unchanged. Only private mask metadata and encoder IPC changed; existing negotiated PNG/video/stripe shapes remain unchanged. Local commits use `[skip ci]`; no publication, CI or deployment occurred.

## MP-11 — first failing seam and fix

The valid baseline used the actual embedded phase18 ELF, whose controller assets match b775, with the new failure-retaining harness. Its receipt correctly records a dirty harness, independently binds the ELF SHA, and verifies materialized controller asset hashes. It failed on three protected presentations, maxima RGB75/71/73. The diagnostic adapter captured the actual offending H264 key packet before egress. A replay of its exact public raster proves source RGB0 reconstructs to RGB76, above the unchanged client RGB64 gate. This establishes a lossy codec reconstruction failure; it does not establish that raw protected text reached the codec, or rule out every other race.

The runtime checks protected BGR channels are black before encoding, then decodes the actual protected codec output before packetization. It rejects reconstruction above RGB32, leaving headroom below the client gate, and rejects uncertain geometry/decode results. A rejected row drops the entire stripe batch, retires every advanced reference, and recovers with exact protected PNG bytes. Mask changes reset row chains. Only protected rows need the additional decoder. Protected full-video follows the same check and uses verified software; the unprotected hardware route remains available.

Trusted mask geometry stays private through native copies, CDP screenshots, crops and thumbnails. Protected copies have no shared mutable raster lease. The fallback queue retains bounded immutable bytes and rejects oversize work. A private Symbol never enters client JSON. The verifier subsequently needed a fail-first correction: an always-negotiated exact PNG is a valid independent navigation fallback, while stale documents, tiles and dependent video/stripes remain rejected.

## MP-08/MP-10/MP-11 — measured component gates

|Condition|Cycles|Protected presentations|Violations|Result and limit|
|---|---:|---:|---:|---|
|Base native1080p DPR1|Stopped on failure|107|3|RED; failure PNGs retained|
|Committed runtime batch1|70|1883|0|PASS_LOCAL_COMPONENT|
|Committed runtime batch2|70|1946|0|PASS_LOCAL_COMPONENT|
|Committed runtime batch3|70|1912|0|PASS_LOCAL_COMPONENT|
|Canonical DPR2, corrected verifier|10|235|0|Privacy component passes; motion7.28fps is RED|
|Manual-credit variant|10|358|0|Privacy component passes; not a production cadence benchmark|

The 210-cycle gate has 5,741 checked presentations, 5,099 stripes, 340 tiles and 302 PNGs. Every cycle drives physical click motion, verifies exact settle, then forces reference loss and checks independent recovery. Screenshot pairs/diffs are retained per step; the canvas callback checks every protected presentation. All three batches use clean source0aca and actual embedded assets, without a script override. Three bounded70-cycle runs avoid the fixture launcher's240-second limit; an earlier87-cycle preview hit that deadline and is retained as RED, not counted toward210.

DPR2 uses1280x800 CSS/2560x1600 raster. Its final click/type P95 is88.6/92.0ms locally, but its motion7.28fps misses30fps. An earlier canonical DPR2 run reached ten zero-violation cycles then hit the PNG-only verifier error; it also measured5.12fps and click P95937ms. Both are retained. A separate invalid2560x1600 canonical configuration failed controller startup; that was a drill parameter error. The directory named `protected-cdp` is a historical label: `MD_WINDOW=0` changes credit handling, not capture. Its trace includes647 native captures and96 CDP protected verifications; it does not prove a forced CDP fallback.

Configured regressions:245Node controller tests (zero skips),5Python protection,6stripe,6encoder,16client/presenter,7navigation/settle pass. Rust checks:26host,3protocol,2native-packet and1transient-event pass; five host tests are ignored by that focused invocation. The display launcher separately runs its ignored exact protocol drill. Release kernel/relay/developer CLI and self-contained Bun/OpenTUI `chariox` build successfully. Help/protocol checks establish artifact startup only.

## MP-08/MP-10 — profile and rejected performance experiment

Fresh same-host1080p/DPR1/8Mbps software fixtures remain RED against the coordinator's Selkies comparison. Values below are supplementary and host-load-sensitive; fixture rAF is not photon latency.

|Mode/workload|Pipeline cores during motion|Motion fps|Click P95 ms|Type P95 ms|
|---|---:|---:|---:|---:|
|Native stripes/canvas|1.873|57.80|51.1|52.3|
|Native stripes/docs|—|—|52.9|50.5|
|Existing full-video/canvas|1.917|58.10|40.0|53.3|
|Existing full-video/docs|—|—|57.6|35.7|

The bounded Python profile identifies codec encode as the largest active helper stage:0.704s versus0.370s self time in row preparation and0.064s conversion, across the complete canvas run.14.309s blocking readline is idle/wait time, not CPU work. V8 and capture profiles plus stage traces are retained; instrumented CPU is reported separately from the table.

The full-video experiment changes only the supplementary client's advertised stripe capability. It uses the existing720p software motion route scaled into1080p, with swscale. Its initial requested-libyuv comparison correctly failed converter provenance; the corrected run records the actual route. It shows no consistent CPU/latency win and lowers motion resolution. The patch is retained externally, the temporary worktree/branch is removed, and no performance change is landed. Encoder work remains the largest unresolved stage. Builder2 has no supplied owner GPU endpoint; hardware results remain unmeasured.

## MP-08/MP-10/MP-11 — exact artifacts and commands

Evidence root: `<lane evidence>/phase19/`. Commands/exits, source and asset hashes, screenshot pairs, codec replay inputs/packet, failed previews, profiles, resource and cleanup receipts are retained there. `FINAL19.json` indexes the final conditions.

- Product archive: `product-kit-final.tar.gz`, SHA256 `72b1812efb8bd737aee994eec684becc5f3f1d0c5994de8b2d67e11c179f6c39`. Source0aca; real kernel/relay, self-contained TUI, public Node/Python/PyAV dependencies and libyuv. `PRODUCT_MANIFEST.json` pins every file. The included `chariox-cli` is a developer launcher requiring its build checkout; use compiled `chariox` for portable TUI drills. No app, account or runtime identity is bundled.
- Component archive: `component-kit-final/display-lan-kit.tar.gz`, SHA256 `50b8e6046517a142ba9c460067c92640a0d6dfc3c9095ab43e7b95983260b5f8`. Harness40905, kernel build0aca explicitly bound;58embedded assets checked. Integrity verifier passes. This is the supplementary harness entry, not the Cloud app.
- Lane binaries: `<lane state>/phase19/bin/`. Kernel SHA256 `955cefba83634893272e2a005e42ac94af2e38dcc17721abfc32c80c6fe9b0c9`; relay `d4c5492dd35351e2399091e416cea708c989fb7b2e1d3bcf9a3a3f3dcbc7ab43`; compiled TUI `869f00059bedae3234d927e34c0d455ea5cd23b8d6734fee61283142b27611a7`; test ELF `dc7be3ac0a2aa4a5fd17882cce71475764a21647cbc00855b15ec1fbb72e6031`.

Exact Rust build: toolchain exports from the lane rules, `CARGO_BUILD_JOBS=4`, shared target, then `the shared compile slot, cargo build --release -p chariox-kernel -p chariox-relay --bin chariox-kernel --bin chariox-cli --bin chariox-relay`. TUI build: frozen filtered pnpm install, `pnpm --filter @chariox/cli run build`, repository-pinned Bun1.4.2 `apps/cli/scripts/compile.mjs --target linux-x64 --outfile <external lane bin>/chariox --version 0.1.0-phase19`. Generated repository dist outputs were removed after packaging.

To reproduce the component gate on builder2, run `run-case.sh` from this checkout with `MP19_BINARY=<lane bin>/kernel-tests MD_PROTECTION_REPETITIONS=70`, three sequential distinct output names. The retained `run-final-mask.sh` gives exact arguments. For a Linux owner host, extract both kits outside repositories, use `sudo` preserving its non-root user's UID/GID, and run the component kit's `run-lan.sh` with `MD_CASES=local:canvas:8000000 MD_GEOMETRY=1920x1080 MD_PROTECTED=1 MD_DYNAMIC_PROTECTED=1 MD_PROTECTION_REPETITIONS=70 MD_SOFTWARE=1 MD_ENCODER=libx264 MD_STRIPE_WORKERS=1 MD_LIBYUV=<product-kit>/runtime/lib/libyuv.so.0`. Use `MD_GEOMETRY=1280x800 MD_DPR=2` for DPR2. This remains supplementary evidence.

## MP-08/MP-10/MP-11 — coordinator real-live actions; BLOCKED here

The phase19 prompt delegates real-live execution to the coordinator. This lane may not contact the hosted relay or Apps machine. Its supplied Cloudf6cfd006 lacks the aa9c44f8 real app/e2e stack and real entry flags. Required coordinator actions:

1. Bind the exact runtime artifacts to Cloud `md/display-web` aa9c44f8 on p1b, rebuild the real app entry with its actual feature flags, and supply/run the paired e2e command. Keep Cloud bootstrap-only and use the existing kernel/relay paths. No protocol number is requested for this unchanged shape.
2. Launch the product kernel as a non-root desktop user with a disposable external `CHARIOX_HOME`, normal product identity/pairing, real installed Chromium, `CHARIOX_KERNEL_BROWSER_DISPLAY=1`, `CHARIOX_KERNEL_BROWSER_MIRROR=1`, `CHARIOX_BROWSER_DISPLAY_GEOMETRY=1920x1080` or `1280x800`, and the chosen encoder dependency paths. The product kit's `runtime/bin` supplies Node/Python; set `PYTHONPATH=<product-kit>/pytools` and `CHARIOX_BROWSER_DISPLAY_PYTHON=<product-kit>/runtime/bin/python3`. Use the bundled loader/library path on older Linux hosts; do not disable Chromium sandboxing. Capture normal UI startup and each action.
3. Rerun the coordinator's unchanged masked gate against the exact runtime; retain failure evidence. Run real Wikipedia article and portal, GitHub repo, Google search results, major news, MDN, video and canvas service flows at DPR1/2 in the real desktop web client. Report per-site DOM-mirror coverage, screenshots, click/type P95≤150ms, streamed scroll≥30fps, and transport kinds. Do not replace the primary path with PNG polling.
4. Use the real hosted `wss` relay at about8Mbps uplink and measured RTT≥60ms, a real Chariox-linked official provider where agent behavior is exercised, the compiled TUI where that flow applies, and at least one multi-hour session. Record exact machines/builds/flags/accounts via non-secret product status, console captures, disconnect/stuck-view results and cleanup. A GPU comparison needs a supplied real machine endpoint.

Security-critical observation/codec guard semantic review is pending coordinator review. The absolute display-perf review inbox was absent after each commit batch; no approval is inferred. MP-11 non-security exact-blob review is outside the narrowed scope.

MP-11 cleanup:24component receipts, all24state/short-temp roots absent;1,286resource samples, minimum MemAvailable22.00GiB and disk88.46GiB. Two exact diagnostic directories and the experiment worktree/branch are removed.1,501,642,155bytes of own obsolete public kits/build outputs were removed. Final kits/binaries, evidence and lane build dependencies remain for coordinator use. Key stores/backups, linked profiles, shared reviewer state, foreign lanes, shared Cargo outputs and Docker resources were excluded. No provider credentials or durable owner keys were read, copied or printed.
