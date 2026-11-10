# Multidomain display transport — MD-DISPLAY-01/02/03/04

Phase 2 research and component prototypes, builder2, 2026-10-04/05. **Static
fidelity improves; a universal higher-quality replacement at comparable latency
is not established.** At a stated 2 Mbps PNG egress budget, small-change pages
converge to exact RGB at 83–96 ms median. Native RGB H.264 reaches 51–72 dB on
all frozen fixtures at a 2 Mbps VBV target and 80–153 ms median. Selkies remains
faster on media/dense scrolling. Public scrolling defeats the native RGB
ultrafast configuration; `veryfast` 8 Mbps only matches Selkies CBR 8 Mbps fidelity at
more than twice the p95 latency. These are measured limits, not acceptance.

MD-DISPLAY-04 recommendation: attach a transport-neutral frame/input seam to
kbrowser; deliver portable video with exact idle refresh and RGB dirty regions;
keep native 4:4:4 as a negotiated software option and DOM as a selective pilot.
Native signed App views remain valuable. Preserve Room desktop coverage until
replacement gates pass. **The owner decides the production design.**

The owner plan has no numbered MD rows. Lane labels: MD-DISPLAY-01 options/prior
art; MD-DISPLAY-02 headed host video/DOM/RGB; MD-DISPLAY-03 installed baseline;
MD-DISPLAY-04 API/migration/decisions. They do not redefine managed gates.
No MD, MP, Browser/Computer, Path-1 or M20 acceptance item is closed. No production
serialized shape changes or protocol allocation; kbrowser owns protocol 417.

## MD-DISPLAY-01 — What each option buys

“Send the actual HTML/JS” has two different meanings. Loading remote page code
in every viewer creates another live application with its own cookies, network,
timers and side effects. Mirroring sends sanitized rendered structure/state,
executes source scripts only in the kernel browser, and routes every action
back to that browser. The latter preserves one browser authority. It can keep
text sharp, but does not promise pixel-identical rendering across font stacks,
browser versions, operating systems or different viewports.

| Option | Fidelity: text, colour, scaling/HiDPI | Latency and bandwidth | Host/client CPU and GPU | OS reach and engineering cost |
| --- | --- | --- | --- | --- |
| Native signed App UI per viewer | Native text/selection/accessibility at viewer DPR; local fonts and transient state can differ from agent view. Colour is normal local web rendering. | Local UI feedback; kernel operations still take an RPC. Sends signed assets and state, avoiding video. | Normal client page cost; kernel still runs authoritative App state and the agent's browser copy. | Web on Linux/macOS/Windows, native clients need a webview/renderer and bridge. Medium cost: sandbox/origin, bridge identity, per-viewer state and shared actions. No universal arbitrary-site claim. |
| Sanitized DOM + CSS + assets, with pixel/video patches | Native sharp text and zoom. CSS, pseudo-elements, fonts, shadow roots, CSSOM, animations and layout can drift. Pixels/DRM remain raster. Mixed native/raster colour pipelines need testing. | Sparse mutations can be cheap; full styles, initial assets and many patches can exceed video. Source action → observer → patch apply adds a turn. Client selection can be immediate; authoritative form input cannot just mutate a local clone. | Source observation/style/layout work plus viewer layout; patch capture/encode remains. Our computed-style prototype duplicates some layout work. | Structure protocol is OS independent, fidelity is not. High cost for general web compatibility, resources, drift detection, stale refs, composition and masking. Prototype works on limited controlled pages. |
| Drawing commands / Skia pictures | Potentially scalable glyph/vector geometry and high fidelity, including some canvas. Fonts/images, shader/video paths, colour management and unsupported operations still matter. | Resource caching and command deltas can reduce bytes; decoding/replay/round trips remain. A bandwidth/latency win is unmeasured here. | Instrumented browser recording and client renderer, often WASM; GPU and resource caches needed. | Client portable in principle; modified Chromium hooks require per-platform maintenance. Very high cost; not a documented stable CDP remote compositor feed. Keep as research. |
| Chromium CDP screencast → encoder → existing encrypted display channel | DPR2 source captures work here; video quality depends on codec, bitrate, chroma subsampling and decoder scaling. Text does not become resolution independent. PNG source is lossless before encode. Page content only. | Event-driven capture is useful for static pages; base64 PNG/CDP decode then encode costs latency/CPU. Bytes are content/codec dependent. | PNG encode/decode plus video encode is an avoidable extra pass. WebCodecs exposes codecs, not a guarantee of GPU hardware support. | CDP is available on Chromium across Linux/macOS/Windows; actual support and latency still need OS tests. Medium cost: frame adapter, codec negotiation, rate/queue control, decoder lifecycle. No desktop/native dialog capture. |
| Tab capture / native window capture → encoder → encrypted channel | Removes PNG intermediate; can retain DPR2 pixels. Native capture covers chrome/dialogs/desktop as selected. Video still has chroma/scaling/colour tradeoffs. | Better capture/encoder path is a plausible improvement, not proved by this prototype. | Hardware encoders may reduce host CPU; software fallback required. Client decode usually cheaper than re-layout. | Tab capture requires a trusted Chrome extension and API permission/activation contract. Native capture needs OS adapters, permissions and tests. Medium/high cost; no Chromium fork required for capture APIs. |
| Existing Selkies / noVNC fallback | Raster desktop. Current installed Selkies component performs well on frozen-frame quality; cannot gain native selection or arbitrary zoom. Cursor metadata and viewer scaling are separate from pixels. | Installed Selkies loopback baseline is competitive; noVNC was not newly benchmarked. | X capture + encoder, client decoder. Existing Linux slice runtime cost. | Selkies captures the Linux display inside a slice even on a Mac host. It does not capture a native macOS/Windows browser. Preserve during migration; no Selkies-specific feature investment in this lane. |

The same display selection must work for either domain; domain attachment is
kernel state, not a codec or UI-side policy. A future native App view is an
additional authorized projection, not permission for its viewer to own App
bindings, critical actions or user-domain focus. User-only windows remain
unattached to Rooms; the focused agent's access follows the kernel's focus
record, as in the owner plan.

## MD-DISPLAY-01 — Interaction fidelity

| Interaction | Structure/native App views | Video/drawing views | Required kernel seam and evidence |
| --- | --- | --- | --- |
| IME and keyboard | Viewer composition, source caret, selection and acknowledgement can conflict. Unicode `Input.insertText` passes here; that is not a live IME proof. | Client composition can be forwarded as semantic text/keys; raster caret is remote. | Composition begin/update/commit/cancel, shortcuts, non-US keys, source focus, reconnect cancellation. Preserve ordinary controller input/arbitration. |
| Clipboard and selection | Native selection is valuable for notes/accessibility; copied text must obey source secret policy and selection anchors. | Source selection observer/clipboard is needed. Drawing commands alone do not provide semantic text. | Explicit authorized read/write clipboard operations, Vault restrictions, selection quote/range, size bounds and audit. No plaintext clipboard in relay control messages. |
| Drag/drop | Native DOM drag behavior differs from a reconstructed scriptless page. Need element and pointer semantics, source drag data and browser permissions. | Coordinate mapping is direct; local files and source drag payloads still need transfer support. | Source drag lifecycle and existing upload/download staging; fixture, cross-frame and real app tests. |
| File chooser and browser dialogs | Cannot execute chooser behavior in a source-free clone. | CDP page capture omits browser/native UI; OS/window capture may be needed. | Controller file chooser/dialog APIs + kernel RuntimeInteraction, safe upload staging. Video fallback does not automatically solve permission UI. |
| Cursor | Whitelisted computed cursor names can render natively; custom cursor URLs need authorized assets or default fallback. | CDP frame capture does not give a complete cursor-shape protocol. Host overlays must not force a crosshair. | Trusted cursor type/hotspot and actor overlay above content, one coordinate transform. Owner's Selkies/Mac crosshair complaint remains an unroot-caused report, not a reproduced defect here. |
| Multi-viewer focus/scroll | Per-viewer zoom/selection may be local; shared canonical source viewport and source mutation focus need an explicit contract. | Viewers share raster source geometry; independent resizing can fight without kernel ownership. | One tab, canonical viewport, input owner and action ledger; document/viewport epochs; stale input rejection and takeover across Web/TUI/native. |

The DOM prototype forwards actual native CDP mouse/text input to the source,
so the source's JavaScript handlers execute there. It does not copy those
handlers to the viewer. Element-addressed control passes on ordinary controls;
the cross-origin iframe patch uses mapped coordinates and verifies the source
button's effect. A page rendered as pixels can still be inspected and acted on
through the existing browser controller; agents do not need to act on the
viewer clone. Image-based computer actions need the exact source viewport
transform. Refuse stale epochs instead of guessing after layout drift.

## MD-DISPLAY-01 — Prior art and limits of the evidence

| Primary source, checked 2026-10-04 | What it establishes | What it does not establish for Chariox |
| --- | --- | --- |
| [rrweb guide](https://github.com/rrweb-io/rrweb/blob/main/guide.md) | Full/incremental DOM recording and replay, input/privacy options and existing machinery. | General live bidirectional control, Vault policy, hostile-source safety, or exact cross-OS layout. Its default input masking is not a Chariox policy. Consider its serializers after security review rather than assuming recorder completeness. |
| [Surfly technology](https://help.surfly.com/en/the-surfly-proxy), [security](https://www.surfly.com/security-and-compliance), [Cobrowse masking](https://docs.cobrowse.io/sdk-features/redact-sensitive-data) | Commercial co-browsing demonstrates interaction rewriting and pre-transmission element/field masking. | A drop-in open-source transport, compatibility without integration, or a safe reason to move cookies to a relay. Their deployed architectures differ from Chariox's encrypted transport-only relay. No service was bought or tested. |
| [Cloudflare NVR overview](https://blog.cloudflare.com/cloudflare-and-remote-browser-isolation/), [canvas remoting](https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/canvas-remoting/), [limitations](https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/known-limitations/) | Drawing-instruction browser isolation is a deployed product; difficult canvas content can still need a separate remoting path. | Universal perfect canvas/DRM fidelity, an available Chariox Chromium fork, or measured cost/latency on these clients. We did not prototype NVR. |
| [Chromium Paint Preview](https://chromium.googlesource.com/experimental/chromium/src/%2Bshow/refs/heads/lkgr/components/paint_preview/README.md), [player](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/components/paint_preview/player/) | Skia pictures + metadata can record/replay page previews; existing compositor/player code is useful research. | A real-time remote compositing protocol. Previews use capture/compositing/player components; the documented player is principally Android. “Chromium remote compositing” is not an off-the-shelf stable CDP draw-command stream. |
| [CDP Page](https://chromedevtools.github.io/devtools-protocol/tot/Page/), [#607](https://github.com/charioxai/chariox/pull/607) | CDP exposes screencast, screenshot, frame acknowledgement and isolated-world creation. Frozen source has a controller-owned focus world. | CDP screencast is experimental, not a codec stream or desktop capture. The referenced #607 page was not retrievable in this run; frozen source focus-world code is the implementation reference, not proof of a DOM resource broker or secret boundary. The isolated world shares the DOM with page code. |
| [WebCodecs specification](https://www.w3.org/TR/webcodecs/), [Chrome tabCapture](https://developer.chrome.com/docs/extensions/reference/api/tabCapture) | Browser codec configuration/support probing and encode/decode queues; a documented tab media capture API exists. | Mandatory codec/hardware availability, transport, security admission, or automatic background capture permission. Probe supported configurations on both ends. Tab capture has its extension/activation requirements. |

## MD-DISPLAY-02/03 — Phase 2 method and bandwidth fairness

All comparisons use the same 960×600 CSS / 1920×1200 physical DPR2 fixture
geometry, source probe, viewer decoder and paired RGB metric. Source/client
Chrome 154 run headed, sandboxed and outside slices on independent Xvfb displays.
The installed baseline is Chromium 147 in an owned 2-CPU/3-GiB container. It is
an installed Selkies component, not a signed F/G2/Room or noVNC validation.
No production kernel, account, provider, Vault, hosted relay or external Apps
machine is involved. Application WebSockets are plaintext loopback; production
must use existing encrypted delivery with kernel admission.

Each cell has 20 actual viewer clicks. Latency is harness click submission to
matching source probe read back after viewer rAF. It includes Playwright/polling
and excludes physical panel scanout. DOM acknowledges its mirrored probe after
rAF rather than checking raster pixels. Ingress-only latency, raw samples,
nearest-rank p50/p95/p99 and histograms are in JSON. With 20 samples, p99 is the
maximum; shared-builder load and browser differences preclude a production SLO.
The public scroll experiment has 10 wheels, checks source scroll and the first
subsequent viewer output; it is a weaker freshness proxy, not a frame-matched
physical input-to-photon measurement.

Frozen full-frame RGB MSE/PSNR compares source and viewer. `lossless:true` means
zero RGB error at identical dimensions; null PSNR alone never means exact.
Intentional redaction/caret/focus differences stay in the metric. PSNR is not a
text-readability, HDR, colour-management or moving-frame guarantee. A separate
moving diagnostic pairs timestamped input PNGs with decoded video and introduces
extra PNG/readback overhead. Source PNG capture-only pairs are exact in the
measured portable-video fixtures: their loss comes after capture.

### MD-DISPLAY-03 — What the baseline actually does

The unchanged installed parser selects **software x264, CRF25, paint-over
CRF18 enabled, fullcolour false, nominal 8 Mbps and 30 fps**. The 8 Mbps value
is not a bandwidth cap in CRF mode. The harness records only allowlisted,
non-secret installed parser settings and packet counts; it never reads private
live configuration. Baseline CBR ladder explicitly selects 0.5/1/2/4/8 Mbps,
turns paint-over off and records resolved initial values. Default CRF remains a
separate point, not a fictional 8 Mbps CBR point.

Portable WebCodecs High H.264 / VP9 / AV1 select constant bitrate, realtime,
software preference, DPR2 PNG capture and a 2-second timestamp-based keyframe
policy. Actual browser rate control remains an implementation result. Native
RGB x264 uses CRF18 plus target-rate VBV with a 2-second buffer, not CBR. Exact
PNG is paced at the selected application egress rate, with bounded credits and
atomic tile batches. None of these configured targets can substitute for
measured traffic. Curves expose configured **and** observed rates separately.

Byte rates include video packet payload or complete DOM/video/patch messages as
labelled, but exclude source-local CDP/base64, Python IPC, encryption/TLS, WS
headers and relay fragmentation. Fixture rates include bootstrap and settled
measurement over the reported interval. One-second maxima expose bursts.
Public latest receipts separate bootstrap bytes from bytes after bootstrap;
older public totals include bootstrap while their duration excludes it and
must not be called a steady-state bitrate. Different content cadence and a
short VBV experiment can exceed target averages; production needs a real shared
egress scheduler with a specified burst allowance, not just encoder hints.

### MD-DISPLAY-02 — Higher fidelity configurations

The initial High H.264, VP9 and AV1 WebCodecs ladders still trail Selkies on
text, even at 8 Mbps. Chrome's encoder rejects AVC High 4:4:4 / VP9 profile1;
its **decoder supports both** here. HEVC encode/decode probing is unsupported
in this Chrome build. Support probes are separate from actual decoded-frame
proof. Browser build and negotiated client capabilities determine availability.

A native PyAV16.0.1 / Pillow11.3.0 `libx264rgb` probe avoids chroma subsampling:
RGB24, five threads, ultrafast, zerolatency, CRF18, repeat headers, keyint60,
scene-cut off, target VBV and 2-second buffer. Full-range GBR/sRGB encoder VUI
and decoder colour metadata are explicit. A single frame decoded without the
correct colour metadata turned white pink (15.9 dB); that result is retained.
A corrected unconstrained one-frame colour probe reaches 56.4 dB but is **not**
a latency/bandwidth result. Live ladder results below include capture, software
encoding, WebSocket, browser decode and input acknowledgement. Native Python
base64 IPC is an experimental adapter, not the proposed shipping dependency.

Exact RGB is a complementary mode: browser-native image decode/readback,
Uint32 dirty comparison, 128-pixel PNG tiles and at most eight in-flight credits.
It chooses one full original PNG when smaller, applies tile batches atomically,
and suppresses unchanged frames. Full snapshots are paced too. A final settled
refresh establishes convergence; moving-content freshness is not established
by that exact frozen pair. Source-difference dirty updates are distinct from
repairing errors across an entire lossy video frame.

Every portable-video ladder cell also tests an explicit settled repair after
stream stop/flush: compare decoded RGB with source, pace PNG correction tiles,
then verify zero RGB error. A real source click followed by source-difference
tiles also verifies exact pixels. These 150 settled/dirty image checks across 75 cells pass; this is
**not** an automatic idle scheduler or production epoch fencing. Production
must prevent a later video frame from overwriting corrected pixels.

### MD-DISPLAY-02 — Selective structure and hybrid

DOM-only docs/SPA remain the strong selective case: exact docs and 85.6 dB SPA,
74.5/81.4 ms median,95.5/95.2 ms p95 in `phase2-dom-switch`. Every source action
runs on the original browser. One switch per page costs 323/298 ms to first
video,19/42 ms for full DOM restoration with every patch rAF acknowledged;
restored frame fidelity is recorded. This is a one-shot loopback measurement,
not a robust switching policy or generic-site result.

Hybrid encodes the **full page**, crops decoded opaque regions and re-encodes
those regions as PNG images in a scriptless DOM viewer. It demonstrates
composition/input but saves no regional video bandwidth. Media/iframe frozen
PSNR 50.9/43.0 dB, median 632/127 ms, p95 932/144 ms; owned CPU 642/153% of one
core. DOM→video227/229 ms, reverse200/190 ms; restored settled patches96.8 dB.
All 20 clicks and the cross-origin source button pass. Count **both** DOM and
video egress; encoded-only rates undercount this method. An attempted direct
canvas patch presentation showed blank opaque regions despite passing inputs;
`phase2-hybrid-final` is MEASURED_UNACCEPTED (17.7/12.9 dB). Restoring the
validated image path fixes the first presentation seam; no general canvas
lifecycle diagnosis or optimized regional encoder is claimed.

### MD-DISPLAY-02/03 — Evidence provenance and retained RED results

Evidence stays under `/root/.codex/evidence/browser-resume-20260930/display/`.
`phase2-review-v2/report/index.html` provides configured/observed curves, raw
per-cell rows, source/viewer opacity overlays and amplified diffs.
`summary.json` binds rows to exact receipt SHA-256. `PHASE2_MANIFEST.json`
records commands, exits, source identities, resources and cleanup. No source
identity is reassigned to the handoff commit.

- Core codec/default-baseline matrix: clean `0a92ef8d9`,105 cells/2,100 clicks,
  75 portable-video cells plus 30 baseline cells. Selkies CBR 2 Mbps form is RED at a
  measurement-stage decoder queue bound; child exit 1. The old campaign did not
  propagate child failures to its parent exit status. The synchronous PNG
  comparison left producers running and accumulated frames. Stop/settle before
  comparison corrects this harness seam. Clean `6a116581b` rerun passes all five
  CBR2 cases; raw failed cells stay visible rather than overwritten.
- Native PNG ladder: clean `6a116581b`, five rates×five pages, all 20 inputs/cell,
  exit 0 and exact settled pairs. Latest all-page 2 Mbps confirmation is clean
  `ea08ec18b`; public 8 Mbps and DOM switch are clean `c7c60a426`.
- Hybrid image composition: clean `d211d4e7b`. Earlier blank-canvas hybrid and
  public window-only scroll timeout at `6a116581b` stay unaccepted. The public
  site scrolls inner containers; checking source total scroll fixes the test.
- First Phase 2 curve campaign was cooperatively interrupted to bind clean source
  identities; it is diagnostic (parent 130/child 1), not a completed ladder.
- Native RGB H.264 ladder: clean `18ff62cdc`, 25 cells/480 successful probes;
  0.5 Mbps docs RED (zero probes), other four campaigns exit 0. Preset4/8 Mbps
  diagnostics are clean `0fdeae013`. Pixel-quality failures remain unaccepted
  even when probe counts and process exit pass. Full identities are in manifest.

Post-review clean source `a0c88cce7` has 39 fixture cases / 780 input trials:
High H.264, VP9, AV1, paced exact RGB and native RGB at 2 Mbps; docs/SPA DOM
switches; media/iframe hybrid switches; default and CBR2 baseline. All nine
positive runs exit 0 with empty owned cleanup. The expected missing-Chrome run
exits 1, writes a RED receipt and cleans acquired displays/profile/listeners.
`review-recovery/clean-confirm/PROVENANCE.json` binds each receipt and all 14
core module hashes, which match the final execution files. These are focused
confirmations, **not** a rerun of the full historical bitrate ladder or new WAN,
Vault, native-OS, physical scanout or production acceptance.

Seventeen focused tests pass at clean `a0c88cce7`. The real installed-baseline
callback fault run injects a send failure; it reaches the awaited failure path,
writes RED/exit 1, and removes its labelled container/browser/stream. Unit tests
also reject striped packets and malformed JSON. Safe signal tests cover invalid
and unregistered IDs, readiness failure and owned descendant teardown. Campaign
checks cover 0/1/124/130, signal/null exits, launch error and interruption; failure
aggregation is a new fix, not a claim about historical parent statuses.

Historical Phase 1/2 measurements do not validate changed lifecycle modules.
In particular `run.mjs`, `selkies.mjs` and `viewer.html` differ from Phase 1's
measured blobs. No historical receipt is assigned to the final head. The new
report keeps native encoder presets and rate-control series separate; its own
generator/file hash is recorded in `phase2-review-v2/report-validation.json`.
The 187-row report, point/axis selection, paired images and opacity controls
pass headed-browser validation; screenshots are retained outside Git.

Phase 1 evidence remains historical: H.264 300 inputs at clean `e183160b6`,
VP9/Selkies 100 each at clean `7f5cb0a11`; default Selkies 47–54 dB versus initial
portable H.264 35–44/VP9 36–45 dB. Phase 1 public DOM 33.01 dB remains
MEASURED_UNACCEPTED; public VP9 navigation failed ERR_NETWORK_CHANGED. Phase 2
updates those conclusions rather than relabelling those images or hashes.

Installed baseline tag `chariox-local-headed:f1c402b82aa0053bb69f0f8fffe04b06e02a5c73`;
immutable image `sha256:e76b80392f3368efaceed6e6636cc4d736fe57c25be0206ca4cf8f132a168f64`;
embedded source digest `5d65f6e6b40902c31995b1806b04b7d7a83edce6567933fc629987d489f3335e`
(64-hex digest, not a Git commit), embedded relay 68, Selkies source
`3f87241fcd6abc44e205b22f6596e78ef4946670`. Harness frozen G2 base is 411/70.
Image/browser differences stay explicit; no signed aggregate acceptance.

## MD-DISPLAY-02/03 — Fidelity versus observed bitrate

Each cell below is **RGB PSNR dB / observed application Mbps**, with target
columns 0.5/1/2/4/8 Mbps. Observed rate includes the fixture bootstrap and test
interval. The interactive report contains curves and all five pages, latency
histograms, bursts, CPU, screenshots/diffs, source identities and rejected cells.
CBR2 baseline cells below use the corrected rerun; the original RED form receipt
remains in the report. `exact` denotes zero RGB error at frozen convergence,
not continuously exact moving frames. This is a rate/quality tradeoff; identical
targets are not identical measured bandwidth.

### MD-DISPLAY-02/03 — docs curve

| Method | 0.5 Mbps | 1 Mbps | 2 Mbps | 4 Mbps | 8 Mbps |
| --- | ---: | ---: | ---: | ---: | ---: |
| Selkies CBR | 14.13 / 0.051 | 35.20 / 0.391 | 46.17 / 0.577 | 49.98 / 1.057 | 50.16 / 1.085 |
| WebCodecs High H.264 | 30.63 / 0.262 | 31.97 / 0.304 | 31.97 / 0.404 | 32.63 / 0.540 | 34.70 / 0.660 |
| WebCodecs VP9 | 27.76 / 0.208 | 27.76 / 0.247 | 31.77 / 0.359 | 34.16 / 0.577 | 35.53 / 0.627 |
| WebCodecs AV1 | 28.30 / 0.255 | 28.30 / 0.277 | 28.62 / 0.342 | 31.81 / 0.563 | 34.19 / 0.627 |
| Native RGB H.264 ultrafast | 16.74 / 0.121 RED | 35.20 / 0.447 | 51.06 / 0.917 | 60.20 / 1.299 | 60.17 / 1.107 |
| Paced exact PNG | exact / 0.282 | exact / 0.386 | exact / 0.499 | exact / 0.530 | exact / 0.537 |

### MD-DISPLAY-02/03 — media curve

| Method | 0.5 Mbps | 1 Mbps | 2 Mbps | 4 Mbps | 8 Mbps |
| --- | ---: | ---: | ---: | ---: | ---: |
| Selkies CBR | 13.21 / 0.042 | 42.79 / 0.310 | 47.19 / 0.425 | 47.45 / 0.489 | 47.47 / 0.572 |
| WebCodecs High H.264 | 43.35 / 0.423 | 42.46 / 0.425 | 41.95 / 0.414 | 43.70 / 0.431 | 43.99 / 0.436 |
| WebCodecs VP9 | 44.53 / 0.725 | 44.54 / 0.852 | 44.58 / 0.790 | 43.55 / 0.759 | 43.62 / 0.752 |
| WebCodecs AV1 | 44.22 / 0.322 | 44.11 / 0.347 | 44.08 / 0.321 | 44.10 / 0.314 | 44.39 / 0.294 |
| Native RGB H.264 ultrafast | 8.85 / 0.741 poor image | 61.52 / 1.498 | 71.60 / 1.541 | 71.84 / 1.586 | 71.56 / 1.514 |
| Paced exact PNG | exact / 0.463 | exact / 0.834 | exact / 1.410 | exact / 1.999 | exact / 1.958 |

The native 0.5 Mbps docs cell has zero successful probes and exits 1 at the first
visual acknowledgement; its 16.7 dB frame is not a passed point. Media 8.9 dB at
that target is visibly broken although 20 probe inputs passed. `PASS_PROTOTYPE`
in old/raw harness receipts means input count and comparable dimensions,
**not** fidelity or production acceptance. The native short-run media1 Mbps
average exceeds target (1.50 Mbps); the 2-second VBV target is not a hard egress
cap. Prototype pacing is required before a production bandwidth promise.

### MD-DISPLAY-02/03 — Static and moving cost at a stated 2 Mbps target

The default CRF baseline is uncapped; the portable modes have the stated 2 Mbps
budget/target. CPU aggregates owned host processes; baseline container cost is
separate. Exact/native columns use clean `ea08ec18b` / `18ff62cdc` respectively.

| Page / method | RGB PSNR dB | Observed Mbps | Click p50 / p95 ms | Owned host CPU % of one core |
| --- | ---: | ---: | ---: | ---: |
| docs / Selkies default | 48.30 | 0.482 | 69.23 / 90.56 | 95 |
| docs / Selkies CBR 2 Mbps | 46.17 | 0.577 | 70.31 / 74.72 | 89 |
| docs / Native RGB H.264, 2 Mbps | 51.06 | 0.917 | 95.24 / 125.57 | 115 |
| docs / Exact PNG, 2 Mbps | exact | 0.499 | 96.25 / 115.81 | 108 |
| spa / Selkies default | 53.68 | 0.165 | 69.53 / 71.06 | 87 |
| spa / Selkies CBR 2 Mbps | 51.46 | 0.321 | 74.41 / 95.24 | 101 |
| spa / Native RGB H.264, 2 Mbps | 65.25 | 0.352 | 80.72 / 85.82 | 110 |
| spa / Exact PNG, 2 Mbps | exact | 0.226 | 85.62 / 98.29 | 104 |
| form / Selkies default | 53.55 | 0.138 | 82.31 / 94.91 | 79 |
| form / Selkies CBR 2 Mbps | 50.39 | 0.244 | 81.28 / 96.14 | 98 |
| form / Native RGB H.264, 2 Mbps | 66.71 | 0.296 | 81.46 / 87.23 | 107 |
| form / Exact PNG, 2 Mbps | exact | 0.206 | 83.21 / 90.59 | 98 |
| media / Selkies default | 47.23 | 0.253 | 71.50 / 90.02 | 87 |
| media / Selkies CBR 2 Mbps | 47.19 | 0.425 | 80.54 / 95.45 | 94 |
| media / Native RGB H.264, 2 Mbps | 71.60 | 1.541 | 152.73 / 178.07 | 418 |
| media / Exact PNG, 2 Mbps | exact | 1.410 | 231.23 / 312.37 | 331 |
| iframe / Selkies default | 52.02 | 0.188 | 70.68 / 71.57 | 82 |
| iframe / Selkies CBR 2 Mbps | 51.93 | 0.358 | 71.22 / 103.12 | 100 |
| iframe / Native RGB H.264, 2 Mbps | 65.51 | 0.398 | 82.45 / 95.99 | 116 |
| iframe / Exact PNG, 2 Mbps | exact | 0.226 | 87.95 / 99.69 | 113 |

**Best demonstrated exact-text configuration:** DPR2, 2 Mbps paced RGB PNG,
128px dirty tiles/full-PNG size choice, at most 8 in-flight credits, atomic frame
presentation and unchanged-frame suppression. Small-change docs/SPA/form/iframe
have 83–96 ms median, 91–116 ms p95, 0.21–0.50 Mbps actual and 98–113% owned CPU.
This beats default Selkies on static RGB fidelity at a stated budget. Static
latency remains in the same 100 ms range, with 20–40 ms p95 penalties against
the faster default baseline and some comparable CBR2 cells. The owner has not
set a tolerance that would classify this as accepted latency parity. It is
not suitable as the only general display mode: media p95 312 ms versus default
baseline 90 ms and public scroll p95 1572 ms. Initial bootstrap
and dense whole-frame changes cost hundreds of KiB.

**Best demonstrated video text option:** native RGB 4:4:4 software x264 at 4 Mbps
VBV, CRF18/ultrafast, explicit GBR/full-range sRGB; frozen docs 60.2 dB at 1.30
Mbps observed and 118 ms p95 versus Phase 2 default 48.3 dB at 0.482 Mbps/91 ms.
It spends
more bandwidth for sharper pixels. At 2 Mbps it improves all five frozen fixture
PSNRs, but media latency 178 ms p95 still trails default baseline 90 ms. The `veryfast` 4
Mbps docs point 49.8 dB at 0.499 Mbps/p95 130 ms is closer in **actual** bandwidth
to Phase 2 default 48.3 dB at 0.482 Mbps/p95 91 ms; it is a scoped fidelity gain
with about 40 ms extra p95, not a
universal win. CPU 124.8% versus default host 94.6% plus container cost.

High H.2642 Mbps settled video repair costs 87–326 KiB and 699–1772 ms across
fixtures; a source-difference update costs 0.8–4.0 KiB and 23–37 ms. The static
PNG bootstrap (~97–250 KiB in fixtures) can be smaller than an all-tile repair;
production should compare encodings before sending a correction. Actual bytes,
frames/full-frame choices, initial bytes and maximum frame sizes stay in JSON.
Encoder refresh policy alone does not solve stale patch/video composition.

### MD-DISPLAY-02/03 — Public dense scrolling boundary

Actual public DevTools documentation, 10 wheel inputs, same DPR2. Modern rows
count bytes **after bootstrap** over scroll plus settling/final-capture time;
bootstrap bytes remain separate. All are MEASURED_UNACCEPTED, with the weaker
first-output latency proxy described above. Final frozen image quality is not
moving-frame freshness or a general-site acceptance.

| Mode / target | Frozen RGB PSNR | After-bootstrap Mbps | Wheel p50 / p95 ms | Bootstrap KiB |
| --- | ---: | ---: | ---: | ---: |
| Exact PNG, 8 Mbps | exact | 5.321 | 456.28 / 485.25 | 290.3 |
| Selkies CBR 2 Mbps | 23.81 | 1.103 | 59.38 / 86.40 | 9.5 |
| Selkies CBR 4 Mbps | 31.93 | 2.484 | 59.76 / 95.77 | 24.0 |
| Selkies CBR 8 Mbps | 37.45 | 5.430 | 59.96 / 75.48 | 58.2 |
| Native RGB ultrafast2 | 16.22 | 1.360 | 99.44 / 135.25 | 283.3 |
| Native RGB ultrafast4 | 17.75 | 2.986 | 112.16 / 152.96 | 507.2 |
| Native RGB ultrafast8 | 20.98 | 7.322 | 102.82 / 146.28 | 658.2 |
| Native RGB veryfast4 | 20.12 | 3.596 | 117.85 / 141.83 | 306.8 |
| Native RGB veryfast8 | 37.41 | 6.426 | 140.01 / 166.04 | 332.0 |

Exact PNG, 8 Mbps is still slow (485 ms p95); 11 full images, maximum 356.5 KiB/frame,
show why tile reuse cannot help much when most text moves. Exact PNG, 2 Mbps is 1572
ms p95 (older receipt totals include bootstrap; no steady rate inferred).
Native ultrafast scrolling shows visibly bad prediction/colour artifacts even
at 8 Mbps. Independent PyAV decoding of the saved 8 Mbps trace reproduces poor
public fidelity (19.2 dB): the first failing seam is the generated encoded
stream, not the viewer WebSocket/browser decoder. Internal codec cause is
unresolved; changing preset improves 8 Mbps to 37.4 dB but still fails the owner's
higher-quality/comparable-latency goal. Never use these points to justify a
production default. Default uncapped Selkies public pair is 40.2 dB/p95 64.8 ms,
but its older bytes include bootstrap and cannot be plotted as steady bitrate.

Separate moving diagnostics: High H.2648 Mbps three paired frames 42.66–42.80 dB;
native RGB2 Mbps one paired frame 63.17 dB. Diagnostic readback adds cost and
three captured candidates did not all yield paired native frames, so it is not
a three-frame native sample or a motion-quality distribution.

### MD-DISPLAY-02/03 — Resource and cross-OS limits

Selected Phase 2 receipts sample at least 22.03 GiB MemAvailable and 211.15 GiB
free disk, above16/10 GiB floors. Native software worker uses five encoder
threads. Owned host CPU includes source/viewer Chrome, Xvfb, Node and native
worker (12–21 ms typical per-frame PNG decode+native encode on simple pages).
Exact2 Mbps host CPU 98–113% static/331% media; native2 Mbps 107–116%
static/418% media. Evidence CPU/readback can add cost; RSS sums processes and
can double-count shared pages. No GPU utilization, hardware encoding, macOS or
Windows measurements were made. Baseline container samples remain separate;
no claim that host-only baseline CPU is its entire cost. Software-encoder build
and allocation differ from native OS hardware paths.

## MD-DISPLAY-01/04 — Security boundary

Keep confidentiality and authority in the kernel before serialization or
encoding. Encrypted content can cross the relay, but cookies, profile contents,
Vault values, reusable asset credentials and plaintext payloads must not enter
Cloud/relay routing metadata or logs. The relay needs only authenticated scope,
bounded opaque packet routing, backpressure and expiry. It cannot choose a DOM
node, decode a frame, proxy an authenticated image URL, or approve input.

| Threat / surface | Required production behavior | Prototype evidence / gap |
| --- | --- | --- |
| Executable HTML, script or event handler on client | Construct typed sanitized nodes in an isolated scriptless origin; deny scripts, event attributes, custom elements, navigation/forms and active embeds. Enforce sandbox/CSP; preserve trusted host UI outside content. | Narrow tag/attribute/style allowlists, DOM node construction and scriptless sandbox are exercised. No adversarial security review, parser fuzzing or production isolation proof. |
| CSS/fonts/images/network exfiltration | Parse/rewrite CSS with a real parser; no unmediated source URLs or credentials. Typed, epoch-bound kernel asset handles feed authorized per-viewer cached bytes. Validate MIME/size/origin and prevent SVG/script or CSS resource escalation. | URL-bearing computed styles and direct resources are blocked; opaque images use pixels. Authenticated resource proxy/fonts, CSSOM/adoptedStyleSheets/pseudo-elements are unimplemented. |
| Secret fields and reflected secrets | Kernel Vault target registry masks before DOM snapshots, attributes, accessibility projections, region capture, whole-page fallback and encode. Include unmasked/custom fields, cross-origin frames, reflected text, CSS attributes and dynamic navigation. No viewer-side overlay is a redaction boundary. | Password values become fixed placeholders and password-target replay is denied. No Vault was read; field type and page-authored private hints are not kernel policy. General secrets can still leak with this prototype; do not use it on authenticated pages. |
| Cross-origin frames/canvas/video/DRM | Use separately authorized frame observations or composited patches without copying cookies. Apply masks before pixels leave the host. Detect protected/unsupported capture and show an explicit unavailable/alternative state. | Separate-origin input effect and PNG patches pass on fixtures. DRM, tainted media, OOPIF lifecycle, frame crash, native overlays and Vault masks remain untested. |
| Source page attacks observer identity | Controller owns worlds, stable refs, navigation/viewport epochs and native input; page metadata is observation only. One observer/context per document; discard on navigation. | Isolated-world native DOM reads work; #607's focus policy is reused only as a pattern. IDs reset per document in this prototype; no production stale-epoch admission. |
| Forged input, duplicate actions, takeover | Admitted viewer identity, domain/window/tab binding, expected epoch, actor/input lease, finite normalized targets and kernel action ledger. Reconnect creates a new display generation. Input uses existing controller/kernel mutation arbitration, never a relay-side authority. | Local fixture input replay is real; production authorization, takeover, exactly-once semantics and permissions are not installed. |
| Slow/stalled viewer and terminal starvation | Bound frame/patch queues and byte budgets per viewer, drop/coalesce safe obsolete frames, request fresh keyframe/full snapshot after loss. Prioritize kernel terminal/control traffic; revoke stream promptly. | Prototype encode queue, decoder queue and socket payload/buffer bounds exist. No slow-viewer or shared terminal-load proof; memory/resource sampling only. |

DOM often expands the confidential surface relative to video: hidden text,
attribute values and non-visible structure can leave the source even though
they never appeared in a screenshot. Serialize only the permitted projection;
keep source profiles/assets in the kernel. Conversely, video can contain a
secret rendered by page JavaScript. Switching transport never replaces Vault
masking or viewing authorization. Native App UI must obey the same masking,
bridge identity and critical approval policy for each viewer.

## MD-DISPLAY-04 — Concrete internal seam proposal (Phase 2)

This is an internal API sketch for the kbrowser lane's protocol-417 browser
handle, not a serialized protocol definition or allocation. kbrowser owns
Chromium launch, sandbox/profile, CDP connection, tabs, source viewport and
existing controller actions. A display service borrows that handle. A slice
adapter can provide the same handle later without a client-specific authority.

```ts
interface BrowserDisplaySource {
  describe(tab: TabId): Promise<SourceGeometry>;
  frames(tab: TabId, request: CaptureRequest, signal: AbortSignal):
    AsyncIterable<CapturedFrame>;
  snapshot(tab: TabId, epoch: DocumentEpoch): Promise<SanitizedProjection>;
  // This delegates to the EXISTING kernel admission/action service.
  dispatch(actor: AdmittedActor, input: ExistingBrowserAction): Promise<ActionReceipt>;
}
interface SourceGeometry {
  browser: BrowserId; tab: TabId;
  documentEpoch: bigint; viewportEpoch: bigint;
  cssWidth: number; cssHeight: number; pixelWidth: number; pixelHeight: number;
  deviceScaleFactor: number; colour: 'srgb'; scope: 'tab';
}
interface CapturedFrame {
  geometry: SourceGeometry; sourceRevision: bigint; capturedMonotonicUs: bigint;
  pixels: OwnedRgbaBuffer | OwnedPngBuffer; release(): void;
}
interface DisplayProjector {
  open(source: BrowserDisplaySource, viewer: AdmittedViewer,
       capabilities: DecoderCapabilities, budget: DisplayBudget,
       signal: AbortSignal): AsyncIterable<DisplayUpdate>;
  acknowledge(stream: StreamId, revision: bigint): void;
  requestResync(stream: StreamId): void;
}
```

`CapturedFrame` is pre-transport and process-local. `Owned*Buffer` has an explicit
lifetime; the projector keeps at most one pending source image and a bounded
encoder queue. Acquiring frames must not expose a second independently-owned
CDP connection or launch another browser. Scope is explicitly tab-only: the
source cannot stand in for Computer's full desktop or native file dialogs.
Capture times must be translated to one kernel monotonic clock; they are not
assumed comparable across kernels or client machines.

A proposed `DisplayUpdate` has a **common envelope**: stream ID, document and
viewport epochs, source revision, presentation revision, mask-policy generation,
physical/CSS dimensions, DPR, colour space and capture timestamp. Variants:
`VideoConfig`, `VideoChunk`, `ExactFrame`, `ExactTiles`, `DomSnapshot`, `DomDelta`,
`OpaqueRegionMap`, `Cursor` and `Unavailable`. Payload and this envelope are
inside the existing authenticated encryption boundary. Relay-visible routing
contains only connection scope and opaque bounded packets. The relay cannot
select a codec, inspect a DOM node, decode a mask, fetch an asset or admit input.

An `ExactTiles` update carries `basePresentationRevision`, physical-pixel
rectangles, exact image payloads and a final `commitRevision`. The client applies
the batch atomically only to the matching base/epochs/mask generation. A newer
video frame either invalidates the tiles or declares compatible dirty regions;
it never silently mixes revisions. During resync, suppress obsolete deltas,
force a fresh decoder configuration/keyframe or exact full frame, then resume.
Navigation, resize, mask changes and revocation cancel capture/encoding and
invalidate cached DOM/assets/pixels. A changed masking policy must immediately
invalidate already displayed content, including an exact overlay.

`DisplayBudget` proposes `targetVideoBps`, `maxEgressBps`, a finite burst allowance,
`maxQueuedBytes`, `maxFrameAgeMs` and `idleRefineDelayMs`. Share the egress budget
between video and refinement. Prefer a compact exact full image for the initial
idle correction, source-difference tiles for subsequent small changes, and
coalesce/drop only deltas that preserve decoder state. Slow viewers must request
a new keyframe/base rather than retain arbitrarily many frames. Kernel control,
terminal and interaction traffic keep priority over display. This prototype's
encoder bitrate is only a target; production needs an actual egress scheduler.

Input reuses the existing kernel/controller action service. The display adapter
supplies source viewport/epoch metadata for mapping, never a new permission or
prompt path. Pointer positions map from CSS viewport to source CSS coordinates;
DOM actions carry a kernel-issued element ref plus document epoch and are
resolved on the source. On layout drift, reject/fall back before issuing a click.
Composition/IME transactions, key modifiers, clipboard permission, file chooser
and drag/drop lifecycle remain controller responsibilities. Cursor shape/hotspot
is a separate source observation, not a permanently drawn crosshair or a CSS
cursor inferred from the mirrored client. Multi-viewer input requires the same
Room/user-domain ownership and action ledger as today's kernel.

### MD-DISPLAY-04 — Attachment order and migration gates

1. kbrowser exposes a cancellable tab frame source and source geometry through
   its internal handle. Keep production protocol 417 under that lane; do not add
   a display shape without coordinator allocation, snapshot tests, client
   minimums and a focused drill. Wire a portable page-only fallback to the shared
   secure transport after those approvals.
2. Add decoder negotiation, sRGB/DPR2 fidelity pairs, cursor observation, source
   input replay, bounded delivery, revocation and navigation/resize fencing.
   Hardware encode is an adapter; a software fallback remains available.
3. Add budgeted exact idle refresh and dirty regions, with revision-atomic
   composition. Mask DOM and pixels **before** differencing, capture export,
   encoding, caching and any delivery. The fixture-only reconstruction evidence
   does not establish Vault secrecy or production epoch correctness.
4. Native signed App views can render locally after the owner chooses transient
   viewer state. Selective DOM text/SPA mirroring follows asset/masking/drift and
   input-target gates. Regional video is a measured optimization candidate;
   this experiment encodes a full page and gains no regional network savings.
5. Preserve existing Room desktop coverage until the replacement passes actual
   Browser/Computer, managed, M20, provider/Web/TUI and security gates on reviewed
   release identities. Native macOS/Windows, WAN loss/jitter, two viewers, slow
   viewers, soak, full clipboard/IME/filechooser and cursor shape remain required.
   Retire Selkies in a separate reviewed change only after those gates pass.

### MD-DISPLAY-04 — Owner questions after Phase 2

- Choose separate moving-content and settled-text fidelity gates, a video/total
  egress budget, acceptable idle refinement delay and input p95 under specified
  WAN conditions. Exact settled text does not establish a motion-quality win.
- Is page-only user-domain delivery the initial scope, with Room desktop retained?
- Which clients/codecs must work? H.264 plus PNG is the broad compatibility
  candidate; VP9/AV1 need per-viewer negotiation and real native-OS measurements.
- Approve or change the masking/geometry/revision gates, native App per-viewer
  transient state and element-vs-source-coordinate action policy.
- Accept selective DOM only after drift/security gates, or fund broader CSS,
  font/asset, shadow DOM and iframe work? This lane makes no default-DOM decision.

## MD-DISPLAY-01/02 — Native OS encoder expectation

The Linux measurements use software WebCodecs in a trusted encoder browser and native libx264rgb through a disposable Python worker,
including PNG decode/RGB→video conversion. That browser is a prototype device,
not the recommended shipping dependency. No hardware-encoder or GPU utilization
measurement was made. Per-process RSS includes shared pages; CPU aggregates the
owned source/client browser groups, Xvfb and Node, and includes the native worker, tile comparison,
PNG encoding and evidence work when enabled. Baseline container CPU is reported
separately. These costs cannot be extrapolated as VideoToolbox/MF utilization.

A shipping macOS adapter can encode with VideoToolbox; Chromium maintains a
[VideoToolbox encoder implementation](https://chromium.googlesource.com/chromium/src/+/main/media/gpu/mac/vt_video_encode_accelerator_mac.mm).
Windows has a [Media Foundation encoder implementation](https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/media_foundation_video_encode_accelerator_win.cc).
Those sources establish available implementation paths, not measured performance,
codec availability or 4:4:4 support on the owner's machines. Start with browser
CDP capture without OS screen-recording permission; later evaluate direct tab
frames/native window capture to remove PNG/base64 copies if the CPU/latency gate
fails. Native window capture adds OS consent and capture-scope work.

The [WebCodecs configuration contract](https://www.w3.org/TR/webcodecs/)
exposes bitrate mode, latency preference, codec profile and hardware preference;
capability probing and encode/decode success must both be checked. An encoder
preference does not prove which hardware ran. Common hardware paths use subsampled
video, so a higher bitrate does not by itself prove pixel-exact coloured glyphs.
Exact PNG refresh works in RGB independently of the video codec, at the price of
CPU, bursts and composition/epoch complexity. SDR sRGB/DPR2 is this harness's
scope; HDR/wide gamut and native macOS/Windows colour need separate measurements.
