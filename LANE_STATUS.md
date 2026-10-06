# MP-11 phase23 supervisor cleanup follow-up

2026-10-06: coordinator 22:45 P2 mapped to kernel-owned native capture pool and Xauthority reclamation. Native display uses the existing private kernel packet directory; durable Chromium profile remains separate. Fixed-slot cleanup validates owner/private mode/type/link count/size through retained directory FDs, never recurses or reads authority contents. Fail-first Rust check reproduced pool/authority leak; extended real-binary component crash drill inspects persistent and transient paths before harness removal. Not real live acceptance.

MP-11 allocation audit: SysV capture uses IPC_RMID after attach; X resources die with owned X server; COW encoder mappings close on every request; encoder handoffs and stripe packets already kernel-owned; native pool + display.xauth now kernel-owned too. Durable profile/tabs remain intentional persistence. Current security-anchor review is still coordinator-owned.

# MP-08/MP-10/MP-11 — phase23 IN PROGRESS

2026-10-06: required base7d09b7ae761f4cfe0912ca61eb957be35fb59db0; md/display-perf. Native protected motion now uses a leased private COW transform in Python; no Node full raster read/write per frame. PNG CRC stays checked with native zlib. Native exact quiet30ms; exact contiguous patches avoid full re-verification; small PNG avoids redundant tiles. Supplemental60Hz wheel instrument and opt-in existing whole-video capability for owner VAAPI comparison. Serialized client/protocol shapes unchanged447/90.

MP-11 fail-first masked-handoff/deadline/exact-reuse checks; immutable source/native decoded-mask tests; owned fallback lease retained through materialization. Component preview failures exposed wrapper ownership and capability spread, fixed before commit. Previews are not final ELF or acceptance. Resource monitor external, floor12GiB memory/10GiB disk.

## MP-08/MP-10 Coordinator asks — real live BLOCKED

Run/supply actual authorized Cloud app entry/flags + hostedwss at8Mbps and RTT>=60ms + real desktopDPR1/2, public sites/services/official provider accounts/TUI and multi-hour stability. This lane is prohibited from Cloud/hosted relay/Apps contact. Owner Intel UHD620/iHD GPU comparison stays coordinator-run. Component fixtures never close acceptance. No protocol number requested.

## MP-11 Review

Actual inbox /root/.chariox/dev/browser-resume-20260930/agents/display/REVIEW_INBOX.md; historical #893 masking/reclaim findings fixed on base phase22. Check after each batch and map any new entries. Protection/COW/lease changes require current security-anchor semantic review. Non-security exact-blob audit is outside narrowed MP-11.
