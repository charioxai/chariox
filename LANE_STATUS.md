# MP-08/MP-10/MP-11 display phase25 — FINAL RED / live BLOCKED

2026-10-07 09:13 UTC. MP-08/MP-10 base `b340ae0f7`, branch `md/display-perf`. Runtime/build/kit source `67e5231fb`; final handoff commit is documentation only. Local [skip ci] commits; no publishing, CI, deployment or hosted contact. Public contracts remain local 447 / relay 90.

MP-08/MP-10 targets remain RED. Scroll60: 1080p 1.055 cores / 58.94 fps / exact 795 ms; DPR 2 1.420 cores / 38.00 fps / exact 1,481 ms. Wheel60: 1080p 1.127 cores / 48.67 fps / exact 748 ms; DPR 2 1.634 cores / 36.98 fps / exact 1,524 ms. Retina docs type P95 is 36.4 ms, but click 54.8 ms fails. Combined input gates pass 4/7 at DPR 1 and 0/7 at DPR 2. Fresh seven-case Selkies2 comparison retains its actual `89e4bf3ed` source (scroll60 0.547 cores / 61.94 fps). No acceptance or staging readiness.

MP-11: Rust 45 at `67e5231fb`, Node 256 / report 2 / runtime-pin 1 at `9d5f730bc` pass. Unchanged 210 cycles / 10,548 protected presentations / zero violations. Abrupt crash reclaims 49,152,000 raster bytes and packet/Xauthority roots before parent cleanup, preserving browser profile. Additional protected DPR 2 scroll: five cycles / zero violations. Expired 300-second fixture-grant failures remain RED; bounded 900-second test grant restores complete navigation/crash flows without changing production admission. All 177 recorded roots are absent; public-artifact cleanup and resource floors pass; owned monitors stopped. Changed security anchors need coordinator semantic review.

MP-08/MP-10 kit: `/root/.codex/evidence/browser-resume-20260930/display/phase25/owner-kit-phase25-final/display-lan-kit.tar.gz`; SHA256 `cedd4737257d939f053eccae71325e32656a0ed214dd431560ee1692255f5428`, 359,850,915 bytes. All 1,720 manifest hashes, isolated imports and actual kit component pass. Actual virtio VAAPI error recorded with x264 fallback; no Intel success claimed. Details: [phase25 report](docs/MP_DISPLAY_PHASE25_RESULTS.md).

## MP-08/MP-10 Coordinator asks

Real live acceptance BLOCKED: run authorized actual Cloud entry/flags, real CLI/TUI, exact kernel, official product-linked providers where relevant, hosted wss with ~8 Mbit/s client uplink / measured RTT ≥60 ms, fixed public sites at DPR 1/2 and a multi-hour real desktop session. Lane no-contact rule prevents this run. Stock Arch/Omarchy iHD success needs updated-kit laptop rerun and `motion_backend_vaapi` receipts. Software targets remain RED independently. No protocol allocation requested.

## MP-08/MP-10 Owner questions

No new product decision requested. Final stop follows the explicit phase25 instruction; this is an incomplete feature handoff with concrete resource/authorization blockers and RED measurements.

## MP-11 Review mapping

00:55 xxhash and 01:00 hardware/floor/pool → phase24 commits in the report. 04:55 hardware diagnostics/host driver stack → `75dc87332` / `25f61a71d`: actual error/fallback proved; Intel success still BLOCKED. Latest inbox 04:55 unchanged after milestone checks. Current security anchors need coordinator review; no unrelated exact-blob audit.
