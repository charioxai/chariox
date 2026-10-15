# MP-08 / MP-10 / MP-11 — hosted per-hop kit

Run `node apps/browser-display/hosted-hops.mjs MANIFEST.json EVIDENCE_DIRECTORY`.
It serially runs the supplied V/S commands, or replays existing recordings when
`command` is omitted, and writes `S-hops.json`, `V-hops.json`, endpoint nearest-rank
P50/P95, presented fps, rejected native work per scroll workload, and
`quiet-resource-windows.json`. Replay is analysis, never fresh acceptance.

Each `runs` entry contains `path` (`V` or `S`), `repeat`, `site`, `identity`
(`oss`, `cloud`, `binary_sha256`, plus the actual Selkies image/source identity),
`report` (probe results.json), `resources` (one-second /proc/stat samples), and
`source`/`relay` for V or `native`/`server` for S (numeric JSONL traces).
Optional `command` is an argv array, with `env` for the probe; no shell expansion.
All paths are absolute. Record three back-to-back repeats of each path per cell.

The r5 user-driven probes live in `hosted-hops/`: `probe-visible.mjs` and
`probe-selkies.mjs` take an owned runtime ROOT and evidence directory. Their
supported roots are `/root/.chariox/dev/vidlat/r<N>/drills/<name>` or
`/root/.chariox/dev/vidlat/owner-ab-r<N>-<name>`, with evidence under
`/root/.codex/evidence/browser-resume-20260930/vidlat/r<N>/`. Validation
precedes the state read. Owner A/B kernels use `V/runtime`; CPU accounting
separates V and S sources and tracks their actual kernel/Selkies PID. The
owner kit state supplies `sessionId`, `kernelPid`, `selkiesPid`,
`selkiesOrigin` and source CDP port. Source CDP is read-only; Selkies
navigation uses its native omnibox through real keys. The read-only source
oracle selects the unique visible page; CDP list order can name a background
tab after normal new-tab actions and must never determine input coordinates. Supply `VIDHOPS_IPC`
(the exact SDK dist/ipc.js), `VIDHOPS_WS` (ws/index.js), `VIDHOPS_PLAYWRIGHT`
(default /w/valenv/tools/node_modules/playwright-core/index.mjs), `OSS_SOURCE`,
`CLOUD_SOURCE`, `DPRS=1` or `2`, `TRIALS=20`, `TYPING_TRIALS=20`,
disk TMPDIR and capped NODE_OPTIONS. Presenter instrumentation is off by
default; `VIDHOPS_INSTRUMENT=1` explicitly enables a diagnostic source override.
`VIDHOPS_RECORD=1` records a separate usability run. Screen recording throttled
the DPR2 input cadence in the quiet-host drill: do not use recorded runs as
throughput acceptance. Retain actual wheel input counts and every physical
endpoint refusal. The typing probe uses the visible public glyph `m`, records
its source echo separately, and excludes missing physical endpoints from
percentiles while retaining attempt counts. Its small-region detector uses
three changed columns to exclude the caret. Fewer than20 detected samples
cannot satisfy the campaign gate. Use Node22,
headed Google Chrome, Xvfb, Python numpy/Pillow/Xlib. Source and viewer geometry,
GPU flags, browser cache and shaper topology must match between paths. State is
created by normal product enrollment, never hand-made identities. Protect runtime
state; only numeric traces and public screenshots enter evidence.

Use the real built Cloud entry/flags, source kernel on the dedicated quiet host,
and the hosted TLS/WSS route shaped to 8 Mbit/s and >=60 ms RTT. No builds on that
host. Baseline instrumentation: `pixelflux-timing.py` patches an OWN pinned X11
source into a separate output (MPL upstream source stays MPL); `selkies-timing.py`
and `sitecustomize.py` observe callback/input/socket/credit seams. The r5
`relay-trace.patch` uses a bounded asynchronous numeric-only observer; never use
synchronous relay logging. Record source hashes before and after instrumentation
and run the uninstrumented baseline to bound observer overhead. A rebuilt
pixelflux against system FFmpeg/x264 is not the pinned campaign image.

The production native worker counts upstream rejected work by fixed reason:
stale document, epoch, region revision, serial skipped, shift hold. Opt-in
`CHARIOX_BROWSER_DISPLAY_TIMING=1` records numeric `native_rejected` snapshots;
the runner differences each worker/reason counter within each workload. Region
retirement may invalidate multiple stages; counts are reasons at seams, not a
partition of all captured frames. No new client/relay protocol fields are added.

Owner amendment (2026-10-11 05:42 UTC, MP-08/MP-10/MP-11): strict Selkies parity
replaces the audit tolerances. There is no producer substitution fallback.
Per-hop gates are stepping stones only.

Decision G: >=20 click/type samples x3 repeats, DPR1/2, five public sites;
click/type P95 <= S, presented fps >= S for 120px@20/60Hz and16px@60Hz,
pipeline cores <=1 and <=S, no PNG fallback, MP-11 masking zero violations.
A run above40% host CPU P95 is VOID. No CPU samples is also VOID. Keep each stage's
percentiles separate and retain ambiguous join counts. The r5 probes cover
Wikipedia controls; broad site drivers and masking acceptance
must accompany them and cannot be inferred from this kit's endpoint report.

Stop every owned browser/Xvfb/probe and source runtime, remove runtime-generated
identities/scratch and your KEEP/peer allowance, then stop your own PR stack.
Never signal PID0/1/-1 or shared processes. The probes guard exact child PIDs.
