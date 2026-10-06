# MP-08/MP-10/MP-11 — phase 20 FINAL, RED_PERFORMANCE

2026-10-06: required base `7cf42bc90`; runtime `eeda110c7` (commits `715a1c770`, `d3a66d79e`, `eeda110c7`), branch `md/display-perf`. Local 447/relay 90 unchanged. Report commit changes documentation only. No push, CI, PR, deployment or hosted access.

MP-08/MP-10: protected DPR 2 motion 31.11 fps meets the local 30 fps target (phase 19: 7.28). Click/type P95 56.9/901.5 ms remains RED. Final 1080p canvas: 58.35 fps, 1.97 pipeline cores, 55/52 ms. CPU/input/all-metric Selkies comparison remains RED. Exact settle passes in every final condition. Mask/hash/copy work reduced; opaque row fill ~99 ms→1.06 ms. Source re-attestation and exact repair/pacing remain latency seams. Earlier worker2 label was wrong; actual two-worker experiment does not justify changing the default of one.

MP-11: canonical 210 cycles, 7,146 protected presentations, zero violations. Extra DPR 2: ten cycles, 530 presentations, zero violations. Both codec guards and source retirement remain. 260 configured Node, 25 Python, 24 client and 65 focused Rust checks pass. Five host tests ignored outside explicit drill. One inherited obsolete relay73 assertion fails identically on 0aca/eeda and is excluded, with evidence. Default-stack test overflow resolves with 16 MiB RUST_MIN_STACK. Shared Cargo metadata mismatch resolves by rebuilding own App source under flock, without App code changes.

MP-08/MP-10/MP-11 evidence: `/root/.codex/evidence/browser-resume-20260930/display-perf/phase20/FINAL20.json`. Product/component final archives verified; real kernel/relay and public runtimes, compiled TUI source 0aca with unchanged CLI/client tree. Full report: `docs/MULTIDOMAIN_DISPLAY_PHASE20_RESULTS.md`. All 23 drill state roots and final Rust scratch removed; 2,627,315,351 own obsolete bytes removed. Minimum resources: 24.96 GiB MemAvailable/76.02 GiB disk. Shared, credential, key, reviewer and foreign paths excluded.

## MP-08/MP-10 Coordinator asks — real live BLOCKED here

Phase 20 delegates real live execution to coordinator and forbids lane hosted access. Run exact kits with real Cloud app/entry/flags, hosted wss ~8 Mbps and measured RTT>=60 ms, public-site list DPR 1/2, real linked providers where applicable, compiled TUI and multi-hour stability. Exact commands/required actions are in the report. Performance is RED independently of this blocker; do not stage as accepted. No protocol allocation requested.

## MP-11 Review inbox

Absolute lane inbox absent after every runtime/gate/kit milestone; recheck after final report commit. No security-anchor semantic approval inferred. MP-11 non-security exact-blob review is outside narrowed scope.
