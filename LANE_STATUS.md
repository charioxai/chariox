# MD-DISPLAY-02 Phase 2 — native 4:4:4 ladder ACTIVE

2026-10-05 — Browser encoder rejects 4:4:4 but browser decoder accepts it. Native
PyAV/libx264rgb smoke at 2 Mbps: docs 51.0 dB / 106 ms median, media frozen
71.7 dB / 181 ms; not acceptance. Explicit GBR/full-range/sRGB metadata fixes
a retained pink-background RED frame. Full 0.5/1/2/4/8 Mbps ladder and public
scroll next. RGB PNG remains exact but dense public scrolling is slow.
MD-DISPLAY-03 untouched baseline settings/actual bytes recorded.
MD-DISPLAY-04 seam proposal and owner questions remain review work.

# MD-DISPLAY Phase 2 — codec ladder DONE; exact RGB validation ACTIVE

2026-10-04 — MD-DISPLAY-02/03: 105 cells at clean `0a92ef8d9` (2,100 fixture input trials), one Selkies 2 Mbps form cell RED at measurement-stage decoder backlog. Kept untouched baseline and every target/actual bitrate. All 75 portable-video cells converge to exact RGB with paced settled tiles, but unrefined text still trails Selkies. Capture-only PNG pairs are exact. Browser-native PNG diff prototype improves text median from 188 to 98 ms; dirty-source smoke/no-op correction is diagnostic, not acceptance. Final exact RGB, hybrid and corrected baseline runs next. MD-DISPLAY-04 API proposal pending owner decision; no production protocol changes.

# MD-DISPLAY Phase 2 — fidelity at a stated budget

2026-10-04 — MD-DISPLAY-02/03 measurement implementation ACTIVE from `8fdf12f67138bb234916356f147d2948c6ab2197`. Read frozen plans and owner discussion. Baseline inspection found installed Selkies defaults use quality-driven CRF plus paint-over; target bitrate is not actual throughput. Adding explicit rate-control ladder, settled PNG tiles, codec probes and selective hybrid switching. No production protocols changed or numbers allocated. Resource sample: 32 GiB available, 217 GiB disk free; floor 16/10 GiB.

## Owner questions

MD-DISPLAY-04: production scope, supported clients/OS, motion and settled fidelity/latency/network budgets remain owner decisions; research continues independently.

## Coordinator asks

MD-DISPLAY-04: no protocol requested for research. kbrowser protocol 417 attachment will be a proposal only.

---

# MD-DISPLAY lane status

2026-10-04 — MD-DISPLAY-01/02/03/04 research handoff READY; owner design decision pending.
Branch `agent/display`, frozen base `9334141d420f8a32393f206102c5b8b4a1b0b609`.
See [PUSH_READY.md](PUSH_READY.md) and
[display transport comparison](docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md).

- MD-DISPLAY-01: options, primary prior art, fidelity/colour/HiDPI, interaction,
  security/Vault and OS/cost comparison documented.
- MD-DISPLAY-02: sandboxed headed host Chromium DPR2 H.264 screencast, screenshot
  polling and isolated-world DOM+PNG patches: 15 × 20 inputs passed. VP9:
  5 × 20 passed. Eight execution module hashes match final source.
- MD-DISPLAY-03: installed immutable-image Selkies component baseline: 5 × 20
  passed, exit 0. Embedded source/relay labels differ from release tag; no signed
  F/G2 or Room acceptance claimed. Presentation ACK and sampler teardown fixes
  are harness changes. Earlier failing exits and first seams remain recorded.
- MD-DISPLAY-04: recommend portable video/neutral seam, native signed App views,
  then selective DOM with drift/masking gates. Keep existing desktop coverage
  until replacements pass. No production protocol changes or allocated versions.

Evidence: `/root/.codex/evidence/browser-resume-20260930/display/`;
selected campaigns `review-h264`, `review-vp9`, `review-selkies`;
`report/index.html`, `MANIFEST.json`, `cleanup-inventory.json`.
Exact measured clean commits: H.264 `e183160b6a9851117228cd4eda943fef7d75dcb3`;
VP9/Selkies `7f5cb0a1143ffd35134a720dd05ce3f854082308`.
All core campaigns exited 0; source identities remain unchanged in receipts.
Minimum sampled MemAvailable 41.4 GiB, disk free 219.0 GiB. Recorded owned
containers/processes/profiles gone; public tools/dependencies retained outside
Git for reproduction. No Rust, providers, credentials, shared-kernel changes,
GitHub mutations, deployments, cache pruning or external relay/Apps contact.

MD-DISPLAY-02 limitations: loopback, 20 samples/cell, fixture corpus plus one
actual public documentation page; DOM render-cycle and video canvas-pixel proxies
exclude physical scanout. Public DOM pair 33.01 dB / 1.67% changed pixels remains
MEASURED_UNACCEPTED; VP9 supplemental public navigation RED at ERR_NETWORK_CHANGED.
General sites, Vault safety, GPU, macOS/Windows, real IME/clipboard/drag/filechooser,
WAN, multi-user/slow-viewer/reconnect and production kernel/relay remain open.
No MD or managed acceptance item is closed. All authorized research is finished.

## Owner questions

MD-DISPLAY-04: choose user-domain-first scope; native App per-viewer transient
state; required client/codecs and capture permissions; geometry/masking gates,
colour/HiDPI and latency/network budgets; element and source-pixel action policy.
Detailed alternatives and recommendation are in the comparison document.

## Coordinator asks

MD-DISPLAY-04: review local commit series and decide the production lane order.
Assign protocol versions if production display capabilities/contracts change;
this research requires none. Nothing pushed; publication remains with coordinator.

## Earlier MD-DISPLAY work

Read frozen AGENTS, Browser/Computer plan, Path-1 inventory, M20 and owner
MULTIDOMAIN_PLAN. Lane-local labels above identify deliverables because the
owner plan has no numbered MD rows; they do not redefine acceptance items.
