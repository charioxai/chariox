# MP-08 / MP-10 / MP-11 — culinux Phase A handoff

Local branch `computer/unicode-input-ocr-main`, base
`e325afa580d81954e2c179757fc53fa02ed2a2b3`. Coordinator publishes; no push by lane.
`eed1860b0` records #846's empty remainder; the final documentation batch contains
the complete audit/design and this review map. There are no runtime changes.
No shared local/relay protocol number was allocated or changed.

## MP-08 / MP-10 / MP-11 review inbox mapping

Read absolute inbox
`/root/.chariox/dev/browser-resume-20260930/agents/culinux/REVIEW_INBOX.md`
after the reconciliation milestone and before final handoff. The coordinator's
binding design requirements are addressed in `docs/COMPUTER_USE_LINUX_PLAN.md`:

| Inbox requirement | Response / remaining limit |
| --- | --- |
| All providers × slice/host × Web/TUI before OSWorld rental | Exact 24-cell matrix and six-task oracle; explicit rental gate. All paid cells remain NOT_RUN; image-admission RED retained, no successful model execution inferred. |
| AT-SPI tree, OCR fallback, exact screenshots; agent independent of video | Proposed scoped session bus, protected bounded handles/tree revisions and actions, OCR/pixel fallback and viewer-independent observations. No AT-SPI implementation/test claimed in Phase A. |
| Reuse optimized display; no separate streamer | Whole-desktop XShm/XDamage source in #893's shared pipeline; explicit distinction from historical inspected #900 display receipt. No streamer code added. |
| Copy-rect, separate cursor, region classification, human-only stream | Acknowledged-base/protection fencing, XFixes channel, exact UI/H.264 motion regions and last-viewer shutdown described. No performance proof claimed. |
| ≤1 core/1080p software, <50ms, exact settle, beat CPU/GPU baseline | Targets and matched-comparison requirements retained; percentile/endpoint clarification in LANE_STATUS. Existing baseline evidence reused; no new Selkies-specific work. |
| Virtual display first; real X11/Wayland later, Hyprland limits | Owned Xvfb/WM first; XShm/XTEST versus portal/PipeWire/libei later. Official Hyprland portal declaration lacks RemoteDesktop; backend probing remains required. |

These are report/design responses to the inbox, not implementation fixes or
acceptance disputes. MP-11 scope follows the owner's narrowed behavioral parity
and security-anchor review rule; non-security source does not require a new
per-anchor exact-blob audit. Host signal safety explicitly excludes the old
slice lifecycle's name-based pkill implementation.

## MP-08 / MP-10 / MP-11 evidence and publication limits

External evidence root:
`/root/.codex/evidence/browser-resume-20260930/culinux/`.
`MANIFEST.json`, `tests.json`, two physical `report.json` files,
`provider-preflight.json`, `reconcile.json`/diff and `cleanup.json` bind results.
PASS Node131/native42/guards5; PASS_HELPER_ONLY Xvfb18/Xorg18.
Paid provider/client/optimized-display/AT-SPI/fresh-Path-1 gates are unestablished.
All exact owned fixture resources were removed; borrowed images/caches and
protected state were preserved. Phase B requires coordinator review; lane stops.
