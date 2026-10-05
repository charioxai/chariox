# MD-DISPLAY-02/04 — Phase 3 milestone, local only

Implementation reviewable; final clean-source measurement is pending. Branch
`agent/display-impl`, base `84056953d1b478a581d1a557ed096829e51e12c2` from
`md/kernel-browser`. Protocol allocation 419, bumped with shared guards and
request/event hashes in this implementation commit. Flag defaults off.

Protected DPR2 capture → portable VP9 → exact PNG/dirty tiles → existing kernel
terminal event queues and encrypted relay route; displayed-document input uses
the existing physical input/cancellation seam. Self-contained presenter and
production local relay drill under `apps/browser-display/`; integration note:
`docs/MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md`.

MD-DISPLAY-02/04 intermediate proof: 29 Node tests pass; Rust request/event
snapshots 3/3, transient event test 1/1, client conformance 4/4. Kernel-client
TypeScript check passes with read-only public dependency links. Real headed
Chromium, focused dev-stub MCP open, production local relay/scoped admission,
browser request/event encryption/decryption and 20 visual click acknowledgements
pass. Settled and small-change RGB pixels are exact. Intermediate latency is
RED versus the research target: p95 708–761 ms. The latest PNG decode optimization
is not yet covered by a clean live receipt at this milestone.

All intermediate receipts preserve dirty source and binary identities. Research
Phase-2 numbers remain historical; they are not relabelled as this source. No
MP, MD acceptance, native Mac/Windows, Cloud deployment, hosted WAN or Room
replacement gate is closed. No push/PR/deploy/external service mutations.

MD-DISPLAY-02 reviewer mapping: Phase-2 findings 1–4 remain fixed/proven on
`agent/display` as documented there; this branch adds awaited encoder errors,
PID guards, a failure receipt and owned cleanup. Initial port-zero and probe
geometry failures remain RED receipts in external evidence, with corrected runs.
The review inbox was absent before this milestone; check it after the commit.
