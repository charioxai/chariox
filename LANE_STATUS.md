# MP-08 / MP-10 / MP-11 — culinux

## 2026-10-06 — FINAL Phase A; stopped for coordinator review

- Branch: `computer/unicode-input-ocr-main`, base `e325afa580` (local435 / relay73).
  Reconciliation commit `eed1860b0`; final report commit follows on this branch.
  No push, PR, deployment, Cloud staging change or protocol allocation.
- #846 `0c4e848e`: no remaining runtime/test patch on this main. Preserve main's
  later authorization/keyboard changes; comparison receipts retained externally.
- `docs/COMPUTER_USE_LINUX_PLAN.md` records current paths, open cells and Linux
  virtual-desktop design. Binding inbox requirements addressed: AT-SPI→OCR→exact
  screenshots; shared XShm/XDamage display with cursor/copy-rect/region policy;
  human-only streaming; 24 provider × slice/host × client proof cells before
  OSWorld rental. Targets ≤1 core/1080p software, <50ms and exact settle are
  planned, not measured. Assume virtual host display before real login session.
- PASS: Node131 / native42 / signal guards5; zero skips. Physical Xvfb18/18 and
  Xorg18/18 at clean `e325afa580`, production helpers mounted into a dependency
  fixture image. Neither run executes the optional physical kernel child-death
  seam. No Rust build/test or live client/provider acceptance claimed.
- NOT_RUN: all `PROVIDER-*-COMPUTER-FALLBACK` paid turns. RED shared preflight:
  available G2 image has a different runtime-source revision and relay70 versus
  source73; no exact-source image found among six runtime-labelled images.
  Failure precedes account import/provider launch. No credential reads/copies,
  provider auth attempt or 401 observed. Mode switching/live traces remain
  unestablished. OSWorld lacks KVM here and also requires the new proof gate.
- Cleanup PASS: exact fixture containers absent, lane-created image removed,
  only lane-owned Dockerfile scratch removed; no volumes/ports/runtime identities
  created. Shared build caches, provider stores, keys and reviewer state untouched.
- Evidence: `/root/.codex/evidence/browser-resume-20260930/culinux/MANIFEST.json`
  indexes exact sources, commands/exits, source tests, physical reports/screenshots,
  resource samples, RED preflight and cleanup. Physical minimum MemAvailable
  23.70GiB / free disk134.49GiB; final17.21GiB /135.12GiB. No MP cell closed.
- Review mapping: `PUSH_READY.md`. Phase A finished; Phase B not started.

## 2026-10-06 — Phase A reconciliation milestone

- Base: `e325afa580d81954e2c179757fc53fa02ed2a2b3`, local435 / relay73.
  Branch: `computer/unicode-input-ocr-main`; no push or deployment.
- #846 `0c4e848e` has no remaining runtime/test delta on this base. Preserve
  main's later fixes; reconciliation recorded in `docs/COMPUTER_USE_LINUX_PLAN.md`.
- Focused checks PASS: Node131 / native42, no skips. Physical Xvfb PASS:
  18/18 helper rows; Xorg run in progress. No MP acceptance closure claimed.
- Evidence: `/root/.codex/evidence/browser-resume-20260930/culinux/`.
- Remaining Phase A: audit/design, local provider prerequisites/results,
  resource/cleanup receipt and final coordinator handoff. Stop before Phase B.

## Coordinator asks — MP-08 / MP-10 / MP-11

- Reserve a future protocol allocation for host Computer surface identity,
  desktop generations/geometry, shared mode/display and access projections
  plus bounded AT-SPI handles/actions and cursor/copy-rect extensions, only after
  reviewing the Phase A design against #900/#893. No number claimed.
- Supply/build an exact-source immutable slice runtime and verified host binaries
  with paired clients for paid fallback replay; do not reuse the G2 image as
  `e325afa580` proof. Keep linked profiles materialized via product commands.
- Coordinate full-desktop capture/protection and measured latency/CPU contracts
  with the display lane; the inspected #900 reference is `6dde21a8c`, not an
  execution claim for #893's optimized source.

## Owner questions — MP-08 / MP-10 / MP-11

- Confirm the <50ms target's percentile and measurement endpoint for acceptance;
  propose p95 source input to verified human presentation, with all percentiles
  retained. This does not block Phase A or authorize a performance claim.
- No owner decision blocks Phase A. Virtual display first is the stated
  assumption; real X11/Wayland login-session control remains a later phase.
