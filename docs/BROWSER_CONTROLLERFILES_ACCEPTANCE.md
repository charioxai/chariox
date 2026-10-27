# MP-08/MP-10/MP-11: shared Browser artifacts and observation fixtures

The first-party fixtures change no benchmark solver, evaluator or frozen score.
Frozen G2 `9334141d420f8a32393f206102c5b8b4a1b0b609` fails the visible
**Choose documents** control with `browser_upload_invalid`. That RED evidence
remains attributed to G2. Protocol 426 adds the common artifact runtime described
below; a passing fixture does not close MP-08, MP-10 or MP-11.

## MP-08/MP-10/MP-11 runtime contract

The visible chooser uses the existing Browser action path: resolve the observed
control, arm Chromium file-chooser interception at physical click dispatch, and
require the same session and the observed control’s owning frame/document before
selecting files. Same-origin child controls share a renderer session and ordinary
backend references; the controller maps their Document backend ID through CDP’s
multi-document snapshot. It fences that frame tree and any isolated renderer
ancestors, and requires the chosen hidden input to belong to the same Document.
Navigation,
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
MP-08/MP-11 images use the recorded Vault fill targets: only a still-filled
plain text field receives a pixel mask. Password dots remain visible, and
show-password toggles are rechecked. Image metadata reports `fill_targets`
only when pixels were masked, otherwise `none`. The kernel validates both
values before publication and preserves the value in artifact identity metadata.
`full_viewport` remains readable for older captures. These are values of the
existing opaque artifact metadata; no typed daemon/relay shape changes.
Downloads are withheld while protected values are registered.

MP-08/MP-10/MP-11 isolated-password regression:
`node --test apps/kernel/slice-linux-docker/docker/browser-controller-image-capture.test.mjs`
checks exact image/mask bytes, owned/foreign renderer boundaries and identity
fences. The real Chromium controllerfiles drill forces site isolation and embeds
a localhost password iframe in its 127.0.0.1 tab without any registered Vault
value. It requires an actual iframe target absent from the top DOM snapshot,
then checks that unfilled password inputs alone do not mask the viewport.
`browser-fill-target.browser-test.mjs` checks actual plain/password fill capture
metadata and pixels at DPR 1/2. The Rust artifact regression validates and
publishes both redaction states through the operational artifact store.
These focused checks do not establish provider/Web/TUI or Path-1 parity.

Passive capture retains actual CDP request/response metadata for the same
Tab/document, including observed extra-info headers. It allowlists only Accept,
Content-Type and Content-Length, removes cookie/auth headers, userinfo, query,
fragment and bodies, and bounds history to 500 entries. `browser-network.har`
is a HAR-shaped metadata subset; it does not reconstruct headers or supply a
complete timing/body archive. Later registered protected values are scrubbed
again when the attachment is captured.

`RoomBrowserArtifact` exposes the same capture/read/inspect service to attached
Web/local/remote TUI clients, with membership and attachment-owner checks. The
shared TypeScript builder declares minimum local protocol 426. Client capture
with `return_image_base64=true` includes inline bytes only when the complete encoded
kernel WebSocket response fits its 1 MiB inline-image budget. Encrypted relay
replies retain their existing envelope limit. Larger captures still return the opaque
artifact metadata; clients read ordered chunks of at most 128 KiB and verify the
size and SHA-256. Native provider MCP image delivery retains the 8 MiB artifact
bound. This delivery bound changes no serialized shape or version. Runtime provider
calls use normal authenticated home admission and leased-worker routing. New
peer variants require the coordinator-allocated union relay peer protocol 73.
Existing lease admission/rebind and hosted token installation/confirmation gates
reject older peers; image preflight requires peer73 with matching runtime source
lineage. Released v70 workers cannot decode Artifact commands/results or opaque
upload objects.

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
canvas, same-origin child-frame chooser and synthetic cookie/auth traffic. Receipts verify filenames, MIME,
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
The socket regression uses a valid high-entropy canonical PNG whose inline
response exceeds 1 MiB, verifies complete artifact metadata through the shared Unix WebSocket
dispatcher, reads every chunk and compares bytes/hash, and retains inline provider
bytes. Its CDP pixels are synthetic, so it does not prove actual Chromium or
official-provider perception. A separate MCP converter
check verifies byte-exact native image content; protocol snapshot/hash tests pin
426 and the attachment request. These checks complement the actual Chromium
fixture; they are not a live conjunction run.

Independent review, official-provider model-visible delivery, Web/local/remote
TUI conjunction, real transferred-file UX, installed PDF extraction, fresh
Path-1 comparison and outside-slice browser integration remain acceptance gates.
The coordinator schedules the live client/provider conjunction. None of the
source or fixture checks closes MP-08/MP-10/MP-11.

## MP-08/MP-10/MP-11 peer73 compatibility review drill

```sh
cargo test --manifest-path apps/kernel/Cargo.toml --lib mp08_mp10_mp11_browser_artifact -- --test-threads=1
cargo test --manifest-path apps/kernel/Cargo.toml --lib restored_stale_or_unversioned_binding_is_rejected_until_rebound
node --test scripts/browser-artifact-peer-version.test.mjs scripts/slice-relay-identity-contract.test.mjs
```

On shared builders wrap every Cargo invocation in the allocated compile-slot
lock and respect the lane resource floors. The focused encrypted lease drill
registers a live loopback worker using the production dispatcher, changes only
its lease-version advertisement, and checks the home admission behavior. v70
must cause lease cleanup without a spawn request; v71 must bind and clean up.
The artifact integration separately exercises v71 image bytes and opaque upload
through the real encrypted home/worker/controller path with synthetic CDP.
The hosted token cases decrypt actual encrypted install/confirmation receipts
and exercise the production version/slice/nonce predicates. The image case runs
the production preflight predicate with synthetic Docker labels. These fixtures
are neither an old-v70 binary deployment nor a live Cloud token-refresh run.
No provider account credentials or provider profile files are fixture inputs.
