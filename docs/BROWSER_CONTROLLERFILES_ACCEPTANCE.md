# MP-08/MP-10/MP-11: shared file and observation fixtures

The controllerfiles fixture is first-party functional evidence. It changes no
benchmark solver, evaluator, frozen score, protocol shape or runtime authority.
The shared artifact-store correction is a separate common service change.

Run the small HTTP fixture checks with:

```sh
node --test apps/cli/scripts/lib/browser-controllerfiles-fixture.test.mjs
```

The fixture has a visible **Choose documents** button backed by a hidden file
input, cancellation, strict multipart receipts, deterministic Unicode, binary
and valid one-page PDF downloads, a slow cancellable download, a visual canvas,
and a network endpoint with synthetic cookie/auth traffic. Receipts retain
only filename, MIME type, exact byte length and SHA-256. Malformed, absent,
oversized and over-count uploads create no receipt. The strict receipt checker
rejects any name/type/length/hash mismatch.

Run the installed-Chromium controller check in a disposable Linux environment:

```sh
CHARIOX_TEST_CHROMIUM=/absolute/path/to/chromium \
CHARIOX_CONTROLLERFILES_EVIDENCE=/absolute/external/evidence \
node --test apps/kernel/slice-linux-docker/docker/browser-controllerfiles.browser-test.mjs
```

The evidence directory must already exist, belong to the executing user, and
be empty. No dependencies or browsers are downloaded. The test starts Chromium
through the production browser-lifecycle helper with its normal sandbox,
uses the production controller and stager, and removes the owned browser,
lifetime records, staging, files, profile and fixture even on assertion failure.
Container runs must have explicit resource limits and use only lane-owned
containers and external evidence directories.

On frozen G2, the final visible-chooser assertion is deliberately RED with
`browser_upload_invalid`. Earlier controls use the raw observed input reference
and verify actual local uploaded file receipts, cancel/reset, missing/denied
files, exact downloaded bytes, active download cancellation and retirement,
same-session PNG bytes and document/viewport identity, and redacted actual CDP
network events. The PNG and event metadata are written before the chooser
assertion to retain this first failing product seam. An evidence-write failure
is a harness failure and must not be reported as a chooser reproduction.

This is controller/Chromium fixture evidence. It does **not** prove an actual
kernel-owned Browser capture, a model-visible MCP image, a HAR attachment,
opaque completed-download inspection, permitted opaque-artifact upload,
transferred or remote files, Vault screenshot redaction, or provider/Web/local
and remote TUI conjunction. The direct screenshot is a verifier capture from
the controller's actual session; it is not a new runtime tool or an attachment
delivery path. CDP event metadata omits headers rather than fabricating HAR
headers. None of these runs closes MP-08, MP-10 or MP-11.

The new runtime contract remains coordinator-gated: reserve the shared
controller/projection files with r2next and obtain a protocol allocation before
adding artifact-ID upload, completed-download read/inspect, same-tab Browser
image capture or passive kernel-browser attachment commands. Keep Room/home
admission, leased-agent routing, the common artifact service and official
provider attachment paths as the only authorities. Repeat these fixtures
through that full path after implementation and independent review.
