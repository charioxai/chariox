# MD-DISPLAY-02/04 — Phase 3 handoff, local only

Branch `agent/display-impl`, latest execution source
`836e64630bc53d42e488dc97142416fdb0c92271`. Rebased onto kbrowser
`d6d03751ffea37198fb33530829f4cd76ae30fbf` as requested in REVIEW_INBOX 01:35 UTC.
The final handoff commit changes documentation only. Shared local protocol 419
is allocated and bumped in `5d6fb3c39`, with Rust/TypeScript guards and snapshots;
`0949b015d` updates that allocation's request hash for the shared actor adapter.
Relay wire shapes/version remain unchanged. Flag defaults off.

MD-DISPLAY-04: kernel-owned headed Chromium → protected DPR2 screenshot → VP9 →
exact PNG/dirty tiles → bounded existing local events and encrypted relay events.
The typed `KernelBrowserDisplayRequest` owns capture/input/takeover/release/actor
admission; the portable adapter adds negotiation/encoded-capture variants inside
that same barrier. Public input carries the displayed source document and uses
the shared actor ledger. No raw CDP source, Cloud media proxy, hosted service or
parallel browser authority is introduced. `apps/browser-display/presenter.mjs`
and harness are ready for coordinator Cloud wiring; integration and migration
note: `docs/MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md`.

MD-DISPLAY-02/04 final proof: real sandboxed headed source outside slices, focused
MCP dev-stub open, actual scoped-auth production local relay, standard browser
WebCrypto, exact settled/patch RGB pairs, 20/20 click acknowledgements, stale
input rejection, human takeover/MCP fencing/owner input/release and resumption.
Final p50/p95/p99 771.71/822.16/829.50 ms. Bootstrap VP9 34.04 dB. Click interval
received encrypted bytes/s 7,451; bootstrap/repair excluded from that rate, but
included in frame pacing at the 2 Mbps budget. Observed CPU 70.5% of one core,
including readback and excluding exited workers. Geometry differs from Phase 2.
**Latency remains RED; this is not acceptance or a proven general Selkies replacement.**

MD-DISPLAY-04 validation: 33 Node checks; 14 Rust request/event/conformance/actor/
takeover/origin checks; kernel-client TypeScript; rustfmt and local build pass.
Build has 40 warnings. Failure receipts, pixel pairs/diffs and raw distributions
are external. `phase3/provenance.json` binds final source, commands/exits, 118
source hashes, 28 exact embedded assets and the final binary SHA-256. Final
receipt: `/root/.codex/evidence/browser-resume-20260930/display/phase3/final-typed-relay-2mbps/results.json`.
Preserved manifests: `pre-rebase-provenance.json`,
`typed-api-before-retry-provenance.json`. Neither old results nor Phase-2 research
are relabelled as final-file coverage. `handoff.json` verifies unchanged execution
files across this final documentation commit.

MD-DISPLAY-02 reviewer/coordinator mapping:

- Phase-2 PR #844 findings 1–4 remain attributed to their fixes/receipts on
  `agent/display`; no claim that those executions cover this implementation.
- New worker errors reach awaited cleanup; unsafe PIDs are guarded. Fail-first
  `encoder-retry-fail-first.log` exposes/fixes retry spawning after worker failure
  in `836e64630`; no unawaited replacement is launched.
- Fail-first `presenter-credit-fail-first.log` exposes/fixes an event-before-receipt
  credit race in `b42b33787`; one credit stays held through presentation.
- REVIEW_INBOX 01:35: rebase onto d6d03751f; `0949b015d` attaches through the typed
  API and shared actors. Final live relay receipt proves takeover/release behavior.
- Direct browser/native socket attempt remains a RED historical receipt. The
  existing Origin refusal is preserved and passes its focused test; the harness
  uses the admitted relay path. No guard weakened to make the drill pass.

MD-DISPLAY-02 cleanup: exact owned process inventory empty and disposable runtime
state removed after every final run; no containers created. Removed 6.16 GiB own
incremental output and own temporary links/helper; binary/dependency objects
retained only for coordinator replay. `phase3/cleanup.json` records inventory and
protected-path exclusions. No shared caches, services, keys/accounts or other
lane resources changed. No push, PR, CI, merge, Cloud staging or deployment.

MD-DISPLAY-04 next: independent review, Cloud flag wiring, profile the kernel
capture/PNG/serialization/actor cost, then matched-geometry bitrate/media/WAN,
slow-viewer/reconnect, live Vault and native OS gates. Owner budget/design and
Room migration decisions remain open; no MP or MD acceptance item closes here.
