# MP-08/MP-10/MP-11 — phase21 results

MP-08/MP-10 performance remains RED. MP-11's two P2 review findings are fixed with fail-first coverage; the canonical release masking gate passes 210 cycles / 7,046 protected presentations / zero violations. Real live acceptance is BLOCKED on coordinator execution, expressly delegated by the phase21 task and forbidden from this lane's hosted-access scope. These component runs do not close an MP item or authorize staging as accepted.

MP-08/MP-10/MP-11 source: required base fadfcd59bc629ed7aeb2cf46ce01ab3ee9b4b48d; branch md/display-perf; runtime e4440628e6b3d4c54086d8ab6af0cadf70ce3049, preceded by 18be64d0b and eca8b2492. Local447/relay90 unchanged; private helper commands and symbols add no serialized client/relay shapes. Local [skip ci] commits only; no push, PR, GitHub CI, deployment, Apps/hosted-relay contact or provider credential access.

## MP-11 — review fixes and privacy gate

P2 offscreen raster intersections: clamp both geometry endpoints to the raster extent and omit empty intersections before Python slicing. Fully offscreen boxes on all four sides remain empty; partially intersecting boxes remain enforced. Tests exercise actual stripe encoding/decoded-output guards and actual full-frame PortableEncoder input/output, including rejection of an unmasked intersecting pixel. Base RED bounds-red.log; final GREEN bounds-green.log and configured Python logs.

P2 protected crop merging: preserve complete native viewport mask metadata under a private symbol alongside crop-relative metadata, and restore it when DisplayCapture merges the crop into the full raster. Missing full metadata fails closed. The capture-to-DisplayStream-to-PortableEncoder regression uses a noisy crop to exceed the negotiated patch budget and includes another protected field outside the crop. Base RED crop-red.log; final GREEN privacy-green.log. This covers both encoder input binding and decoded-output binding; metadata does not cross the client protocol.

The unchanged canonical three70-cycle gate uses the exact committed release-test ELF and verified embedded assets, no script override. 870 mask checks, 7,046 protected presentations, zero violations. Extra protected DPR2: ten cycles, zero violations. Every final case also passes independent exact idle fidelity, navigation retirement and reference recovery. This is a supplementary local fixture gate, not the real-app/hosted-relay/security-anchor acceptance matrix.

## MP-08/MP-10 — input and exact refinement

An already attested native surface refreshes trusted region metadata after protection-relevant DOM changes without rerunning owned-window attestation. The source stops publishing while refresh is pending, discards readbacks older than the new metadata and drops readbacks retired during the awaited fence. A coalesced refresh requests one new readback even for a paint-free marker change. Navigation, page replacement and pre-attestation changes still fence the source. The actual native source is woken before and after input dispatch; the former wrapper call never reached it.

Exact native idle repair now takes an admitted, retained native raster rather than a CDP screenshot under the input/capture lock. The worker converts BGR to RGBA and prepares the exact full PNG outside the controller's input event loop. Quiet300ms, deadline, document/epoch/actor/policy/serial checks, immutable leases and both codec privacy guards remain. Native PNGs retain their document generation; the first preview exposed a missing generation and failed, then was fixed. Exact fidelity remains independently checked, not inferred from a timing result.

Fail-first refresh-red.log, input-red.log and refinement-red.log reproduce the old source fence, ineffective wake/native screenshot path and stale-readback/synchronous PNG seams. Green regressions cover navigation fencing, pending sample disposal, actual worker PNG/mask binding, and actual source wakes. These tests do not establish live acceptance or a50ms guarantee.

## MP-08/MP-10 — profiling and final performance

CPU stages were measured before changes from bound phase20 receipts. 1080p canvas: approximately1.04kernel / .37controller Node / .39codec / .14capture / .06Xvfb cores; protected DPR2: .44kernel / .94Node / .62codec / .22capture / .16Xvfb. An instruction-only perf profile of the old debug ELF shows substantial debug slice/core checks and JSON work. The final ELF uses cargo test --release: CPU changes across these receipts include build-profile differences and must not be described as a source-only speedup. Preview source overrides and rejected/resource-floor runs remain separately identified in SUMMARY21.json.

Final1080p canvas stages are about .50kernel / .32Node / .38codec / .14capture / .06Xvfb; protected DPR2 is .20kernel / .98Node / .69codec / .23capture / .14Xvfb. Linux100Hz process ticks estimate stage windows; nested timing spans are not additive. Remaining protected CPU is dominated by controller work and codec conversion/encode/decoded-output protection. Input timing retains capture wake/browser paint and rAF presentation latency; a canonical typing run still reaches119msP95. No security guard was bypassed to meet a performance number.

|MP-08/MP-10 condition|Motion fps|Pipeline cores|Click P95 ms|Type P95 ms|Exact settle ms|
|---|---:|---:|---:|---:|---:|
|protected-dpr2/canvas|42.96|2.24|55.60|71.60|1540.92|
|unprotected-1080p/canvas|59.60|1.40|55.20|54.90|1123.78|
|unprotected-1080p/docs|—|—|56.20|37.50|—|
|unprotected-1080p/scroll30|29.86|1.24|55.80|54.80|1766.96|
|unprotected-1080p/video|30.06|0.99|41.80|54.90|1393.37|
|unprotected-1080p/wheel30|21.96|1.04|40.40|55.50|1344.71|


Software x264/libyuv, one stripe worker,8Mbps negotiated budget, loopback kernel/relay component path. DPR2=2560x1600 raster;1080pDPR1=1920x1080. Performance motion spans10seconds, masking spans3seconds; shared-builder CPU compilation overlapped parts of Chariox testing and is disclosed. Exact settle means independent full visible-raster fidelity after motion, not merely delivery. Software rAF is a presentation proxy, not physical photon timing. The fixture's PASS_LOCAL_COMPONENT status means functional gates pass; it does not override the explicit RED performance targets. Wheel30 misses30fps; input<=50ms and pipeline<=1core are not met in all requested conditions. Hardware performance is unmeasured.

|MP-08/MP-10 fresh Selkies2 condition|Presentation fps|Content fps|Pipeline cores|Click P95 ms|Type P95 ms|
|---|---:|---:|---:|---:|---:|
|canvas|62.00|59.91|0.44|48.20|34.10|
|docs|—|—|—|31.60|30.30|
|scroll30|61.74|61.54|0.54|31.90|30.10|
|video|61.74|30.07|0.44|36.20|36.00|
|wheel30|61.38|49.08|0.61|36.50|34.40|


The fresh upstream Selkies2.0.0 software reference uses the same host, fixture workloads,1920x1080/DPR1,8Mbps and10second motion interval. Its direct local WebSocket path differs from Chariox's component kernel/relay path; it is not a hosted-relay comparison or product investment. All-metric Selkies parity remains RED. No Selkies/noVNC-specific product work was added.

## MP-08/MP-10/MP-11 — verification, artifacts and exact coordinator action

Configured controller checks:253 Node tests,252 pass/one SIMD adapter skip; the separate configured backend run covers that skip.25 client tests and26 configured Python tests pass. The broad Docker Node glob has one unrelated managed-provider isolation wrapper failure under root (a denied host path appears readable); no blanket suite pass is claimed. Unittest discovery matches no hyphenated modules; direct invocations of all four scripts pass. Startup's initial wrong kernel protocol flag fails as expected; corrected --print-local-daemon-protocol-version returns447. Eleven focused release Rust checks pass: protocol snapshots/hashes3, native packets2, owned signal groups2, display scheduling3 and transient byte isolation1; all run under the shared compile flock.

Exact product archive product-kit-final.tar.gz: SHA256 b0ffb1d382896caf460755e66c9a995b369a4c1a1524aaab5a0dc43b607bd9a3;962 manifest files verified. Real release kernel/relay/developer CLI plus public runtimes; compiled portable TUI source remains0acaf81dbcb543be8da90e17b5ea67737071d518 with unchanged apps/cli and packages/kernel-client trees. Use runtime/chariox for portable TUI; developer chariox-cli needs its build checkout. Help/protocol startup passes, not real-user acceptance. Product kernel SHA f48816cef1d51704122c31c7e6bac65984204ade0e4219f6d4e0b034f4143c0a.

Component archive component-kit-final/display-lan-kit.tar.gz: SHA25660d13c6c8e000c3b04d5ebf7bd589ec044ee2ce4388dd906a8c3518adc931293;1,626 files verified. Exact source/build e444; original release-test ELF780b1a7d4e41a8574b3def4248603c6f3b9e0de7aba45a245adb3265c87052c1, packaged strip --strip-debug ELF422e7cc1a2b582005bbbcc5b79ab91dc512d0da5f97e46600fa9402e9d82ebd5. No runtime identity/account/credential is bundled. Public Node22.20.0 is archive-pinned. All artifacts/evidence are outside Git under <lane evidence>/phase21; FINAL21.json binds commands, exits, hashes, screenshots, resource samples, failures and cleanup.

Builder2 reproduction: set MP21_BINARY=<phase21 lane>/bin/kernel-tests then run the external run-final-drills.sh, or MD_GEOMETRY=1920x1080 MD_DPR=1 MD_PROTECTED=1 MD_DYNAMIC_PROTECTED=1 MD_PROTECTION_REPETITIONS=70 <evidence>/run-case.sh <new-case>, three sequential cases. DPR2 uses1280x800/2. All settings and exits are retained. Portable supplementary fixture: use the component run-lan.sh and product runtime libyuv with MD_SOFTWARE=1 MD_ENCODER=libx264 MD_STRIPE_WORKERS=1 MD_MEMORY_FLOOR_GIB=12 MD_CREDIT_WINDOW=4, preserving geometry/DPR/protection variables through sudo as documented in phase20. This entry is not the real Cloud app.

Coordinator must execute COORDINATOR_LIVE.md with exact product artifacts and the real rebuilt Cloud app/entry/feature flags, normal product pairing and real hosted Caddywss. Run the real compiled TUI and kernel as a non-root desktop user using external disposable CHARIOX_HOME, normal linked official providers where relevant, and preserve Chromium sandboxing. Enable CHARIOX_KERNEL_BROWSER_DISPLAY=1, CHARIOX_KERNEL_BROWSER_MIRROR=1; public runtime/bin on PATH, PYTHONPATH=<product-kit>/pytools, CHARIOX_BROWSER_DISPLAY_PYTHON=<product-kit>/runtime/bin/python3 and CHARIOX_BROWSER_DISPLAY_LIBYUV=<product-kit>/runtime/lib/libyuv.so.0. Software settings are CHARIOX_BROWSER_DISPLAY_SOFTWARE=1, CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER=libx264, CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS=1 and selected CHARIOX_BROWSER_DISPLAY_GEOMETRY. Use the bundled loader/library path on older Linux.

Coordinator matrix: Wikipedia article+portal, GitHub repo, Google results, major news and MDN at DPR1/2, real user navigation/click/type/scroll/screenshot/reconnect, per-site DOM-mirror coverage and screenshots, shaped~8Mbps uplink and measuredRTT>=60ms, inputP95<=150ms and streamed scroll>=30fps, real official provider/TUI steps where relevant, and a multi-hour stability session. Full mirroring is expected absent an owner exception; no PNG polling primary path. Bind actual app/kernel/client identities and transport kinds. Local50ms/one-core optimization remains RED independently of this real-resource blocker. Do not stage as accepted.

MP-11 security-critical source/codec/observation/lease semantic review remains coordinator work. Absolute lane review inbox absent at each local milestone; absence is not reviewer approval. Non-security per-blob review is outside narrowed MP-11. MP-11: all38 recorded drill state/temporary roots are absent, all five owned baseline namespaces have empty inventories and are removed, and the final Rust scratch (0 files/0 bytes) is removed. Removed 52680 redundant debug bytes from the lane test ELF after testing; retained kit ELF SHA is recorded separately from the original drill hash. Public final kits/binaries remain for coordinator use. Shared Cargo, foreign paths/processes, keys/backups, credential profiles, reviewer state and Docker resources were excluded. MP-08/MP-10 resource samples:2,006 total; final/baseline1,637 samples, minimum32.82GiB MemAvailable/75.37GiB disk. A preview stopped at11.79GiB during shared load and cleaned its state; do not claim all runs stayed above the12GiB floor. Heavy Cargo uses flock/four jobs and own CPU affinity. No foreign process was signaled.

Final report commit is documentation only; runtime artifacts and receipts remain bound to e444, not relabeled to the later report head.
