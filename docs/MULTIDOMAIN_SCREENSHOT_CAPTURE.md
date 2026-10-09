# Multidomain screenshot capture (425)

A human selects a visible region and receives a PNG in their own prompt draft.
The image uses the existing prompt attachment intake, transfer/materialization,
and provider image input paths. No screenshot agent tool, provider adapter,
relay authority or Cloud runtime proxy is added. Annotation editing is deferred.

## Capture contract and hosts

`CaptureVisibleRegion { capture_id, surface, region }` requires an authenticated
human terminal caller. The kernel derives the owner from that caller. A Room
surface supplies its existing attachment ID, runtime generation and viewport
revision; both session membership and attachment ownership are checked. A
kernel-browser surface supplies a tab ID and host generation; the host selects
only that owner's private browser. A user App surface supplies a view ID and
host generation; the kernel resolves the live owner-bound view to its host tab.
A closed, another-owner, native-only or stale view cannot return kernel pixels.

`region` uses integer coordinates relative to the **painted surface**, excluding
letterbox margins. Viewport width/height describe its client size; frame
width/height must match the decoded host PNG. The kernel maps the selection
with floor/ceil rational arithmetic, rejects offscreen/empty/oversized regions,
and bounds compressed and decoded image memory. Masking precedes cropping.

The request-scoped `VisibleRegionCaptured` event includes the capture ID,
resolved source, capture time, dimensions, PNG media type, SHA-256 and bytes.
Only the requesting human client receives it. It is not broadcast to sessions
or agents and creates no new retained user-domain artifact store. Existing Room
capture artifacts retain their current worker-side protection/revocation rules.
Clients verify the capture ID/source, digest and image dimensions before intake.

Room browser and App tabs use the existing **protected desktop screenshot**
service. They do not substitute a page screenshot. The worker observation
barrier, Vault mask policy, input fence and protected artifact chunk reads
remain in force; a viewport change invalidates the crop. The home/worker relay
path remains encrypted transport, with no relay screenshot inspection.

MP-08/MP-11 user-domain captures use the shared kernel Chromium controller and
Vault input/capture barrier. Miguel's 2026-10-09 fill-target model replaces the
former generic field/frame/shadow and credential-echo masks: only the exact
Vault-filled target is covered when it is currently a plain text field.
Password fields show dots; show-password toggles are checked on every capture.
Frame/backend-node/document/generation identity fences the target. Navigation,
removal and user clearing/replacement retire it. DPR-aware field boxes receive
small padding before cropping; racing measurements drop the capture for retry.
Unknown policy refuses capture. No direct-backend bypass or extra generic mask
layer remains.

## Flagged web prototype

The existing `CHARIOX_USER_APP_VIEWS_PROTOTYPE=1` flag gates the entire capture
flow. A host-owned top-layer overlay observes browser-trusted Ctrl+drag; Esc,
pointer cancellation, owner/kernel/prompt target changes cancel it. Selection
and pointer capture are outside the App iframe. App messages and synthetic
keyboard events cannot begin capture or supply its region. A host activation
button also starts selection: Ctrl initially pressed inside a cross-origin App
iframe cannot be observed by its parent. No App key forwarding is trusted.

Native panels use a client-side DOM render. Native Apps render the visible
iframe viewport over a dedicated parent-bound MessagePort; the App channel API
has no capture method, the signed UI never receives the selected region or
capture port, and the parent/window/origin/view binding is checked. The host
then crops/composes that frame. MP-08/MP-11: the earlier prototype
password/passkey/opaque-block masking described here is superseded by Miguel’s
2026-10-09 fill-target model. Kernel capture covers only recorded Vault-filled
plain fields; password dots and other content remain visible. The client does not crawl cross-origin documents or capture offscreen
content. Native DOM rendering is a prototype: canvas/image content is supported;
unsupported/tainted resources fail rather than invoke a page screenshot service.
DOM rasterization is not a browser compositor API, so fonts, pseudo-elements,
backdrop filters and complex native controls can differ from displayed pixels.
A browser-mediated trusted capture API would be needed to remove that limitation.

A visible Room/kernel frame is cropped by the kernel after viewport mapping;
visible native DOM is rendered locally. Regions spanning both composite into
one PNG. Host DOM occlusion prevents a hidden kernel surface from being painted
over another dialog. The registered frame/input presentation is temporary and
transport-neutral; the App native default remains unchanged. Its host-switch
button lets the prototype compare native rendering and the same kernel-hosted
App without creating a Room.

The normal attachment controller adds the image/thumbnail to the draft. Its
existing source URL carries the window/Room/view identity, time and per-kernel
capture digest; the filename identifies the source/time too. No prompt wire
shape changes. Image-capable providers still receive the materialized image
through their normal provider-native image input. Capture does not send a prompt
or share the user domain with an agent until the user sends their own draft.

## TUI

Behind the same flag, `/attach region room <x> <y> <width> <height>`,
`/attach region tab <id> <x> <y> <width> <height>` and
`/attach region app <view-id> <x> <y> <width> <height>` use desktop pixel
coordinates. The client obtains current host/Room revisions, verifies the PNG,
stores a private temporary file, passes it through existing transferred-file
intake, and deletes the temporary file. It requires the user's existing prompt
attachment/session; it does not create a synthetic Room for a user-domain view.
The resulting `[image N]` token uses normal prompt attachment submission.

A terminal has no general mapping between text cells and a remote desktop video
region, so arbitrary Ctrl+mouse region selection is not advertised. A native
text-only App projection has no pixel surface; use its existing kernel-hosted
view. No annotation editor or new provider permission route is introduced.

## Protocol history and integration

This branch advances local 418 to the coordinator-allocated **425**; relay
protocol remains 70. Existing immutable 417 browser and 418 App-view shape/hash
snapshots remain intact. Current-version guards become 425; new capture shapes
and their hash are snapshotted at 425. Only capture clients require minimum 425;
App views still require 418 and browser control 417. Numbers 419–424 belong to
other coordinator lanes and are not invented here. If merge order changes,
renumber the current local constant, current-version guards, 425 capture fixture/
hash and capture client minima together, retaining historical 417/418 snapshots.

## Owner/coordinator follow-ups

- Native DOM rendering can differ from compositor pixels, especially shadows,
  custom content and backdrop filters. Protected opaque blocks deliberately
  trade fidelity for observation safety. Decide whether a later browser capture
  permission/API is acceptable for exact native pixels.
- The installed public runtime revision 5 lacks the current group-mapping
  handshake. Release acceptance requires a fresh approved signed runtime. The
  live builder drill uses the explicit private-namespace signed test fixture;
  this does not claim release-artifact acceptance or rotate durable keys.

MD-stack integration: the unreleased feature allocation is folded into local
protocol 427 (relay peer 74). This union and its shape/hash guards supersede the
per-feature versions described during development above.
