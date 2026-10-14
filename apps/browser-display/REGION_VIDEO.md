# MP-08 / MP-10 / MP-11 — native region producer

`NativeWindowRegionCapture` in the kernel's
`kernel-browser-window-region.mjs` implements the native source interface used
by `MotionEncoder`, `PortableEncoder`, `NativeRefiner` and `DisplayStream`.
It uses the same `startNativeReadback` adapter, kernel executable, private raster
leases, packet files and decoder recovery as the whole-page source.

The kernel resolves the owned Chromium window and supplies its physical size,
crop, content height, canonical scale, protection policy and a current lease
predicate. The rectangle uses physical window pixels. Its source interface
reports CSS width/height and maps native pointer input through the crop origin.
The private worker refuses an ambiguous window, wrong PID, changed window size
or out-of-window crop. Destroy and configure events wake the ownership fence.
Small raster fingerprints remain inside their rows, including two-pixel crops.

For `page`, the region must stay inside the content area. Vault field rectangles
are translated into crop coordinates before every native encode and exact
repair. Admission compares the crop against the already protected page capture
and retains the document fence. For `browser_chrome`, the band must stay above
the content area; page navigation does not retire the window source. Its `tab`
binding must represent the kernel-issued window lifetime, and `isCurrent`
must invalidate it on lease, policy or owned-window retirement. A Chrome-only
chord adapter uses the private native key control with the existing CDP modifier
bits. Chords cannot bypass page text-target admission.

Region sources require the compiled native worker; they do not start a Python
or PNG polling fallback. Cropped motion should negotiate whole-video encoding
with dependencies; the existing exact repair and ACK pump remain responsible
for recovery. The private crop raster must have even pixel dimensions within
2560×1600. At the canonical scale 2, integral CSS crops satisfy that condition.

This producer does not issue public region subscriptions or authorize input.
The kernel's shared terminal admission, actor/input ownership, Vault barrier,
mirror epoch and relay subscription lifecycle must wrap it. Mirror P3 requires
a current kernel-resolved region ID and rectangle from mirror v2; browser chrome
requires the browser UI lane's owned-window binding and enterprise policy
admission. The public adapter now lives in `kernel-browser-display-regions.mjs` on the
integration line and consumes allocated519/127. It delegates to the concrete
mirror resolver and the ChromeWindows/arbiter owner seams. This source module
does not implement those owner authorities.

MP-10 acceptance requires the real built clients/kernel/hosted relay, public
sites, normal cookies/popups/tabs/typing/scroll, watched DPR1/2 recordings and
strict paired latency/FPS/CPU measurements. Physical source fixtures and a
real-Chromium source drill qualify this producer only; neither closes P3 or
native Chrome UI acceptance. In particular, owned auxiliary windows used by
omnibox suggestions and menus need their explicit consumer binding and proof.

Run the producer qualification with `node apps/browser-display/drill-native-region.mjs
STATE_ROOT EVIDENCE_ROOT` as an ordinary user (the documented root builder may
use its user namespace wrapper). Roots must be new
`/root/.chariox/dev/vidlat/r<N>/region-live-<name>` and evidence under the
corresponding evidence lane. Set `OSS_SOURCE`, `NATIVE_WORKER_SOURCE`,
`CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER` to the exact built executable,
`CHARIOX_BROWSER_DISPLAY_OPENH264` to the installed public codec and the normal
headed browser/display/software flags. The drill hashes the worker itself,
uses `KernelBrowserHost` to launch sandboxed Chromium, and qualifies Wikipedia
page/Chrome crops, native codec, exact output, an omnibox chord and lease
retirement. It exits nonzero for a failed case and closes its owned source
processes before deleting its state. An older worker which refuses private
window-region configuration is the red-capable comparison; record its distinct
worker source. This is producer qualification, not public-client acceptance.

MP-08/MP-10/MP-11: the private `nativeWindowInventory` helper returns only
owned Chrome XIDs, physical dimensions/positions, visibility, focus, redirect
and transient-parent metadata on `OwnedDisplay`. It rechecks its admitted
window-generation predicate after the native query. A region may carry the
kernel-resolved XID `window.id` (not the CDP window ID) to select one exact owned X window when another
window has the same PID/size (for example a separate keepalive). The worker
still verifies the process and exact geometry; a root, foreign or retired ID
cannot widen capture. The old no-ID path still refuses ambiguity.

Inventory is not popup admission. A window without the exact Chrome PID is
excluded; the consumer must prove auxiliary role, parent/lifetime and policy
before selecting it. Unobserved omnibox suggestions remain a failing UI cell.
A native Chrome consumer must also keep the internal keepalive out of the user
strip, enforce navigation/download/password policy and use the shared kernel
input arbiter. These bindings are outside this private producer qualification.

MP-11: native pointer and chord dispatch recheck the exact window dimensions
before injection, including input that arrives before the next readback.
Resize events retire the crop even when the window returns to its old size.
Moving a window without resizing it retains its binding and updates the native
pointer origin. A replacement crop requires a newly resolved kernel lease.

MP-08/MP-10/MP-11 integration preparation: the shared presenter admits a crop
only from a kernel-negotiated `source_kind: 'region'` lease with its opaque
`binding_id`, document, applied mirror `sequence`, canonical CSS dimensions
and scale. Frames must match that geometry and document; frame headers cannot
select a crop. Region input retains the lease and mirror epoch, while ACKs
retain the ordinary display sequence. A target request requires protocol 519
and refuses a whole-page negotiation. The public target adapter consumes519/127 and the concrete mirror resolver.
Native Chrome uses the browserux-owned ChromeWindows and arbiter; their live
integration and strict paired hosted drills remain DISPLAY-P3 acceptance gates.
