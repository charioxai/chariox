# MD-DISPLAY-01/02/03/04 — local research handoff

READY FOR REVIEW on `agent/display`, based on G2
`9334141d420f8a32393f206102c5b8b4a1b0b609`. Local commits only, all `[skip ci]`
and required Codex coauthor. Coordinator owns publication; owner owns the
undecided transport design. This handoff does not authorize production merge.

MD-DISPLAY-01/04: [comparison and recommendation](docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md)
covers native Apps, DOM/co-browsing, draw commands, portable video and current
baseline, primary sources, secret boundaries, interaction gaps, a transport-neutral
kbrowser seam, staged gates/rollback and owner questions. Recommendation:
portable video first, native signed App views next, selective DOM pilot after
masking/drift gates. PNG patches are too costly for a default general-page path.

MD-DISPLAY-02/03: [reproducible harness](experiments/multidomain-display/README.md)
ships live docs/SPA/form/canvas+video/cross-origin fixtures, source input replay,
WebCodecs H.264/VP9, DOM isolated world/scriptless viewer, installed Selkies adapter,
paired PNGs/diffs/RGB PSNR, latency histograms, bytes/s, CPU/RSS/resource guards,
exact source/hash receipts and ownership-checked cleanup. The standalone report
adds source/viewer opacity overlays and binds rows to receipt SHA-256.

Selected evidence root: `/root/.codex/evidence/browser-resume-20260930/display/`.
`report/index.html` and `report/summary.json`; `MANIFEST.json` lists exact commands,
exits, source identities, resources and cleanup. Results:

- MD-DISPLAY-02 `review-h264`: 300 input trials, 15 core cases, exit 0;
  clean `e183160b6a9851117228cd4eda943fef7d75dcb3`.
- MD-DISPLAY-02 `review-vp9`: 100 input trials, 5 core cases, exit 0;
  clean `7f5cb0a1143ffd35134a720dd05ce3f854082308`.
- MD-DISPLAY-03 `review-selkies`: 100 input trials, 5 core cases, exit 0;
  clean `7f5cb0a1143ffd35134a720dd05ce3f854082308`.
- MD-DISPLAY-02/03 all eight executed module hashes match final files;
  3 focused metric tests pass; module syntax checks pass; headed-browser report
  check passes (25 cases, images and opacity control, no page errors).
- MD-DISPLAY-02/03 minimum available memory 41.4 GiB, free disk 219.0 GiB;
  selected cleanup receipts and subsequent inventory find no live recorded
  owned processes, disposable profiles or baseline containers.

MD-DISPLAY-02 supplemental actual public docs DOM pair is MEASURED_UNACCEPTED
(33.01 dB, 1.67% changed pixels). VP9 public navigation is RED at
ERR_NETWORK_CHANGED. Early harness RED runs remain retained with first failing
seams. Installed baseline image/browser/source labels are attributed separately;
its tag is not proof of release provenance. No noVNC, Room, signed release,
production encrypted relay, Vault, OS parity or full Browser/Computer acceptance
is claimed. No MD acceptance item is closed.

MD-DISPLAY-04 owner decisions: pilot scope, App transient viewer state,
geometry/masking gates, latency/network/colour budgets, action policy and
supported codecs/capture permissions. Coordinator assigns versions before any
production serialized change. No production files/contracts changed, no Rust
build, provider/credential access, deployment, GitHub write or shared-state cleanup.
