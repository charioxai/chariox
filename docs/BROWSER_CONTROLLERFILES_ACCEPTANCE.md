# MP-08/MP-10/MP-11: shared Browser artifacts and observation fixtures

The first-party fixtures change no benchmark solver, evaluator or frozen score.
Frozen G2 `9334141d420f8a32393f206102c5b8b4a1b0b609` fails the visible
**Choose documents** control with `browser_upload_invalid`. That RED evidence
remains attributed to G2. Protocol 420 adds the common artifact runtime described
below; a passing fixture does not close MP-08, MP-10 or MP-11.

## MP-08/MP-10/MP-11 runtime contract

The visible chooser uses the existing Browser action path: resolve the observed
control, arm Chromium file-chooser interception at physical click dispatch, and
require the same session, frame and document before selecting files. Navigation,
missing chooser, unrelated events and cancellation discard the staging reservation.
No alternate permission, prompt, provider or relay authority is introduced.

`slice_browser_upload` accepts exactly one of legacy permitted `files` or
`artifact_ids`. Opaque artifacts must belong to this Room and come from the
common transfer or Browser artifact service. Names, sizes and SHA-256 are
verified before encrypted home/worker transport, private materialization and
the existing upload receipt/recovery path. Provider-profile paths are never
resolved by this new interface.

`slice_browser_artifact` captures an `image`, `network` or completed `download`
from the actual controller browser, using the observed browser generation and
current focused Tab. Downloads also require the observed GUID. The home records
an operational artifact, without archival enqueue or filesystem paths, bound to
Room, Environment, runtime generation, physical browser, Tab, document,
canonical viewport, actual visual viewport geometry and secret observation
revision. Browser bytes are capped at 8 MiB. Reads verify identity and the
stored hash, reject symlinked blobs, and return at most 128 KiB per chunk.
Text inspection accepts at most 256 KiB and scrubs protected values. PDF
inspection uses installed `pdftotext` with a three-second timeout, 256 KiB
output bound and, on Unix, 256 MiB address-space/three-second CPU limits. It
feeds verified bytes over stdin, never a provider path. Missing utility is an
explicit inspection error; download chunk reads remain available.

Image capture uses the same attached CDP Page session, checks document and
viewport before/after capture, and passes actual PNG bytes to the existing MCP
native `image` converter. Protection checks traverse all owned renderer sessions,
including nested isolated cross-origin iframes and local descendants, before and
after the composited screenshot. Frame/document/parent identity changes reject
the capture, and temporary isolated-frame sessions detach on every outcome.
If protected values or a password input are present,
the whole viewport is conservatively blacked out. This is not selective visual
redaction. Downloads are withheld while protected values are registered.

MP-08/MP-10/MP-11 isolated-password regression:
`node --test apps/kernel/slice-linux-docker/docker/browser-controller-image-capture.test.mjs`
checks exact image/mask bytes, owned/foreign renderer boundaries and identity
fences. The real Chromium controllerfiles drill forces site isolation and embeds
a localhost password iframe in its 127.0.0.1 tab without any registered Vault
value. It requires an actual iframe target absent from the top DOM snapshot,
then compares the returned artifact with the full-viewport black PNG byte for
byte. These focused checks do not establish provider/Web/TUI or Path-1 parity.

Passive capture retains actual CDP request/response metadata for the same
Tab/document, including observed extra-info headers. It allowlists only Accept,
Content-Type and Content-Length, removes cookie/auth headers, userinfo, query,
fragment and bodies, and bounds history to 500 entries. `browser-network.har`
is a HAR-shaped metadata subset; it does not reconstruct headers or supply a
complete timing/body archive. Later registered protected values are scrubbed
again when the attachment is captured.

`RoomBrowserArtifact` exposes the same capture/read/inspect service to attached
Web/local/remote TUI clients, with membership and attachment-owner checks. The
shared TypeScript builder declares minimum local protocol 420. Runtime provider
calls use normal authenticated home admission and leased-worker routing. New
peer variants require matching kernels; relay-version coordination remains a
merge responsibility, not a lane-allocated number.

## MP-08/MP-10/MP-11 focused validation

```sh
node --test apps/cli/scripts/lib/browser-controllerfiles-fixture.test.mjs
node --test apps/kernel/slice-linux-docker/docker/browser-controller-artifacts.test.mjs
CHARIOX_TEST_CHROMIUM=/absolute/path/to/chromium \
CHARIOX_CONTROLLERFILES_EVIDENCE=/absolute/external/evidence \
node --test apps/kernel/slice-linux-docker/docker/browser-controllerfiles.browser-test.mjs
```

The HTTP fixture provides a visible chooser backed by a hidden file input,
strict multipart receipts, Unicode/binary/PDF downloads, cancellation, a visual
canvas and synthetic cookie/auth traffic. Receipts verify filenames, MIME,
exact length and SHA-256. The Chromium test starts the production browser
lifecycle/controller, checks local and opaque-upload bytes, missing/denied
files, reset/no partial reuse, completed/canceled downloads, actual same-Tab
PNG geometry, conservative protected-image redaction and actual passive CDP
header provenance. Evidence must be an existing empty externally owned directory.
Owned browser, staging, profile and fixture state are removed even on failure.

The kernel source integration crosses real home/worker, encrypted relay,
controller stdio, artifact store, authenticated provider-tool routing and
attachment client authorization. A synthetic permitted transfer-store record
travels through real encrypted upload routing/staging and preserves Unicode
filename, length and hash; missing artifacts never reuse the prior selection.
Its CDP pixels are synthetic, so it does not
prove actual Chromium or official-provider perception. A separate MCP converter
check verifies byte-exact native image content; protocol snapshot/hash tests pin
420 and the attachment request. These checks complement the actual Chromium
fixture; they are not a live conjunction run.

Independent review, official-provider model-visible delivery, Web/local/remote
TUI conjunction, real transferred-file UX, installed PDF extraction, fresh
Path-1 comparison and outside-slice browser integration remain acceptance gates.
The coordinator schedules the live client/provider conjunction. None of the
source or fixture checks closes MP-08/MP-10/MP-11.
