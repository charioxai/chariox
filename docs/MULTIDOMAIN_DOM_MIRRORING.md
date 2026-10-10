# MP-08/MP-10/MP-11: kernel browser DOM mirroring

MP-08/MP-10/MP-11: the main-based round-2 successor uses local443 / relay86.
See `MULTIDOMAIN_ROUND2_ON_MAIN.md` for the exact imported source heads and
validation limits; earlier allocations below describe their branch history.

The user-domain browser now has a DOM transport and a shared reference renderer.
The screen tiers are native App views, mirrored web pages, then protected video.
This implementation targets host Chromium on Linux. It does not introduce a
browser inside a slice, a Cloud runtime proxy, or a second input authority.

## Protocol 489: DOM mirror v2 (current; introduced in 482, compact wire in 489)

`mirror_subscribe {wire: 2}` selects v2; the 443 computed-style packets below
remain for one protocol version (TUI/tests) and are superseded for web clients.
v2 follows rrweb's model instead of a per-credit computed-style dump:

- **One observer per viewer.** Each subscription has its own observer instance
  in the isolated world (ids, mutation journal, listeners), so one viewer's
  snapshot or reset never touches another's; it stops when the subscription
  closes or expires.
- **Snapshot, then deltas.** One pre-order snapshot per document (`reset`),
  then MutationObserver deltas as ops: `children` (reconcile a parent's child
  list; new subtrees carried as records), `attr`, `text`, `css`, `adopted`,
  `form`, `scroll`, `size`, `res`. Records travel as compact rows
  `[idDelta, parentBack, tag | kindCode, attrs | 0, extra]`; a stylesheet text
  of 2 KB or more travels once per snapshot epoch (`sheets` + `css_ref`).
  Deltas carry only changes: a child list that ends as it was sends nothing, a
  form op travels only when the field's state changed, and a subtree the page
  rebuilds with the same shape (a typeahead list replaced by innerHTML) keeps
  the viewer's nodes: the new page nodes take over the removed nodes' ids and
  only `attr`/`text`/`res` differences travel. A `form` op carries only the
  properties that changed (489). Masks, form fields, frames,
  styles, opaque and custom elements are never morphed, and a morphed node
  counts as a changed input target.
- **Author CSS and real attributes.** Stylesheets are the page's own CSSOM
  text (imports inlined; cross-origin sheets read through CDP and normalized
  by a constructed sheet); adoptedStyleSheets and open shadow roots are kept.
  HTML attributes ship by denylist and only if rendered/exposed by the UA or
  referenced by the page's CSS selectors/attr(); links carry `href="#"`.
- **Pushed credits.** `mirror_next {wait_ms <= 2000}` is a long poll answered
  by the next page change; the client keeps four credits outstanding, packets
  apply strictly in sequence (`base_sequence`), and a gap asks for a reset
  (`after_sequence: 0`). Replies arrive in credit order, so a gap with no older
  credit outstanding (for example a credit replayed after a reconnect, which the
  kernel runs again) resets at once; the reset credit does not wait for credits
  in flight, and the kernel ends the long poll of every credit that arrived
  before it. A queued credit's wait starts when its predecessor is answered
  (one heartbeat per wait instead of one per credit), at most three waits from
  its arrival (below the client's 8 s request timeout).
- **Compact wire (489).** A reset packet carries the binding and header
  (`subscription_id`, `tab_id`, `generation`, `document_id`, `css_width`,
  `css_height`, `device_scale_factor`, `scroll`, `focused`, `selection`). A
  delta carries `wire`, `sequence`, its ops, and only the header fields that
  differ from its base (an explicit `null` replaces); its base is
  `sequence - 1`, and the viewer restores the rest from its binding and the
  base. Empty `resources`/`tiles` are omitted. The controller encodes the packet
  (the Rust kernel passes it through): a body from 512 bytes travels as
  `{encoding: "deflate", packet_bytes, packet_base64}`, raw deflate in the
  subscription's context (one sync flush per packet; a reset packet starts a
  fresh context), so repeated structure such as typeahead rows costs a few
  bytes. The viewer inflates such bodies strictly in sequence order; a body it
  cannot inflate asks for a reset. Smaller bodies (echoes) travel as JSON outside
  the context. A credit replayed after a reconnect gets the same encoded bytes.
  `mirror_input` answers `{"accepted": true}`: the kernel reconciles its actor
  ledger with the full browser state, and the viewer sees the input's effect in
  the next packet.
- **Resources** are only bytes the page itself loaded (data: URLs, the resource
  tree, or Chrome's cache without credentials for Resource Timing URLs), typed
  by magic bytes; fonts ride with the snapshot, images follow it. SVG renders as
  a `data:` image; the sandbox CSP is `img-src blob: data:; font-src blob:`.
  Element images (`<img>` in the top document) travel only within one viewport
  of the view, nearest first; the rest wait until the view comes near them.
  While the view moves (a viewer `scroll_to` or a page scroll, and 300 ms
  after), a near image above 8 KB travels as a preview: decoded in the
  observer's world from the page's bytes, half its CSS size, WebP, wrapped in an
  SVG of the original pixel size so the layout does not change. Its exact bytes
  replace it under the same key once the view settles (settled pixels stay
  exact). CSS images and frame resources keep the earlier order. A packet's
  resource budget is 256 KB of base64, or 64 KB within a second of viewer input,
  so an echo waits behind at most one such packet. A resource larger than the
  room left travels in slices (489): `offset`/`total` in base64 characters
  (4-aligned), in order, under the whole resource's `resource_id`; the viewer
  verifies the digest of the reassembled bytes. A reset restarts a resource
  still in slices (its slices may have been lost with the base).
- **Frames.** Same-origin frames are part of the document. Cross-origin frames
  are mirrored through their own CDP session/isolated world with the same
  observer (ids and resource keys rebased per frame slot; stylesheets its CSSOM
  cannot read are read through the child's CDP session); nested or
  unattachable frames, canvas/video/plugins are opaque regions painted from
  masked lossless captures (at most 1 Hz until video regions land).
- **Input** is node-addressed with the viewer's offset inside the node; the
  kernel clamps to the live box, hit-tests at dispatch (through frames/shadow),
  refuses protected ancestry and targets whose attributes changed after the
  viewer's applied sequence. Epochs are snapshot sequences, not wall clocks.
  CDP pointer coordinates follow the emulated view scale (DPR1 on a scale-2
  window). Text/keys go to the live focus behind the shared text fence.
- **Scroll** is shared (owner decision): the kernel tab has one position. A
  viewer scrolls its own copy natively for frame rate and sends coalesced
  `scroll_to`; the kernel applies it and every other viewer follows the
  kernel's position (last writer wins). A viewer ignores kernel echoes only
  while it is scrolling itself, and the kernel does not send a viewer its own
  scroll position back. Wheel over opaque regions drives the kernel.
- **No tree hash.** Correctness comes from sequenced deltas, base checks,
  resnapshot on a gap and independent client validation.
- **Fallback.** Over-budget or unavailable DOM returns a labelled `fallback`
  packet; the web client shows protected video and retries the mirror after
  30 s, doubling to 10 min. A retired subscription (`stale or foreign mirror`)
  or a credit failure streak longer than 15 s ends the mirror the same way;
  shorter streaks back off (250 ms to 4 s). A packet that fails to apply asks
  for a reset; later packets still apply.

MP-11 rules for v2 (owner decision, Miguel 2026-10-09 11:30 UTC, replacing the
earlier variant scanning): only the fields the kernel filled from the Vault are
masked, by node identity (top-level `backend:N` and child-frame
`frame:<id>:<loader>:backend:N` targets), and only while they are plain text
fields (text-like input, textarea, contenteditable); the type is checked on
every capture, so a show-password toggle becomes a mask. A password field
renders as dots of the same length and its value never leaves, nor does a
hidden input's. No page-text, attribute or CSS value scanning and no
container, frame or media masks: cross-origin frames are mirrored. Fonts and
images ship; every url() spelling is a kernel resource key or `none`; the
client validates every packet independently (unknown record keys, forbidden
tags/attributes, any fetching CSS refused).

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

MP-08/MP-11: input admits the latest eight issued sequences, each at most two
seconds old, on the same caller/subscription/tab/generation/document. Both bounds
apply. Navigation, policy change, close and expiry clear admission. Old targets
are checked against their observed and current identities, current protection,
selection text/offsets and pointer geometry; the isolated world checks connected
nodes, live geometry/attributes and existing focus/hit guards before dispatch.
MP-11: a plain key's first dispatch requires the live active leaf and its
ancestors to match the painted focus, including current protection, attributes
and geometry. This check runs after text-key preflight. The paired keyUp keeps
document/epoch checks without rechecking focus moved by keyDown (such as Tab).
Coordinate-wrapped keyboard input follows validated native focus. Plain keys
refuse when display fallback has no painted DOM focus.
The exact `MP-11: stale mirror input epoch` marker is reserved for sequence-only
refusal before dispatch. Document/policy/target/later-fence failures never carry
that retry marker. The existing command deduplication remains authoritative.

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
content. Within one read, plain leaf paragraphs may share an exact CSS map only
when parent, tag, bounded raw attributes, box size and complete selector-match
set agree. Uninspectable/grouped/nested/pseudo stylesheets, animations, shadow
and nested documents disable sharing. Vault values disable it too. No style
cache survives a read, including for offscreen blocks. Private CDP CSS palettes
and per-read canonical sorting caches reduce repeated bytes/work; resource-bearing
styles retain separate mutable remapping. Initial offscreen flow blocks retain
geometry and hydrate descendants on the following credit, so visible content
paints first. Public packets retain their original shapes/hash bytes. The kernel
and shared client each admit only one outstanding credit per subscription. No
public push events were added; the reserved439/83 pair is not consumed.

## MP-11 protection boundaries

Page scripts, event attributes, origin addresses, navigation/form destinations,
active markup, custom CSS properties and executable/network CSS are absent from
the wire. Computed style is the CSS representation; source stylesheets never run
in a client. The reference iframe permits same-origin parent DOM access but no
scripts, and enforces CSP with no origin connections, forms or active content.
Only kernel-created image/font Blob URLs are permitted. The client independently
validates tree shape, CSS, attributes, bounds and resource hashes before rendering.

MP-08/MP-11: Miguel's 2026-10-09 visual policy supplies only exact Vault-filled
plain-field regions, with no descendants or values in those placeholders.
Password controls retain their dot rendering; their raw values are omitted.
The collector rechecks type, value and frame/document identity on every capture.
Cleared/replaced, removed and navigated targets retire. There is no page-text,
container/order/bidi/budget, iframe or media masking. Registering a value alone
leaves the whole viewport visible, including image/font resources. Unknown
policy refuses packets.

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
All tiles and masks paint in the root mirror document. Tile crops round outward
to CSS-pixel boundaries, keeping DPR2 image origins exact and avoiding child-frame
compositor resampling. Layout boxes remain local and fractional.
Trusted CDP frame origins place exact Vault fill boxes in the composited page,
including cross-origin targets. Closed-shadow and foreign-frame tiles use source
readback with those same field masks; they are not whole-region placeholders.
Nested and shadow images are decoded before apply completes.

## MP-08 input and client integration

Element actions resolve through the current isolated-world node map, check the
live document, visibility/occlusion and protection, and then enter ordinary
kernel browser input. Focus is checked again after page focus handlers, including
redirected focus. Click, text, scroll, key, selection and IME composition are
supported, with ordinary coordinate input as a fallback. Sanitized
contenteditable state preserves rich editors; opaque tile wheels use root
coordinate input, including full-frame fallback. Nested document listeners bind
once per Document identity independently of CSP meta reconciliation. Source selection drives
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
attributes/styles when only children change, patches scalar paint properties
without resetting layout, restores scrolling after the DOM batch, retains identical
PNG overlays and decoded images, and creates no empty font/pseudo
stylesheets. These prevent redundant layout invalidation during SPA batches. It checks
geometry before each next call and requests sticky per-region video fallback for
layout, text or color drift at a 0.5 CSS px geometry threshold. Full-root drift
uses a stable inert full-video document. Tiles occupy integer CSS raster
positions independently of fractional control boxes. Mirror fallback currently
negotiates the mandatory PNG path. Motion uses protected crop PNGs; the next idle
credit does a protected full native readback and exact PNG refinement, avoiding
DPR2 crop-edge differences. It shares `losslessRegion` with display refinement;
there is no second encoder. No VP9-motion throughput claim is made for this path. Protected placeholders
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
It records initial and settled-after-mutation native raster pairs, per-visible
non-DOM-region settled MSE (must be zero, including masked placeholders), raw MSE/mismatch and source-only one-native-pixel
edge-band mismatch. Client-only holes cannot enlarge the exclusion band. Box and
each text line fragment geometry, exact text and computed colors are measured.
The gate is <=0.5 CSS px geometry, exact text/colors and <=0.10% raster mismatch
outside that band. It also records bootstrap/hydration/patch bytes, per-stage CDP,
serialization, decode and renderer spans, ten-sample input/presentation and
mutation/paint distributions, owned-process CPU tick samples,
resource floors and cleanup. An extra idle refinement credit precedes settled MSE measurement and is excluded
from motion latency; mutation latency includes the source CDP call and
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

Round 2/3 fix the known fractional tiles, mask edges, native controls and
same-origin frame seams. Component fidelity/settled MSE can pass while latency remains RED;
retain each failing receipt and exact execution identity. Neither one fixture
matrix nor source checks close an MP item. Keep the feature opt-in until runtime
raster drift/fallback and production clients satisfy the complete gate. The
existing display plan reports another source and relay topology; do not relabel
its benchmark as this component run. Production Cloud/relay/WAN parity, OS-level
IME, animated media performance, Room rollout, macOS replay and current security
anchor semantic review remain acceptance work. MP-11 does not require exact-blob
review of unrelated non-security source.

## MP-08/MP-11 round3 renderer sync

The coordinator's exact WeakSet patch hunks are applied. Its published
`d2ca081586619123df576a5f634d8ce85550a9e8ec46aebd69e498d8c6ba5c93`
hash refers to round1 renderer bytes plus that patch; round2 already changed the
renderer. Cloud must import the current complete file and repin its actual
SHA-256. The evidence manifest records both base and final renderer hashes; no
claim of byte identity with the older Cloud pin is made.

## MP-08/MP-10/MP-11 round4 renderer review fixes

Local focus and input progress are fenced by the observed source document and
sequence. While local input is pending, and through the one credit that can
already be captured before its acknowledgement, a delayed packet cannot restore
older source focus or selection over the user's newer target. The following
credit may restore authoritative focus, including keyboard focus changes. Reset
(including protection reset), navigation, removed/replaced or masked targets
discard local intent. Tracking runs after asynchronous hash/resource validation
so input during that await is included. The one-outstanding-credit contract and
kernel epoch/actor admission remain unchanged; no new wire fields or minima.

Listener removers belong to a per-Document binding map. Reconciliation releases
retired nested Documents; resets release previous nested bindings before rebuilding
and close releases every binding. Surviving Documents still bind once, independent
of CSP metadata. Retired trees are absent from both bindings and the node map.

Run the existing external-tools drill with
`CHARIOX_MIRROR_DRILL_RENDERER_REVIEW=all` for the focused real-Chromium checks:
400ms delayed pre-click credit, immediate B typing and typing after apply (A stays
unchanged), input during hash validation, settled source focus, reset/navigation
fences and twelve iframe replacement/removal cycles including resets before close.
`focus` or `documents` selects either regression for fail-first evidence. This is
a component/loopback fixture, not production Cloud/relay or MP acceptance. Cloud
must copy the complete shared renderer and repin its new SHA-256; public renderer
API, packet shapes and all other shared renderer source files are unchanged.

## MP-08/MP-10/MP-11 round5 input review fixes

Clicks on DOM form fields and editable descendants forward the actual root
coordinates, including nested frame borders and padding. Native source input
therefore places the caret at the clicked character boundary before queued text;
an element-center click no longer overwrites that boundary. Ordinary element
clicks and selections retain their existing actions.

Coordinate input binds to one unambiguous observed leaf at the point. The live
isolated-world hit must match that identity, and its element/frame ancestors must
retain their observed geometry and attributes. Current protected ancestors are
checked across shadows and nested frames. Overlapping ambiguous leaves, moved
targets/ancestors, newly protected nodes and changed hits refuse before physical
dispatch without the sequence-only retry marker. The input path awaits this
live guard after focus capture and the final document wait. Mouse release stays
paired with press when a page reacts to the press by changing layout.

Full-frame observer/unsupported-composition fallback has an inert plaintext
editable tile beneath its protected raster. Printable `beforeinput` and IME
commit use the existing coordinate-wrapped display text input, including its
secret-field refusal and document/epoch/actor fences. Intermediate IME text stays
local; no source state or page script is copied into the input bridge. The tile's
local text/caret is transparent and never replaces source pixels. This fixes
typing while retaining successful protected full-frame fallback packets.

Run the external-tools drill with `CHARIOX_MIRROR_DRILL_RENDERER_REVIEW=input`
for clicked-caret, full-fallback typing/IME/protection and stale-coordinate live
checks; `caret`, `fallback` and `coordinate` isolate a seam for fail-first runs.
The existing `all` mode retains delayed-focus and Document-lifetime regressions.
No serialized action, packet, protocol minimum or shared authority changes;
local433/relay79 remain current, and the reserved439/83 push allocation is unused.
Component typing and synthetic IME commits do not establish OS-level IME,
Cloud/relay, Room, macOS or managed acceptance. Cloud must repin the complete
current renderer after review.

MP-08/MP-11: the later Cloud review's Tab-then-type P1 is included. After a
forwarded native key, printable text/IME commits resolve the actual native
focused editor at dispatch rather than refocusing the old viewer node. That
focus must be a connected, observed, unchanged and unprotected field/editor;
its ancestors and live geometry are rechecked without moving its caret. Native
focus can cross an observed same-origin frame or open shadow. Unknown/new or
protected focus refuses before dispatch. The renderer retains this native-focus
mode through pending input and the credit that can predate its acknowledgement;
a later observation restores ordinary element-bound text. Explicit clicks,
reset/protection/document changes and close discard it. Tab/Enter/Escape also
discard old local focus intent so it cannot suppress authoritative focus changes.
`tab` isolates normal-polling and400ms delayed-response regressions, including an
Enter handler that moves source focus. No new request or reply fields are used.
Native control tiles size their sampled outer boxes with `border-box`, including
controls whose source uses `content-box` with padding/borders. This prevents
false geometric drift from retiring ordinary nested native fields. The `controls`
mode isolates that fail-first geometry check; the complete input mode also
tests post-key focus inside a frame and protected post-key refusal without
rotating a healthy browser generation.


## MP-08/MP-10/MP-11 round6 native keyboard review fix

While native focus progress is pending, subsequent navigation/editing keys use
the same coordinate-wrapped native-focus bridge as printable text. The first
key retains ordinary painted-focus admission. A recent admitted epoch may
contain both fields while a newer focus observation travels to the viewer;
subsequent keys resolve the current native focus immediately before dispatch,
without refocusing the painted field or waiting for another credit. The target
and its element/frame ancestors must remain observed, connected, unchanged,
unprotected and valid against live geometry/attributes. Keys permit observed
noneditable focus (for example a button reached with Tab); text still requires
an editable field/editor. The native key release remains paired when keyDown
changes focus. Document/generation/policy/actor/cancellation/8-sequence/2-second
fences remain in effect, including for release. Unknown/protected/changed focus
is a real refusal without the sequence-only retry marker. No ambiguous key is
replayed. Click/reset/navigation/close retain the native-mode clearing contract.

The shared host input adapter supplies Chromium virtual key codes for all its
existing supported navigation/editing keys, so caret/deletion effects accompany
successful admission. The `native-keys` real-browser drill uses normal25ms
polling with a400ms delayed post-Tab B-focus packet and verifies Backspace,
ArrowLeft, second Tab plus immediate Enter, subsequent text, and newly protected
focus refusal before keyDown. The complete `input` mode includes these cases
and nested-frame key/caret input, alongside all earlier input regressions.

The existing serialized coordinate/key actions are reused; no public shape,
protocol minimum or push event is added (local433/relay79 retained). Cloud must
copy the full renderer and repin matching kernel observer/service/input assets.
Component evidence does not close ordinary/managed behavioural, hosted relay,
Room/macOS/physical IME or current security-anchor review acceptance.
