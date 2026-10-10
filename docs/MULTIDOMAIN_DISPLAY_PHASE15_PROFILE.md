# MP-08/MP-10/MP-11: phase 15 pipeline profile and stripe contract

**STOP_FOR_PROTOCOL_ALLOCATION; performance remains RED.** Step A ran on the
assigned clean source `1b4d98783b0007b78db3bd5f29d3cd33bf10e79b`, starting
2026-10-06 around 09:28 UTC. Independently encoded H.264 stripes need a new
negotiated frame contract. The owner explicitly requires stopping for a number
when transport shapes or semantics change. No runtime changes, version changes,
stripe prototype, or post-change acceptance are included in this handoff.

## MP-08/MP-10 measured scope and identities

Evidence: `<lane evidence>/phase15/`.
`profile-summary.json` and `summarize.py` retain the stage aggregation; original
receipts, screenshots, console/kernel logs, Python cProfile, V8 CPU profiles,
perf records and pidstat captures remain beside it. All samples use software,
1920×1080 DPR1, 8 Mbps ceiling, the existing docs and scroll30 fixtures,
20 click and 20 typing acknowledgements, and 10-second motion windows.

The kernel test ELF was rebuilt from the clean assigned base under the compile
slot, four jobs, `CARGO_PROFILE_TEST_DEBUG=0`. SHA256:
`9b4a77d97f59bed7efa561626b3d275afd967e3f744c032ed733723b0ba57d55`.
It is an **unoptimized test executable** running the real kernel/relay services
inside the existing ignored display drill. The presenter is the real module
inside a fixture entry. This is diagnostic evidence, not the owner's required
real web-app/CLI end-to-end acceptance. In particular, Rust CPU can be biased by
the test build. No provider, hosted service, physical display, or GPU is tested.

Selkies is 2.0.0, installed pixelflux 2.1.0. The installed extension exactly
matches the release wheel for source
`1ebcb0a10a96caeba20e1ad09be47cc018a55238`: extension SHA256
`7731c620a0ff53432bf366423924643def52fde70375e41e92f2c370e909d52e`;
wheel SHA256 `a4f3fb52f5e2dec03c9b1539ac750c19d43a04d00b4de744054260292d1da0f2`.
The Selkies source dependency lock also references pixelflux
`6974a8c16a14dc0040d5ed937d633a6ceb4c926d`; both revisions were inspected,
but measurements belong to the installed release, not the lock revision.

The existing Selkies comparison uses its **whole-frame h264enc mode**, confirmed
by every profiled packet having y=0, height=1080. Its measured advantage therefore
does not establish that striping alone fixes our gap. Selkies uses direct local
WebSocket/XTEST; ours uses encrypted relay traffic and kernel/CDP input. Native
owned-window capture was admitted in both Chariox runs; compressed CDP motion
fallback was not used. Initial Selkies runs overlapped another lane's compile;
later sequential perf repetitions reproduce the CPU separation. The builder is
shared, not an isolated timing laboratory.

| MP-10 fresh base diagnostic | Click P50/P95 ms | Type P50/P95 ms | Motion FPS | Active pipeline cores | Source/viewer cores | Exact settle |
|---|---|---|---|---|---|---|
| docs, Selkies 2 | 30.0/31.2 | 27.6/29.9 | — | 0.489 | 0.112/1.049 | No |
| docs, Chariox | 52.8/54.8 | 93.9/97.1 | — | 1.272 | 0.447/0.355 | Yes |
| scroll30, Selkies 2 | 30.9/47.6 | 29.2/46.3 | 61.79 | 0.560 | 0.182/1.020 | No |
| scroll30, Chariox | 51.5/56.1 | 113.3/135.7 | 29.71 | 1.754 | 0.338/0.590 | Yes |

Active docs means the click window. Pipeline includes server, capture, encode,
Xvfb and, for Chariox, kernel/relay. Source and viewer Chromium are separate.
Scroll repetition under perf gave Chariox 1.771 pipeline cores, click P95 56.1 ms.
The unchanged phase-14 [15-row software table and GPU table](MULTIDOMAIN_DISPLAY_PERFORMANCE.md)
retain their original source/build identities. They are not relabeled as phase-15
results. There is no new 15-row after comparison because Step B is blocked.
GPU remains unmeasured for every fixture/backend; actual owner-laptop hardware
access and coordinator execution are required for that leg.

## MP-08/MP-10 stage attribution

Below are all-run scroll30 samples, including initialization, input and settle.
Nested spans overlap and **must not be added**. Stage P95 uses nearest rank in
`summarize.py`; the existing end-to-end receipt uses its original percentile
method. The actual input-to-rAF values above come from those receipts. rAF is a
software presentation proxy, not photon timing.

| MP-10 stage | Chariox n; P50/P95 ms | Selkies n; P50/P95 ms |
|---|---|---|
| Owned XShm readback, including copy | 366; 1.398/1.775 | Not separately timestamped |
| Full-raster SHA-256 fingerprint | 366; 4.928/6.951 | Native combined span below |
| Damage scan after fingerprint | 366; 0.058/0.266 | Native combined span below |
| Raw raster pipe transfer | 366; 6.210/11.567 | Pooled native surfaces; no raw pipe |
| Native capture plus policy fences | 366; 15.293/30.660 | Not separately timestamped |
| Encode request/response, with conversion | 308; 6.238/11.914 | Native combined change/color/encode: 1178; 2.691/3.662 |
| Exact CDP capture | 37; 108.940/118.934 | No exact lossless settle |
| Exact PNG decode worker round trip | 37; 33.709/143.718 | No corresponding stage |
| JSON packet serialization | 375; 0.041/0.076 | Native packet header included in encode |
| Native callback/packet fanout | Included in IPC/serialization | 1178; 0.050/0.067 |
| Event serialization/encryption | 375; 1.695/2.793 | Plain local WebSocket baseline |
| Event queue/credit | 375; 0.043/0.073 | Callback end to send: 1150; 0.170/0.259 |
| Event socket write | 375; 0.098/0.174 | WebSocket send: 1150; 0.087/0.127 |
| Capture gate waiting | 2093; 0.588/22.614 | Capture/encode threads decoupled |
| Frame pacing wait | 375; 1.139/11.935 | Continuous 60 Hz stream |
| Client decode | 369; 1.100/3.500 | 1136; 1.000/1.500 |
| Canvas draw | 369; 8.200/10.400 | 1136; 6.600/7.900 |

Pixelflux's `capture_ns` is stamped **after** the XShm read and overlays, at
publication. Publication-to-encode-start is 0.021/0.037 ms, not the capture cost.
Its exposed encode timestamps combine change detection, conversion, encoding
and header construction. Stripped symbols do not support an honest individual
CPU split for those native sub-stages; that limitation remains in Step A.

Python instrumentation isolates the Chariox scroll helpers: 423 raw copies cost
214 ms process CPU, full fingerprints 1744 ms, damage scans 28 ms. For 308 encoder
requests, raw reads cost 434 ms CPU, AV plane copies 364 ms, conversion/downscale
550 ms, codec encode 1233 ms, and redundant same-format conversion 7 ms.
Codec process CPU includes codec threads. Measurements include startup and
profiling overhead and do not sum to whole-motion pipeline CPU.

At 199 Hz helper perf sampling, Chariox's repeated sample distribution was
40.90% libcrypto (fingerprint), 22.69% x264, 13.43% swscale, 12.96% libc and
6.79% Python. Selkies was 81.92% x264, 9.04% pixelflux native module, 6.78%
Python and 1.18% libc. These are helper CPU distributions, not whole-pipeline
percentages. DSO attribution is reliable enough for this comparison; stripped
nearest-symbol names are not used to infer internal x264 routines.

Owned-process ticks for the Chariox repeat over a 9.754-second motion interval:

| MP-10 owned component | Logical cores |
|---|---|
| Xvfb | 0.0523 |
| Kernel plus embedded relay | 0.4757 |
| Node controller plus pixel workers | 0.6572 |
| Python XShm helper | 0.3168 |
| Python encoder | 0.2696 |

The largest excess is Node plus kernel, not only Python encoding. A separate V8
profile attributes main-thread self samples to GC (657 ms), stream writes
(507 ms), buffer copies (316 ms) and zlib chunks (327 ms). Pixel workers spend
603 ms in synchronous zlib and 478 ms in PNG row decoding. These span a complete
run, not just its motion interval. Exact verification repeatedly crosses CDP,
PNG decompression and policy/IPC scheduling. Typing every 100 ms crosses the
80 ms quiet threshold and can contend with approximately 100 ms exact capture;
this is a plausible latency mechanism, not a demonstrated counterfactual fix.

## MP-08/MP-10 pixelflux design and reuse decision

The installed implementation is **Rust/PyO3**, with rayon, x11rb, SIMD YUV
conversion and x264, rather than a Python/C++ encoder. X11 capture uses three
pooled shared-memory surfaces and separate capture/encode threads. Its stripe
software path hashes stripes with xxh3, avoids repeated hashing for sustained
motion, converts changed rows in parallel, and keeps encoder/YUV buffers per
stripe. Software H.264 uses bounded independent horizontal stripes (up to eight;
aligned row heights), with its own reference chain per row. JPEG is independently
decodable. Paintover is lossy and does not provide our exact settled RGB promise.

Encoded packets have bounded geometry/type/frame-id headers. Python gets a
read-only memoryview of native Arc-backed encoded storage and fanout queues share
that storage. When a dependent packet is dropped, the affected row waits for an
IDR. Client decoders composite at the row offset; separate streams keep separate
reference state. These are useful designs to carry into our existing encrypted,
credit-controlled kernel path, not a reason to adopt Selkies input/authority.

Direct Python reuse is **not a drop-in**. `ScreenCapture` joins capture and encode
and grabs an X11 root/region. Our browser source is an attested, owned browser
backing pixmap, masked under kernel protection/document policy. The API does not
expose an encoder for an already admitted raw buffer or owned drawable;
`stripe_frame_from_buffer` wraps already encoded bytes. Reusing root capture
would weaken MP-08/MP-11 source scope. A safe library integration needs an explicit
admitted-buffer/drawable API and the same invalidation/masking fences.

The matching wheel is 25,831,211 bytes, with roughly 240 Rust dependencies and
Wayland/Smithay/DRM/compositor code beyond this lane's browser needs. Prefer an
original bounded stripe implementation using the existing capture helper/PyAV
dependency, measuring it before accepting that Python meets the target. If
native conversion/hash remains necessary, isolate a small native module rather
than importing a second compositor stack. If any upstream source is copied,
retain that file under MPL with its notices; do not paste it into MIT files.
This is a proposed implementation choice, not a performance claim.

## MP-11 licenses and exact evidence

Selkies 2.0.0 and both inspected pixelflux revisions carry MPL-2.0. Exact
16725-byte LICENSE copies all have SHA256
`1f256ecad192880510e84ad60474eab7589218784b9a50bc7ceee34c2b91f1d5`.
They are retained under evidence `licenses/`, with `manifest.json`.
Third-party inventory hashes: locked pixelflux LICENSES.md
`9582d5946852142f8dab87c00c719d97d20c685700030a3bc9d9a0d0e07b64e2`;
matching release LICENSES.md
`a5c112d96d0508b5cd6d4c95908207d25cfe5ec24ca8b409b7e38144ae459668`.
Wheel license files and exact GPL-2/LGPL-2.1 texts are retained too.

MPL permits dependency reuse and a larger work under another license, while
covered source and modifications remain MPL. Preserve notices and provide the
corresponding MPL source and source-access notice when distributing executables.
New files containing no MPL code can retain the repository license. See the
[exact Selkies license](https://github.com/selkies-project/selkies/blob/2.0.0/LICENSE),
[pinned pixelflux license](https://github.com/selkies-project/pixelflux/blob/1ebcb0a10a96caeba20e1ad09be47cc018a55238/LICENSE)
and [Mozilla's FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/).

MPL is not the only dependency obligation: the matching release's default wheel
includes GPL x264, GPL x265 and GPL-built FFmpeg. Its non-GPL build selects
source-built OpenH264/BSD and non-GPL FFmpeg/LGPL. The older locked inventory
describes a different footprint; do not apply it to the measured wheel. Any
distribution must also meet the actual bundled libraries' licenses; putting
them in a helper process does not remove their obligations. Primary inventory:
[matching release LICENSES.md](https://github.com/selkies-project/pixelflux/blob/1ebcb0a10a96caeba20e1ad09be47cc018a55238/LICENSES.md).
No upstream code or dependency was added to Chariox in this phase.

## MP-08/MP-10/MP-11 coordinator allocation request

Current protocol441/relay84 offers one whole-frame `video` decoder/reference
chain. Its output must match the full canvas or the allowed downscales, and it
draws at (0,0) over the entire canvas. Existing `tiles` are PNG-only, each at most
128×128. Independent H.264 stripes cannot be represented by either contract.
An opaque stripe bundle inside `data_base64`, a new codec marker, or silently
reinterpreting tile bytes still changes serialized transport semantics.

Proposed minimum contract, for coordinator review before allocating a number:

- Explicit capability offer for stripe frames; existing clients receive the
  existing path or fail admission clearly. Raise client minimum only where used.
- A bounded `stripes` frame kind with the existing subscription/generation/tab/
  document/sequence/canvas geometry, a displayed `base_sequence`, and at most
  eight full-width rows. Each row carries y, height, codec, key, reference
  sequence and encoded bytes. Keep the existing payload ceiling and encrypted
  relay/credit transport; relay remains opaque.
- One encoder/decoder reference chain per row. Validate nonoverlap, dimensions,
  document and reference before atomically drawing all decoded rows. Dropped
  row dependencies force a new IDR. Navigation, protection changes and takeover
  reset applicable chains and invalidate queued unprotected pixels.
- Keep exact PNG/tile settle on the same admitted source; emit only changed
  rows during motion and stop media bytes while unchanged. Scheduling exact
  work must preserve verification protection without blocking each key echo.
- No cursor/copy-rect additions in this first reservation; evaluate them after
  the bounded stripe path works, rather than guessing further shapes now.

Allocate the shared local version and decide whether relay version also needs
an allocation. Implement snapshot/hash guards and focused transport drill with
that allocation. No number was chosen by this lane. No change was smuggled under
the existing flag or codec label.

## MP-11 review inbox and MP-08/MP-10 acceptance gate

The 09:33 UTC P2 review on `1b4d98783` is reproduced RED:
`default-dpr-red` admits 1920×1080 with omitted viewer DPR becoming 2, then fails
the first frame. Receipt/log and the admitted binding are retained. The pair
must be rejected before emulation/subscription mutation, and the shared viewer
default must be supported. This remains **unfixed**, pending the versioned
redesign; it must not disappear from the next review mapping.

After allocation: fail-first real-client stripes/reference-loss/masking/settle/
idle/geometry drills, actual app entry plus real kernel/relay and provider where
affected; then the full 15-row comparison. Source/component tests and these
fixture profiles do not close MP-08/MP-10/MP-11. The current kernel authority,
protected capture, scoped relay admission and signal ownership checks remain
security-critical review anchors under narrowed MP-11.

Build exit0 and successful base profile runs establish diagnostics only. Initial
profiling attempts failed on helper script accessibility/name shadowing and one
missing runner argument; their original RED logs are retained. Runtime states,
identities, owned scratch, namespaces and the redundant copied ELF are cleaned
after profiling, with exact-path inventory in evidence. Shared Cargo outputs,
existing provider profiles, keys, reviewer state, other lanes and Docker caches
are excluded. Local report commit only; no push, CI, PR, deployment or shared
service change.
