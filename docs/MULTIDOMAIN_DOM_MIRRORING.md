# MP-08/MP-10/MP-11: kernel browser DOM mirroring

The user-domain browser now has a DOM transport and a shared reference renderer.
The screen tiers are native App views, mirrored web pages, then protected video.
This implementation targets host Chromium on Linux. It does not introduce a
browser inside a slice, a Cloud runtime proxy, or a second input authority.

## Contract and authority

Local protocol **433**, relay peer **79**. Set `CHARIOX_KERNEL_BROWSER_MIRROR=1`
on the kernel. The shared client requires 433 only when attaching this feature;
older unrelated feature minima remain unchanged. Commands are terminal-only:
`mirror_subscribe`, `mirror_next`, `mirror_close`, and `mirror_input`.
The kernel derives the user and terminal actor; the caller cannot supply either.
Native App targets are refused and keep their native capability frontend.

Subscribe selects the existing canonical 1280×800 CSS viewport at DPR 1 or 2.
There are at most eight subscriptions; idle subscriptions expire after 60 seconds.
Every next call grants one packet of credit. A lost base or changed document or
protection policy sends a reset. In-flight policy changes fence replies and clear
private bases; delayed focus/text/selection dispatch rechecks the current policy. Packets carry document/generation/sequence,
stable document-lifetime node IDs, ordered child links, removed nodes, computed
styles, form and scroll state, resources and opaque tiles. The client checks a
SHA-256 hash of the canonical sanitized tree against the real source snapshot.
Mirror packets are excluded from command-result retention and automatic replay.
Unknown input outcomes are never retried.

The observer runs in the existing #607 isolated world. A MutationObserver watches
light DOM, open shadows and accessible nested documents. On credit it samples
current computed style, values, selection/focus and scroll, then sends a diff.
The bounded observation still samples current state on every credit, including
property-only form changes. Private isolated-world deltas avoid transferring
unchanged nodes over CDP. The kernel retains a separate sanitized base before
resource remapping or fallback, and canonical node hashes are cached without
changing the public hash contract. Mutation records never cross the boundary.
Computed CSS omits trusted Chromium initial values and starts each element with
`all:initial`. Those defaults come from a short-lived blank target, never origin
content. Simple unchanged text blocks below the viewport reuse sanitized styles;
geometry/effects and raw protected text are checked each read. When a block moves
or becomes visible, its styles are read completely before delivery. Registered
Vault values disable that cache. Initial offscreen flow blocks retain geometry
and hydrate descendants on the following credit, so visible content paints first.
The existing pull-credit protocol remains; push events need coordinator allocation.

## MP-11 protection boundaries

Page scripts, event attributes, origin addresses, navigation/form destinations,
active markup, custom CSS properties and executable/network CSS are absent from
the wire. Computed style is the CSS representation; source stylesheets never run
in a client. The reference iframe permits same-origin parent DOM access but no
scripts, and enforces CSP with no origin connections, forms or active content.
Only kernel-created image/font Blob URLs are permitted. The client independently
validates tree shape, CSS, attributes, bounds and resource hashes before rendering.

The existing observation policy supplies Vault values and registered target
regions. Text is scanned before truncation across adjacent nodes, open shadows
and same-origin frames. Protected fields/regions become opaque placeholders,
with no descendants, values, attributes, pseudo content or resources. Password,
OTP/payment autocomplete and explicit observation-protected regions are also
masked. An unknown registry refuses packets. Resources are refused whenever
Vault values are registered; images become protected fallback regions.

The kernel reads only already-loaded, CDP-listed resource bodies. It never fetches
an arbitrary client URL and sends no URL, cookies or request headers. Resource
admission checks binary image/font signatures, decoded image dimensions, font
size declarations, count and byte bounds. URL caches expire each observation,
and the renderer releases unused Blob URLs. SVG/CSS/HTML/script bodies fall back
to protected pixels. Packet bounds are 4 MiB, 12,000 nodes, 128 resources and
64 visible tiles; unsupported/bounded observer snapshots use a full video region.
If a packet/capture cannot meet its bound, the attachment fails closed and the
client must select the existing display transport.

Tiles use the same protected compositor capture as display, bounded to visible
regions and cropped by the kernel. Native controls, canvas/video/WebGL/plugin content, unavailable resources,
closed shadows and cross-origin frames use opaque tiles. Structured mirror
capture inspects open shadow descendants using trusted CDP metadata; Chromium
user-agent shadow roots do not turn ordinary controls into secret fields.
Same-origin frame tile bounds are translated into root compositor coordinates.
Trusted CDP frame origins and exposed documents permit nested protection scans;
explicitly protected frames retain their masks. Password/payment attribute checks
are ASCII case insensitive, including nested trusted CDP metadata. Closed page roots and foreign or
unavailable frame regions remain masked under the conservative protection contract.
Fully opaque regions use placeholders without source readback. Nested and shadow
images are decoded before apply completes. Thus cross-origin frame content is currently an opaque
protected fallback, not an independently authorized subframe observation channel.

## MP-08 input and client integration

Element actions resolve through the current isolated-world node map, check the
live document, visibility/occlusion and protection, and then enter ordinary
kernel browser input. Focus is checked again after page focus handlers, including
redirected focus. Click, text, scroll, key, selection and IME composition are
supported, with ordinary coordinate input as a fallback. Source selection drives
the existing Notes observer; clients do not submit page text as a note authority.
Region captures retain canonical browser coordinates and the existing protected
capture route. Mirror input shares the video actor ledger, takeover, cancellation,
held-input cleanup, terminal lifetime and generation semantics.

```ts
import { attachBrowserMirror } from '@chariox/kernel-client/browser-mirror'
const mirror = await attachBrowserMirror(transport, container,
  { tab_id, generation, device_scale_factor: 1 }, handleFailure)
await mirror.next() // repeat with one outstanding credit, while the view is visible
await mirror.takeover() // existing display actor contract
// mirror.input(...) for explicit element actions; renderer also forwards user events
await mirror.close()
```

Cloud owns production UI integration and scheduling credits. The reference
renderer restores nested inert documents, open shadow slotting, styles, form state,
focus and scroll. It preserves text nodes across unchanged updates, avoids resetting unchanged
attributes/styles when only children change, and creates no empty font/pseudo
stylesheets. These prevent redundant layout invalidation during SPA batches. It checks
geometry before each next call and requests sticky per-region video fallback for
layout, text or color drift at a 0.5 CSS px geometry threshold. Full-root drift
uses a stable inert full-video document. Tiles occupy integer source raster
positions independently of fractional control boxes. Protected placeholders
preserve border-box dimensions and inline replaced-frame layout. The renderer applies `all:initial` before vendor properties regardless of
serialized map ordering. Mask overlays use the same
outward floor/ceil plus four native pixels as protected source captures. The
source hash is checked every packet. Runtime compositor pixel comparison still
needs a reviewed client readback seam; geometry/text/color checks cannot detect
all raster-only paint differences.
A subscription failure closes the frame and notifies the caller; the caller
negotiates the existing display fallback, using its unchanged authority path.

## MP-10 validation and remaining acceptance

`packages/kernel-client/browser-mirror-drill.mjs` runs the real host controller,
sandboxed non-root Chromium and this renderer against synthetic docs, forms,
60-node text/color mutation SPA, open shadows, same/cross-origin frames, canvas/video and long
scroll fixtures at DPR 1/2. Arguments are absolute external tools, compiled client
and evidence paths. It requires public Chromium dependencies and fonts; its
lane-owned exact-path AppArmor user-namespace profile and temporary profiles are
removed on exit. No global sandbox setting or durable credential is changed.
It records initial and settled-after-mutation native raster pairs, raw MSE/mismatch and source-only one-native-pixel
edge-band mismatch. Client-only holes cannot enlarge the exclusion band. Box and
each text line fragment geometry, exact text and computed colors are measured.
The gate is <=0.5 CSS px geometry, exact text/colors and <=0.10% raster mismatch
outside that band. It also records bootstrap/hydration/patch bytes, per-stage CDP,
serialization, decode and renderer spans, ten-sample input/presentation and
mutation/paint distributions, owned-process CPU tick samples,
resource floors and cleanup. Mutation latency includes the source CDP call and
credit round trip; it is not a passive push/WAN measurement. Presentation ends at
a viewer rAF callback after native compositor readback verifies the expected
binary acknowledgement, using its source geometry (including scrollbars). This
matches the display lane's software endpoint, not physical monitor timing. The
synthetic kernel adapter uses bounded same-origin loopback HTTP JSON; it is a
fixture transport with Rust-style sorted maps, not a production Cloud proxy or
encrypted relay validation.
Automation receives packet JSON strings rather than recursively serialized JS
objects, so large diagnostic returns do not dominate the transport measurement. The
latency gates are input p50 <=80 ms / p95 <=120 ms, mutation p50 <=80 ms and long
page first meaningful paint <=500 ms. The first-paint metric includes native
viewer readback and rAF, before source screenshot analysis. Fixture navigation
occurs after selecting DPR so fonts are laid out at the requested scale.

Rust tests cross two authenticated local transport connections through production
terminal routing and the JS host adapter over synthetic CDP. They test stream
privacy, transient receipts and the shared takeover ledger. Node/shared-client
tests cover sanitization, decoded-resource bounds, protection resets, generation,
stale input, drift, App exclusion and unknown-outcome replay. These complement
rather than replace real Chromium, relay and independent semantic review.

Round 2 fixes the known fractional tiles, mask edges, native controls and
same-origin frame seams. Component fidelity can pass while latency remains RED;
retain each failing receipt and exact execution identity. Neither one fixture
matrix nor source checks close an MP item. Keep the feature opt-in until runtime
raster drift/fallback and production clients satisfy the complete gate. The
existing display plan reports another source and relay topology; do not relabel
its benchmark as this component run. Production Cloud/relay/WAN parity, OS-level
IME, animated media performance, Room rollout, macOS replay and current security
anchor semantic review remain acceptance work. MP-11 does not require exact-blob
review of unrelated non-security source.
