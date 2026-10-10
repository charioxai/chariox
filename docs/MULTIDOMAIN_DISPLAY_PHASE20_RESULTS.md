# MP-08/MP-10/MP-11 — phase 20 performance results

MP-08/MP-10 performance is RED. The changes improve protected native capture, but the local targets and the supplied Selkies comparison are not all met. MP-11 masking component gate: 210 cycles, 7,146 protected presentations, zero violations. Real-live acceptance is BLOCKED here on coordinator execution, explicitly delegated by the phase 20 task. These fixture runs do not close an MP item or authorize staging as accepted.

MP-08/MP-10/MP-11 source: required base 7cf42bc9014994022cd47708eaaff29a729f1cb7; branch md/display-perf; runtime eeda110c7add45f332ba51a40df8d174ac6d032f, preceded by 715a1c770 and d3a66d79e. Local protocol 447/relay 90 unchanged. No serialized client/relay shape changes. Commits are local [skip ci]; no push, GitHub CI, PR, deployment or hosted access occurred.

## MP-08/MP-10 — stages measured before changes

The valid pre-change diagnostic uses the phase 19 ELF whose controller and connector source match 7cf, plus private timing instrumentation through an explicitly recorded script override. It reaches 6.88 fps at 1280x800 CSS/DPR 2, pipeline 2.12 cores, click/type P95 74.0/68.7 ms. It is supplementary, not a clean rebuilt 7cf artifact. Failed earlier timing attempts and a non-ELF preview are retained and excluded. `SUMMARY20.json` binds every run's source, dirty status, asset mode, binary SHA and actual worker count.

|Stage|Before P50/P95 ms|Final P50/P95 ms|
|---|---:|---:|
|native_xshm_get_image|1.40/1.97|1.22/1.79|
|native_readback_copy|0.91/1.43|0.95/1.50|
|native_fingerprint|1.47/1.96|1.45/1.98|
|native_region_fence|1.95/5.42|1.64/5.37|
|native_mask_copy_hash|14.43/27.28|6.01/13.74|
|codec_convert|0.15/0.24|0.12/0.21|
|codec_encode|1.93/4.56|1.42/4.07|
|codec_output_guard|1.72/8.39|1.51/8.33|
|codec_packetize|0.21/0.27|0.22/0.31|
|motion_encode|25.64/137.20|21.38/49.06|
|client_decode|0.90/7.70|1.10/8.60|
|client_present|2.10/14.30|9.50/14.60|

Stage distributions cover each full run, including its different protection-cycle count. They are not a controlled per-stage speedup proof. Convert/encode/guard rows are individual stripe operations; motion_encode encloses helper IPC and the batch. Nested spans must not be added. Client timings are canvas/animation-frame proxies, not physical photon measurements.

Native mask copy/hash was 14.43 msP50 before changes; hashing 16 MiB with SHA256 contributed about 9 ms. The scheduling CRC is over already masked bytes. It is neither attestation nor an equality proof; encoded stripes still compare exact row bytes, and native exact damage is never suppressed on a hint collision. The next dominant cost was transferring immutable masked rasters through the helper pipe. One bounded private snapshot file uses the existing private mmap request; exchanges serialize writes until the helper closes its mapping and replies. The raw capture lease is never forwarded for protected frames. A second 16 MiB allocation was removed by reading directly into the detached mask copy.

A later trace isolates 111–115 ms whole-frame masking passes during a 1.4 second motion stall. Filling every pixel separately is replaced with a native fill per row, keeping the same RGB 0/alpha 255 and outward rounding/padding. A six condition byte-exact comparison passes; the whole-frame DPR 2 microbenchmark falls from about 99 ms to 1.06 ms, allocation excluded. The dirty preview improves to 27.82 fps. Existing codec input and decoded-output guards remain enabled. Exact settle remains independently verified.

The presenter wakes parked credits after input acknowledgement as well as before dispatch. Display-enabled event batching uses the existing bounded event lane with zero additional coalescing delay; ordinary mode retains 33 ms and control priority. Input-window queue-to-viewer P95 is already 2.4 ms in the baseline and 3.2 ms at D3, so these measurements do not establish a batching speedup. No client authority, relay/session authority or provider execution path is added.

## MP-08/MP-10 — final fixture measurements

|Condition|Motion fps|Pipeline cores|Click P95 ms|Type P95 ms|Exact settle ms|
|---|---:|---:|---:|---:|---:|
|Protected DPR 2/canvas|31.11|2.37|56.90|901.50|952.42|
|1080p/canvas|58.35|1.97|55.00|52.00|663.90|
|1080p/docs|—|—|54.70|46.60|—|
|1080p/scroll30|29.35|1.70|40.30|55.80|1265.07|
|1080p/video|30.19|1.57|50.30|52.80|872.11|
|1080p/wheel30|29.37|1.59|55.10|67.60|1184.23|

MP-08/MP-10: DPR 2 motion meets the 30 fps local fixture target. DPR 2 input,1080p pipeline CPU/input and the all-metric Selkies comparison remain RED. The final DPR 2 type outliers are 940.8/901.5 ms: the first contains a 786.4 ms host re-attestation span; the second contains 219.3 ms exact capture,58.6 ms tile preparation and 160.7 ms pacing. These overlapping spans locate unresolved work; they are not an additive latency decomposition.

Software x 264, libyuv, one stripe worker,8 Mbps negotiated budget, loopback relay. Motion spans are 3 seconds and sampling/readbacks reduce cadence. Shared-builder load makes comparisons supplementary. DPR 2 is 2560x1600 raster;1080pDPR 1 is 1920x1080 raster. Final conditions use the actual embedded controller materialization, verify 58 asset hashes, and use no script override.

The supplied phase 18 Selkies 2 software reference remains historical evidence, not a new Selkies run: canvas 61.92 fps/.43 pipeline cores/click 47.9 ms/type 35.9 ms P95; docs 31.4/29.8 ms; video 61.73 presentation/30.02 content fps/.40 cores/48.1/33.8 ms; scroll30 .54 cores/34.4/47.0 ms; wheel30 .59 cores/35.5/46.3 ms. Exact settle is a Chariox strength, but the requirement to beat Selkies on every metric is RED. No Selkies-specific implementation work was added.

Remaining measured seams include protected codec drops followed by exact fallback and expensive source retirement/re-attestation after protection-relevant DOM mutations. These defenses were retained. The earlier directory named workers 2-protected-dpr 2 did not run 2 workers: its wrapper forced 1. The corrected two worker D3 run reaches 19.19 fps, pipeline 2.17 cores, click/type 77.5/977.6 ms; it does not justify changing the default. All results retain their actual source and worker identity.

## MP-11 — final protection and regressions

|Final-head condition|Cycles|Protected presentations|Violations|Result|
|---|---:|---:|---:|---|
|final-eeda-protected-dpr 2|10|530|0|PASS_LOCAL_COMPONENT|
|final-mask-70-1|70|2317|0|PASS_LOCAL_COMPONENT|
|final-mask-70-2|70|2479|0|PASS_LOCAL_COMPONENT|
|final-mask-70-3|70|2350|0|PASS_LOCAL_COMPONENT|

MP-11: the canonical 210 cycle gate checks 7,146 presentations with zero violations. Its transport kinds and per-condition evidence are indexed in `FINAL20.json`. The extra DPR 2 ten cycle run checks 530 presentations with zero violations. All final-run disposable roots are removed.

Each cycle drives physical click motion, independently verifies exact settle, forces reference loss and verifies independent recovery. Every protected presentation is checked, with source/viewer/diff screenshots and console/kernel captures retained. Three 70 cycle runs respect the launcher's 240 second deadline. A passing component gate does not establish real-site or real-network acceptance.

Configured Node controller: 260 pass, zero skips. Python: 25 pass (5 protection, 6 encoder, 8 raster, 6 stripes), explicit files because hyphenated filenames are not discovered by unittest's default pattern. Client/presenter/stripe/navigation/settle/stages: 24 pass. Fail-first collision, stable-mask damage and detached-copy tests are RED on their pre-fix source and GREEN after. The private file reuse test also passes on the base, so it is a regression test without a red-capable claim. Row-fill equivalence and 39 focused checks pass.

MP-11: final copied ELF checks pass 26 host, 3 protocol, 2 native packet, 1 transient event, 27 connector and 6 envelope tests: 65 passed. Five host cases are ignored by the focused invocation; the display launcher separately exercises its ignored protocol drill. The initial broader connector run aborts on default stack overflow. RUST_MIN_STACK=16777216 resolves that test-runner limit. One unchanged obsolete test expects relay 73 accepted while the current admission floor is 90; isolated runs fail identically on 0aca and eeda (exit 101, false versus true). Its source is unchanged from 7cf. It is explicitly excluded from the 65 passing checks, with RED evidence retained; no blanket full-suite pass or semantic approval is claimed.

Shared Cargo target metadata from another lane twice supplied an incompatible App CallerContext.room_id:String to this checkout's Option<String> code. Retained builds fail at that seam. Rebuilding the own App library/invocation after timestamp invalidation under the compile lock resolves it; no App source behavior was changed. Binaries are copied while the slot is held, preventing the observed non-ELF linker-copy race. The corrupt preview is excluded and removed.

## MP-08/MP-10/MP-11 — coordinator kits and commands

MP-08/MP-10/MP-11 evidence root: `<lane evidence>/phase20`. `FINAL20.json` indexes commands/exits, source/build identities, stage traces, screenshots, integrity and cleanup; `SUMMARY20.json` contains all 23 diagnostic/final condition summaries.

- Product archive: `product-kit-final.tar.gz`, SHA256 `2802d1dd7e9eebc49d27cafb7b42cbf26e9c5e1b0f6118eb2898757598e9feb3`. Source eeda; real kernel/relay, self-contained TUI and public Node/Python/PyAV/libyuv runtime dependencies, 962 manifest files verified. TUI build remains 0aca (`apps/cli` and `packages/kernel-client` trees unchanged through eeda), with its original version `0.1.0-phase19` recorded. The developer Rust `chariox-cli` requires its build checkout; use compiled `chariox` for portable TUI runs. Help startup is verified, not real-user acceptance.
- Component archive: `component-kit-final/display-lan-kit.tar.gz`, SHA256 `2b880d643e66a945fe42ac9e715743e57d68f5e15291ef42d66762f980d39d23`. Harness and kernel build source eeda. Its test ELF is transformed by `strip --strip-debug`; original SHA `74f5907ac3f1068613895fc73abff145ef4770073349b2cfc457940c9fa6dafa`, packaged SHA `f848904eac7be80172df16c6f2b646f38230b52591da35425ad3ff49c2719a24`. All manifest files verify.
- Product kernel SHA `a56adbd3182a318a0d7322d2cf48ef61270defa374fc284d8086df01302c4908`; relay SHA `d4c5492dd35351e2399091e416cea708c989fb7b2e1d3bcf9a3a3f3dcbc7ab43`. Final build commands are retained in external `build-eeda-test.sh`/`build-eeda-product.sh`, run under the required compile flock with four Cargo jobs and the supplied shared toolchain/target. Public Node 22.20.0 is archive-pinned in the component manifest. No app, account or runtime identity is bundled.

Builder 2 fixture command (supplementary): `MP20_BINARY=<phase20 lane bin>/kernel-tests MD_GEOMETRY=1920x1080 MD_DPR=1 MD_PROTECTED=1 MD_DYNAMIC_PROTECTED=1 MD_PROTECTION_REPETITIONS=70 <phase20 evidence>/run-case.sh <unique-case-name>`, three sequential runs. `run-final-mask.sh` and `run-final-drills.sh` retain exact settings and exits. DPR 2 uses `MD_GEOMETRY=1280x800 MD_DPR=2`. `MD_CASES=local:docs:8000000,local:canvas:8000000,local:video:8000000,local:scroll30:8000000,local:wheel30:8000000` gives the 1080p comparison.

Portable component command: extract both kits outside repositories; from a non-root Linux desktop account invoke `sudo --preserve-env=MD_GEOMETRY,MD_DPR,MD_PROTECTED,MD_DYNAMIC_PROTECTED,MD_PROTECTION_REPETITIONS,MD_CASES,MD_LIBYUV env MD_SOFTWARE=1 MD_ENCODER=libx264 MD_STRIPE_WORKERS=1 MD_MEMORY_FLOOR_GIB=12 MD_CREDIT_WINDOW=4 <component-kit>/run-lan.sh`. Set `MD_CASES=local:canvas:8000000`, `MD_LIBYUV=<product-kit>/runtime/lib/libyuv.so.0`, `MD_GEOMETRY=1920x1080 MD_DPR=1`, `MD_PROTECTED=1 MD_DYNAMIC_PROTECTED=1 MD_PROTECTION_REPETITIONS=70`. Use 3 distinct runs. DPR 2 uses 1280x800/2. This command remains the fixture entry, not the real app.

## MP-08/MP-10/MP-11 — exact missing coordinator actions

The lane cannot contact hosted relay/Apps and lacks the coordinator's real Cloud app/e 2 e entry. Required actions:

1. Bind the exact product artifacts and updated presenter to the coordinator's real Cloud app source/entry and actual feature flags (previous coordinator identity aa9c44f8 on p1b; record whichever source is actually used). Rebuild that app and its paired e2e stack. Use normal product bootstrap/pairing, with Cloud bootstrap-only and runtime browser↔hosted wss↔kernel traffic.
2. Run the final kernel and real compiled TUI as a non-root user with external disposable CHARIOX_HOME and the normal linked accounts. Enable CHARIOX_KERNEL_BROWSER_DISPLAY=1, CHARIOX_KERNEL_BROWSER_MIRROR=1, CHARIOX_BROWSER_DISPLAY_SOFTWARE=1, CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER=libx264, CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS=1 and the selected CHARIOX_BROWSER_DISPLAY_GEOMETRY. Use public product runtime/bin on PATH, PYTHONPATH=<product-kit>/pytools, CHARIOX_BROWSER_DISPLAY_PYTHON=<product-kit>/runtime/bin/python3 and CHARIOX_BROWSER_DISPLAY_LIBYUV=<product-kit>/runtime/lib/libyuv.so.0. Run the bundled kernel with no arguments (loader/library path on older Linux). Do not disable Chromium sandboxing or substitute handcrafted identities.
3. In the real desktop web client at DPR 1/2, run Wikipedia article+portal, GitHub repo, Google results, major news, MDN and real video/canvas flows. Record per-site DOM-mirror coverage, screenshot success, click/type P95≤150 ms, streamed scroll≥30 fps and actual transport kinds. Full mirroring is expected absent an owner exception. Keep the unchanged masked gate; no PNG polling primary path.
4. Use the actual hosted wss relay, about 8 Mbps shaped client uplink and measuredRTT≥60 ms, real official linked provider agents where provider behavior is exercised, and one multi-hour session. Record exact app/kernel/TUI builds, machines, public non-secret account status, per-step screenshots/console captures and cleanup. Retain RED results with the first failing seam.

Security-critical observation/codec/immutable-file semantic review remains coordinator work; the absolute review inbox was absent after each local batch. MP-11 non-security exact-blob review is outside the narrowed scope. Performance remains RED independently of the missing real live environment; these commits must not be represented as finished acceptance.

MP-11 cleanup: all 23 receipt state/short-temp roots are absent; owned process groups/namespaces have empty inventories. Final focused Rust state 40 files/4,148,466 bytes and isolated baseline state are removed. Removed 2,627,315,351 bytes of own obsolete public source copies, the corrupt empty ELF, the superseded D3 kit and unnecessary debug data. The retained lane test ELF is the verified stripped kit ELF: SHA `f848904eac7be80172df16c6f2b646f38230b52591da35425ad3ff49c2719a24`. Original drill identity remains SHA `74f5907ac3f1068613895fc73abff145ef4770073349b2cfc457940c9fa6dafa`; it is not relabeled. Final kits/binaries and public build dependencies remain for coordinator use. All durable key stores/backups, credential profiles, shared reviewer state, other lanes, shared Cargo outputs and Docker resources were excluded. No provider credentials or durable owner private keys were read, copied or printed.

MP-11 resources: 1,491 samples; minimum MemAvailable 24.96 GiB and disk 76.02 GiB. These exceed the 12 GiB/10 GiB floors. Shared compilation uses the mandatory flock and four Cargo jobs. Exact commands/exits and resource samples remain in the external receipts. The final report commit changes documentation only; runtime artifacts and drill results remain bound to eeda.
