# MP-08/MP-10/MP-11: kernel browser DOM mirroring

MP-08/MP-10/MP-11: the main-based round-2 successor uses local443 / relay86.
See `MULTIDOMAIN_ROUND2_ON_MAIN.md` for the exact imported source heads and
validation limits; earlier allocations below describe their branch history.

The user-domain browser now has a DOM transport and a shared reference renderer.
Kernel browser pages and App Tabs use the shared DOM mirror; unsupported page regions use protected video.
This implementation targets host Chromium on Linux. It does not introduce a
browser inside a slice, a Cloud runtime proxy, or a second input authority.

## Protocol 503: DOM mirror v2 (current; introduced in 482, compact wire in 489, control/UA state in 492, resend/custom validity/sprites in 501, per-document fragments in 503)

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
  `form`, `scroll`, `size`, `res`, `state` (492). Records travel as compact rows
  `[idDelta, parentBack, tag | kindCode, attrs | 0, extra]`; a stylesheet text
  of 2 KB or more travels once per snapshot epoch (`sheets` + `css_ref`).
  MP-08/MP-10/MP-11 M2 keeps ordinary small documents atomic. Large documents
  start with a pre-order prefix; later credits extend it with these same
  `children` operations. Each continuation visits at most 4096 nodes, targets
  an 8 ms task after a minimum 256 visits, and stops near 128 KiB of record
  work. The initial prefix completes its fixed 1024-visit allowance (subject to
  the byte bound), so a busy first task cannot cut a detailed header before
  its visible viewport. Document limits remain
  finite (900,000 records, 64 MiB of record work, local ids below 1,000,000);
  one record is bounded at 1 MiB. Local node ids restart on a full snapshot
  only after the producer advances its input/region epoch fence. Old scroll
  suppression and frame slots are retired before rebuilding. Cancellation
  keeps the fence ahead of the published sequence, forcing the next credit
  to rebuild a full base. Resource keys retain their document lifetime.
  An ordinary document has a 20,000-record/8 MiB
  atomic backstop, including large open shadow trees. Input echoes precede
  continuation records. A same-document reset preserves an active root wheel
  target and submits that position through normal input under the newly applied
  sequence; navigation discards it. Node-targeted gestures are never replayed,
  and kernel epoch, identity and protection checks are unchanged.
  A structural mutation or changed protection kind
  restarts a bounded current prefix instead of resuming a stale traversal.
  Closed roots are discovered with bounded `DOM.describeNode` batches on
  serialized host identities, including open roots and same-origin frames;
  no whole-document `DOMSnapshot` reply precedes first paint. Unpublished
  hosts become opaque before their packet leaves, with their light descendants
  removed. Published hosts retain ordinary replacement/identity fences.
  These are kernel implementation bounds; mirror v2's wire is unchanged.
  A loading document waits for its first contentful paint before its initial
  prefix. A document that grows past the small-document limit restarts with
  bounded hydration. CSS event initialization runs asynchronously for loading
  or large documents: Chromium's `CSS.enable` waits for resource bodies. Its
  own 30 s bound leaves ordinary CDP request deadlines unchanged. Stylesheet
  owners are reconciled when activation completes so edits during startup are
  retained; attached frames share the same watcher and discovery rules.
  Completed response metadata is indexed by session, frame, loader and observed
  URL (at most 4096 pending and completed entries each). CSS and font reads use
  `Network.getResponseBody` for those exact completed responses before the
  existing resource-tree/content fallback. This avoids waiting for unrelated
  resource bodies before painting the viewport, preserves authenticated bytes,
  and makes no new request or credential copy. Reconnection and detached sessions
  retire the index; document and protection fences still gate publication.
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
  CSSOM edits (insertRule, rule.style writes, replace/replaceSync) make no
  mutation record: the controller enables the CDP CSS domain per mirrored
  session and a `styleSheetChanged` marks the owner node (constructed sheets:
  the adopted lists; imported sheets: sheets with imports) in every observer
  instance of that frame's world; there is no periodic text compare and no
  rule-count cap. Events of a same-origin child document (observed by an
  ancestor's world, no world of its own) are marked in every world of its CDP
  session. A per-drain signature (rule count, `disabled`, the CSSOM media list)
  covers what Chrome does not report (`disabled` arrives as a removal,
  media-list edits not at all); style/link sheets carry their effective CSSOM
  media list. Imports are found after leading `@layer` statements. Attributes
  ship by denylist (492): every attribute except
  `src`-like/navigation names, `on*` handlers and credential-like names
  (`csrf`, `xsrf`, `token`, `nonce`, `secret`, `passw`) outside the rendered
  set; values outside the rendered set and `aria-*` are capped at 1 KB. Links
  carry `href="#"`; `:target` is rewritten to `[chariox-target]`, which the
  viewer sets, per document, on the element that document's fragment names
  (the top document's `fragment` header; a mirrored frame document's
  `fragment` record field and `fragment` op (503), from its own location for
  same-origin and cross-origin frames alike), and `:defined` to
  `:not([chariox-undefined])`, which the viewer sets on custom elements the
  kernel reports undefined (no viewer registry definition outlives its source
  document). Both markers are denied page attributes.
- **Pushed credits.** `mirror_next {wait_ms <= 2000}` is a long poll answered
  by the next page change; the client keeps four credits outstanding, packets
  apply strictly in sequence (`base_sequence`). The kernel keeps its last 16
  packets (at most 8 MB) with their original bytes. A gap (a reply lost with a
  dropped socket, or one overtaken by milliseconds) sends one
  `mirror_next {resend: true}` at once (501): the kernel ends earlier long polls
  and answers with the packet after the credit's base from that history
  (also once an ended long poll issued it), else as an ordinary credit; a
  duplicate is ignored, and a gap that outlives its resend asks for a reset
  (`after_sequence: 0`). A failed credit (a dropped socket, a relay
  re-authentication that rejects pending KernelBrowser requests) also asks for
  a resend instead of a full snapshot; a reply that could not be applied asks
  for a reset (credits lost with one socket count as one failure for the
  backoff). Without a resend, a packet that stays unacknowledged for 1.5 s
  while a later credit still names an older base is sent again in order. A
  credit the transport replays after a reconnect (same command id) gets its
  original packet, and a still-waiting original stops its long poll. The reset
  and resend credits do not wait for credits in flight, and the kernel ends
  the long poll of every credit that arrived before them. A queued credit's wait starts when its predecessor is answered
  (one heartbeat per wait instead of one per credit), at most three waits from
  its arrival (below the client's 8 s request timeout).
- **Compact wire (489).** A reset packet carries the binding and header
  (`subscription_id`, `tab_id`, `generation`, `document_id`, `css_width`,
  `css_height`, `device_scale_factor`, `scroll`, `focused`, `selection`,
  `fragment` (492)). A
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
  An external `<use href="file.svg#icon">` (501) carries its fragment as `href`
  and the file as a `sprite` resource: the page's bytes parsed inert in the
  observer's world and rebuilt (shapes, symbols, paint servers, sanitized
  `<style>`; href and url() only to fragments; no script, foreign content,
  image, link or animation). `<use>` does not follow `data:` URLs, so the
  viewer validates the file again and uses it as a same-origin blob; a refused
  file renders nothing.
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
  cannot read are read through the child's CDP session). Every field of a
  child record, op and delta is rebased by a declared schema; an unnamed field
  is refused. Nested or unattachable frames, canvas/video/plugins are opaque
  regions, stills at most 1 Hz until video regions land: a canvas bitmap or a
  video frame travels as the element's own pixels (overlays above it stay
  mirrored DOM); tainted media, plugins, closed roots and unattached frames are
  masked screenshot crops. A region under any transform beyond a translation
  (also size-preserving ones such as `rotate(180deg)`, `scaleX(-1)`, the
  `rotate`/`scale` properties), a filter, or opacity/blending on it or an
  ancestor, and a region over most of the view, never hand the page to video:
  the kernel sends no still for it and its region video lease (a viewport crop
  that already holds the compositing, clipped to the viewport) is shown by the
  viewer as a patch at the region's on-screen rect, outside the transformed
  element so the transform and opacity are not applied twice. Only a failing
  document copy (snapshot/observer failure) selects whole-page video. Region and
  mask records carry their layout size; sizes and a mask's display are
  reconciled every drain.
- **UA state (492).** State without an attribute footprint is sampled from a
  declared set (elements with `popover`, dialogs, checkboxes, custom elements)
  on toggle/close/input/change, attribute changes, morphs and the 2 s cursor:
  `state {popover_open, modal, indeterminate, defined, custom_invalid}`
  (records carry non-default values; `custom_invalid` (501) is a control's
  `setCustomValidity` without its message, sampled again after the page's own
  input/change handlers). The viewer shows/hides popovers (its copies are
  `manual`: no viewer light dismiss), opens modal dialogs in its top layer,
  sets `indeterminate`, marks undefined custom elements (`chariox-undefined`)
  and sets a custom validity on its copy so `:invalid` matches.
- **Input** is node-addressed with the viewer's offset inside the node; the
  kernel clamps to the live box, hit-tests at dispatch (through frames/shadow),
  refuses protected ancestry and targets whose attributes changed after the
  viewer's applied sequence. Epochs are snapshot sequences, not wall clocks.
  CDP pointer coordinates follow the emulated view scale (DPR1 on a scale-2
  window). Text/keys go to the live focus behind the shared text fence.
  Focus transitions travel explicitly (492): viewer `focusin` sends `focus`,
  focus leaving to nothing while the mirror keeps focus sends `blur` (the
  kernel blurs only while that control is still its live focus), and a kernel
  header `focused: null` blurs the viewer's control. The caret (a collapsed
  selection) travels inside editable content only. A press's focus/selection
  waits up to 50 ms for the rest of its click, so a menu the page renders a
  frame later shares the packet; after an acknowledged viewer focus, packets
  still in flight that name the old focus leave the viewer's focus alone. Selects and date/time/
  month/week/color/range inputs keep the viewer's own UI (popup, picker,
  typeahead, arrows; only Tab/Enter go to the kernel); their committed value
  travels as `set {node_id, value | checked | selected_index}`, which the kernel
  validates live (protected/sensitive/disabled/epoch) and writes with
  input + change events; the viewer listens for committed changes on every
  mirrored document and shadow root (`change` is not composed), and compares a
  multiple select's option with the state it last sent until the kernel echoes
  it. Keystroke forwarding remains for text controls.
- **Scroll** is shared (owner decision): the kernel tab has one position. A
  viewer scrolls its own copy locally for frame rate (spaced pixel wheel
  steps ease over the frames until the next step, like a browser's animated
  wheel tick; mouse-wheel notches ease continuously over about the browser's
  own tick duration, so reversals and sub-pixel tails do not stall the
  content; frame-rate steps, zoom and snap scrollers stay native) and sends
  coalesced `scroll_to`; the kernel applies it and every other viewer follows the
  kernel's position (last writer wins). A viewer ignores kernel echoes only
  while it is scrolling itself, and the kernel does not send a viewer its own
  scroll position back. Wheel over opaque regions drives the kernel.
- **Paint holding (MP-08/MP-10, owner A/B 2026-10-14).** Like Chrome, a navigated
  document's first snapshot waits for its first contentful paint (or `load`),
  at most 2.5 s, so viewers keep showing the previous page instead of a white,
  half-parsed one, also when the same credit resets the stream (a resynchronizing
  viewer). The viewer keeps its subscription across documents. A bounded prefix
  may still contain only head nodes after the source has painted (large inline
  stylesheets, including GitHub login). The renderer prepares that document in
  a second script-free iframe and retires the prior frame only after continued
  visible content is available. A fresh attachment keeps its labelled loading
  state until then. Preparation work is bounded and stops after the first paint.
  MP-11: the held frame is inert; event forwarding also checks that the event's
  document belongs to the current painted iframe, because native keyboard focus
  can remain in an inert iframe. Same-document protection resets apply at once;
  they never retain the old field. Capture geometry is unavailable during a
  pending document replacement. Repeated navigation retains at most one prior
  frame and one replacement; failure and disposal retire the unused frame.
- **No tree hash.** Correctness comes from sequenced deltas, base checks,
  resnapshot on a gap and independent client validation.
- **Observer bounds.** Instances are least recently used first, at most 16 per
  world (twice the service's subscriptions); a new instance for a running
  subscription answers `resync: evicted`. Resource keys are monotonic and the
  URL map is LRU at 4096, so long sessions keep materializing images; the
  kernel drops resource bytes once the viewer acknowledged them.
- **Stated bounds (unsupported).** JS-only styling without a sheet or attribute
  (`element.animate`/WAAPI, `FontFace`/`document.fonts.add`,
  `CSS.registerProperty`, `CSS.highlights`); CSS animations restart at the
  snapshot; ElementInternals custom states (`:state()`) and form-associated
  custom element validity (`ElementInternals.setValidity`); closed shadow roots on
  non-custom hosts (open ones attached late are found within 2 s); select popups
  and pickers use the viewer's OS UI. Font and
  scrollbar parity assumes the viewer resolves the same fonts and scrollbar
  width as the kernel host (Linux Chrome); Mac/Windows viewers may wrap
  differently.
- **Fallback.** Over-budget or unavailable DOM returns a labelled `fallback`
  packet; the web client shows protected video and retries the mirror after
  30 s, doubling to 10 min. A retired subscription (`stale or foreign mirror`)
  or a credit failure streak longer than 15 s ends the mirror the same way;
  shorter streaks back off (250 ms to 4 s). A packet that fails to apply asks
  for a reset; later packets still apply.

MP-11 rules for v2 (owner decision, Miguel 2026-10-09 11:30 UTC, replacing the
earlier variant scanning): only the fields the kernel filled from the Vault are
masked, by node identity (top-level `backend:N` and child-frame
`frame:<id>:<loader>:backend:N` targets), and only while they are not password
fields (any other input type, textarea, contenteditable: the pixel path's rule);
the type is checked on every capture, so a show-password toggle becomes a mask.
A target is masked only while it holds the fill (coordinator decision
2026-10-11, consistent with #950): the kernel registers it with the secret's
value hash and fill revision before the insertion, so it is masked from then
until the field first holds the fill, stays masked while it does, and becomes a
plain field for every viewer once its value is no longer the fill (an edit);
a refill (new revision) masks it again. Targets without a hash stay masked
(never under-masking). A password field
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
App Tabs use the same caller-bound mirror subscription and kernel input path as ordinary browser Tabs. App calls, grants and actor attribution remain in the kernel App host.

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

MP-08/MP-10/MP-11, coordinator decision 2026-10-15: native region attestation
proves window/Tab/document identity and the authoritative page origin, not the
decoded media pixels. Native compositing and CDP can rasterize video differently.
Only live video, canvas (including WebGL), object and embed boxes reported by
the kernel's isolated DOM observer are excluded from the pixel comparison.
Their geometry and issued node identity are checked before and after capture;
the viewer cannot supply exclusions. Every non-media, unprotected pixel must
match exactly, with no tolerance or media padding. These exclusions never mask
the displayed media or change Vault protection.

When a region has fewer than 10% (and at least 64) usable identity pixels, an
unadvertised readback attests surrounding visible page pixels at the same
native origin, using the same owned PID/XID and current lease. It closes before
the region is advertised. A full-bleed page without enough surrounding anchors
refuses. Wrong windows/Tabs, stale documents or generations, shifted origins,
non-media pixel changes and unreported media still refuse. Admission, delivery
and input retain the native-window, active-Tab, geometry and protection fences.

MP-11 attestation review amendment (2026-10-15 09:41): a DOM tag alone
never supplies an exclusion. The kernel resolves that exact node in its isolated
world and requires its backend node in the same CDP session's current content
layer tree, with the document loader unchanged before and after the read. OOPIF
media needs that child's own observer/session; an unattached frame remains in
the exact comparison. Missing or oversized layer evidence supplies no exclusion.
Static layer evidence waits for the observed document's own animation-frame
commit, with a bounded timeout. It does not take a screenshot that could resize
the native surface merely to request layer metadata.

Exclusions are clipped to the attested visible page, never native Chrome, and
cover at most 75% of that page. Each box needs an exact three-device-pixel ring
on its visible surrounding page edges; another exclusion or a protection mask
cannot cover the ring. Full-bleed or insufficient proof still refuses. Existing
kernel protection wins: its regions mask both comparison rasters before compare,
and any overlap between a media exclusion and a protected region refuses.
Proof-pixel counts exclude all masks. This preserves the owner's existing
Vault-fill/password-dot policy rather than introducing generic page masks.
The private immutable attestation receipt and `native_region_attestation`
diagnostic contain area/count fields only (excluded/page/proof/protected/ring
pixels and media box count), never pixels, content, identifiers or secret values.

MP-08/MP-10/MP-11 (r41): the exact-RGB attestation of a whole-page native video
display exists so Vault masks line up on the stream. It is required only while a
kernel-known protected target exists (a policy target or a recorded Vault fill,
any tab or frame). Without one the owned window is admitted by identity and
sample geometry (live owned window, active Tab, expected sample size), so a
continuously animating page (a full-viewport canvas or video) gets native video
at once. A source admitted that way retires before its next publication when a
protected target appears; its next start attests exactly. If that exact
attestation is refused, the subscription is served from the kernel's CDP display
(protected PNG captures, no native window) instead of ending in a viewer error;
the next document subscribes to native again. Native frames reach only display
subscribers; the model's observations remain protected CDP captures.

MP-08/MP-10/MP-11: a mirror's retained input-capture lease emulates page focus
to keep its renderer awake. Native Chrome text admission must not confuse that
logical state with the page physically owning the keys. After a kernel-observed
toolbar click or Ctrl+L, the shared input owner orders a short physical focus
read with emulation suspended. When Chrome physically owns the keys, the page
stays physically blurred across later characters and screenshots; repeatedly
restoring logical focus would synthesize page focus/repaint events while the
person types in the omnibox. Ordinary page input resumes the retained renderer
lease on the same ordered owner. A refused/failed focus read restores its prior
state, and closing the display still releases the lease. Same-page
input cannot overlap that read; a closing lease stays closed. The isolated
world's real focus and sensitive-target checks still refuse typing into a
protected field or foreign frame. No native-key path may skip those checks.

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
stale input, drift, App Tab caller isolation and unknown-outcome replay. These complement
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

## MP-08 / MP-10 / MP-11 App Tab v2 acceptance

App Tabs (#992) are ordinary kernel browser Tabs at the installation's isolated
App origin. The renderer carries author CSS, text nodes and live form controls
through the existing v2 snapshot/delta contract. Notes selection uses the same
kernel capture path; its public `observation` result remains distinct from the
controller's private `_observation` ordering field. The client exposes the
packet's CSS viewport and native pixel size to Notes and screenshot registries.

Required real hosted drills, driven through the built web client's `/app open`
flow with an installed signed or unsigned Todo or Docs App:

- `MIRROR-APP-TEXT`: at DPR 1 and 2, App heading/status text is selectable text;
  select it, save a Note, Ask with its quoted text, and resolve it. App page
  content never receives trusted Notes chrome or the comment.
- `MIRROR-APP-ECHO`: at DPR 1 and 2, click the author-styled field and type through
  the normal kernel input path. Measure actual presented glyph changes over
  hosted WSS with an approximately 8 Mbit/s client uplink and at least 60 ms
  RTT; P95 must be at most 150 ms. The caret and source field value agree.

Retain fail-first baseline and final-head screenshots, console/log captures,
per-key timings, exact source/binary identities, shaping calibration and cleanup.
Controller fixtures supplement these drills and do not close their acceptance.

### MP-08 / MP-10 / MP-11: ordinary first-snapshot cap (2026-10-15)

Ordinary documents (at most 10,000 native elements) serialize atomically until
completion or 8 MiB of record data. This keeps a site's header/templates from
replacing the previous page before its form or file list arrives. The cap is
evaluated between records: one record is bounded at 1 MiB, so this is not a
strict 8 MiB serialized-message limit. An open shadow tree also switches to
continuation at 20,000 records. Larger documents retain the existing 1,024-visit
first prefix and bounded, viewer-credited continuation. Browserux 91599e472's
bounded head-CSS exception remains on that large-document path.

Atomic observation uses the existing protocol 532 chunks-v2 transport, whose
16 KiB frames share a 256 KiB acknowledged window and fairly interleave display
events with replies; it adds no unchunked bypass or new wire flag. The client
still stages incomplete documents from the capped/large path and fences input
from the held prior document. Same-document protection resets remain immediate.
This internal snapshot budget changes no serialized shape or client floor.

The initial 2 MiB candidate failed real hosted GitHub login at 1920×1080 DPR2:
later CSS made resets stop at 2,117,536–2,119,117 record bytes with continuation
pending. That record-work cap can be reached before the form despite much
smaller deduplicated/compressed wire data. A later hosted GitHub repository
check also exhausts 4 MiB after 946–955 records, while the complete native page
has 2,346–2,518 elements. Its viewer shows only the header. The 8 MiB cap covers
that larger ordinary-document range; over-cap documents still continue.
Resets for lost base/recovery use the same producer policy as first
attachment/navigation. These observations and source regressions do not close
the strict hosted fidelity, interactivity or stability gates.

### MP-08 / MP-10 / MP-11: explicit initial completion (539/147)

`snapshot_complete` is true only after the initial main-document serializer has
drained and native document readiness is complete. False keeps an invisible
body prefix from starting the two-second intentionally-empty-page timer. A
readiness change wakes the credit; a change between credits remains pending.
Compact headers inherit completion from their base. Both renderer copies require
539 and retain the previous inert page until visible content or confirmed empty
completion. This adds no observation authority or protection exception.

The ordinary-document atomic backstop is 20,000 records / 8 MiB of record data
(up to one bounded record beyond the byte threshold), with 40,000 iterator visits.
It is synchronous work on the native page thread; 532 relay chunk interleaving
does not bound that CPU time. The cap has not increased in539. Near-cap first
packet/continuation input timings still need quiet-host evidence before acceptance
or any further cap increase. B3 functional runs and source tests are not that
timing evidence.

MP-08 / MP-10 / MP-11 completed-empty presentation: the local two-second timer
notifies the View with the last applied frame metadata. Idle or stalled credits
cannot postpone View promotion after that timer. The packet callback still runs
only for received/applied packets; presentation creates no sequence or ACK and
is fenced by the current applied sequence, renderer lifetime and frame identity.

### MP-08 / MP-10 / MP-11: native navigation handover (539/147)

Owner decision 2026-10-16 permits the existing protected whole-page VIDEO path
while a typed URL's DOM snapshot prepares. The opting-in subscription announces
`navigation_preview` before observer work, retires old input epochs/region leases,
and continues delivering the new document's mirror offscreen. Ordinary clients
that do not opt in keep their current paint hold. No screenshot polling path or
new capture/input authority is added.

Required cell `MIRROR-NAVIGATION-HANDOVER`: from a fresh real hosted web client,
use the native omnibox to navigate between public sites, then click/type/scroll
while VIDEO changes to DOM. Run 1440×900 and 1920×1080 at DPR1 and DPR2 through
the shaped hosted WSS path. Record full screen video and per-frame mode/geometry
samples across the switch. Watch the recording and inspect every transition
frame: no blank frame, layout jump, old-page flash, lost or duplicate input.
Verify the final field value and caret through normal keys after handover;
repeat a navigation, an active pointer gesture and IME composition. A mismatched
document/geometry, incomplete snapshot, pending input or active composition must
keep the admitted native surface until a fresh synchronized mirror can replace
it. Capture a region before and after the switch. The base must fail the same
video/handover assertion. Source fixtures supplement this real drill only.

MP-08 / MP-10 / MP-11 startup boundary: the subscription binds its initial
document without starting native preview. A later document transition, including
one committed before the first snapshot drains, may announce navigation preview.
This keeps ordinary attachment on the mirror path and prevents native page
initialization from resizing an already painted band before the person's first
URL input. Existing owned-window rebinding still handles later native starts.

MP-08 / MP-10 / MP-11 — native preview scheduling: after a navigation-preview
notification, the observer waits at most2500ms for this observer/document's
ordinary protected visible-page producer to initialize on the owned desktop.
The wait ends on a valid attested sample, a deadline or cancellation; it grants
no capture/input authority. This gives native startup a chance before a large
DOM drain competes on CDP. Record `mirror2_preview_wait` and native first-paint
timings on the real large-document flow; source scheduling checks are not acceptance.
