# MP-08 / MP-09 / MP-10 / MP-11 — client App view feasibility

AV1 result, 2026-11-05: **renderer PASS; containment FAIL**. This is an unused
renderer candidate, not an admitted App-open path. Do not cut over from streamed
Apps or describe either tested browser as supported for contained App views.
The coordinator confirmed this round's renderer-only exit in the lane review
inbox on 2026-11-05 at 14:16Z; session, conversation, relay and TUI opens remain
dependent on AV2–AV5.

The implementation is in `packages/app-view-host`. Cloud vendors its five runtime
modules under `apps/web/src/app-view-host`, checks their SHA-256 pins during the
normal web build, and emits `/assets/app-view-host/host.html` and `client.mjs`.
`sync-app-view-host.mjs --check` also compares those copies to the paired OSS
checkout. No kernel/protocol shape, App admission, managed shutdown policy,
streamed renderer or Vault/observation behavior changes here.

## Browser result and the blocking seam

| Browser / conditions | Renderer | Required network boundary | Product support |
| --- | --- | --- | --- |
| Chrome 155.0.8059.39, Linux headless, fresh ordinary profiles | Both reference Apps render at DPR 1/2 | FAIL at DPR1: speculative navigation / explicit preconnect TCP and DNS-prefetch | Not established |
| Chrome for Testing 155.0.8059.79, same conditions | Both reference Apps render at DPR 1/2 | Same FAIL at DPR1 | Not established |
| Firefox, Safari/WebKit, Edge, macOS/Windows default browsers, native WebViews | NOT RUN | NOT RUN | Not established |

The strict frame produces zero HTTP requests and zero WebRTC UDP packets at the
local receivers, but that is insufficient. Each of iframe navigation, self
navigation, `location.replace`, meta refresh, link navigation and explicit
preconnect opens an accepted TCP socket with zero application bytes. Explicit
DNS-prefetch produces DNS query events for the strict test name. The host has
`X-DNS-Prefetch-Control: off`; both documents have the equivalent meta setting.
These controls do not establish a network boundary in the tested engines.
The final packet captures also contain the strict DNS questions at the local
resolver, independently of the browser NetLog. Moving the parent CSP from a
meta policy to an HTTP response header also did
not stop the navigation sockets (see `navigation-probe.json`).

Network containment was tested at DPR1. The reference renderer/canvas checks
ran at both DPRs. DPR2 network containment is not established; the existing
DPR1 failure already blocks browser support. This evidence is not a live
Web/TUI/relay or native accessibility acceptance matrix.

Use **ordinary profiles** for this proof. Chromium private contexts suppress
explicit preconnect and DNS-prefetch; the earlier private-context runs had no
positive receiving control for those cases. The final drill uses a fresh normal
profile for every network case and reference render, enables background
networking, writes each browser's NetLog, and removes that profile after closing
it. Playwright still supplies its other automation defaults. Root's
`--no-sandbox` launch option concerns the OS process sandbox; the actual HTML
iframe sandbox is present and tested independently.

AV1 needs an enforceable browser/network containment mechanism or an explicit
owner revision of the contract before it can pass. There is no JS navigation
interposition workaround, browser preference requirement, weakened assertion,
automatic streamed fallback or supported-browser claim in this change.

## Packer and verification contract for AV2

The view entry is one UTF-8 carrier file. It contains this exact prefix, followed
by standard padded canonical base64 of a canonical JSON object, followed by
`</script>`:

```html
<!doctype html><meta charset="utf-8"><script type="application/octet-stream" id="chariox-app-view">
```

The JSON has exactly four fields:

| Field | Value |
| --- | --- |
| `schema` | `chariox.app-view-source.v1` |
| `rootId` | Lowercase ASCII letter followed by up to 63 lowercase letters, digits, underscores or hyphens |
| `script` | One statically bundled ESM entry, including local asset bytes |
| `style` | Inline stylesheet text with embedded assets |

Publisher strings are encoded as data, never interpolated into HTML raw-text
script/style bodies. The carrier cannot execute App code on its own. Its final
encoded byte length is bounded at 16 MiB, including base64 expansion. Reference
builds retain their existing 8 MiB frontend budgets. Excalidraw's `--client-view`
build turns off splitting and embeds fonts; the reference producer resolves its
pinned runtime font literals to data URLs. It retains upstream legal comments
inline. Univer embeds its asset imports, keeps its existing pinned English
dictionary build, and removes one unused upstream demo CSS rule containing an
external SVG URL. Original build notices remain in the reference build output.

The reference producer rejects unresolved static/dynamic JS imports using the
esbuild output graph and rejects non-data CSS asset URLs. It is specific to the
two pinned references, not a general JS analyzer. Optional upstream worker/eval
branches may remain in bundled library code; runtime worker/eval restrictions
are tested, and features that require them have not been accepted. All embedded
bytes are included before signing.

`verifyView` accepts a projection with canonical `manifest` and `inventory` JSON
strings, the package publisher `signature`, and exactly one `files` item:
`{path, body_base64}` for `manifest.ui.entry`. The trust input is separate:
`publisherId`, `keyId`, `publicKey`, `contentDigest`. AV2 must supply it from the
authenticated kernel's admission decision. An archive's own key cannot establish
trust. The client uses the existing v2 package signing subject
`{schema:"chariox.package-signature.v2",manifest,inventory}`, Ed25519, SHA-256,
the signed inventory size/digest, and exact entry selection before assembling
executable content. The Rust package golden vector is consumed by the JS tests.
JSON nesting/node limits and interoperable numbers/Unicode are checked; control
canonicalization rejects duplicate keys. Delivery is all-or-nothing.

This client verifier is **not an archive installer**. The reference producer's
in-memory signing key and adjacent public trust file are drill fixtures. Its
frontend-only inventory is not a complete installable v2 archive and has not
passed kernel admission. AV2 must project the real complete signed manifest and
inventory, obtain a release lease, deliver bounded chunks through the existing
authenticated connection, and hash the completed entry. It must not promote
these drill trust files into a production trust root.

## Static host and bootstrap contract for AV3–AV6

`mountAppView(container, {hostUrl, bundle, trust, timeoutMs})` returns
`{ready, close}`. The host URL must be same-origin and contain no query or
fragment. It contains only trusted build code, never conversation data or App
HTML. Opening it directly provides no App entry or service authority.

The private lifecycle exchange is:

1. Host sends `chariox.app-host.ready` to its same-origin parent.
2. The client matches the exact host window and origin, then sends
   `chariox.app-host.attach` with the bundle, separate trust, and exactly one
   transferred MessagePort.
3. The host accepts once, only from its exact parent and origin. It verifies
   bytes, constructs one sandboxed `srcdoc`, then sends
   `chariox.app-host.mounted` or `chariox.app-host.failed` on that private port.
   The port closes after the result. `mounted` means verified document appended,
   not App state loaded or first paint complete.
4. Client timeout/failure removes the host. `close` removes it and ends any
   pending readiness wait. A replacement uses a new host/window/port.

The App iframe has `sandbox="allow-scripts"`, no same-origin, popup, top
navigation, form or download grant, and no referrer. Permissions Policy denies
camera, microphone, geolocation, display capture, payment and device APIs. Its
origin is opaque. Before any publisher code, the trusted bootstrap installs
non-configurable fences for WebRTC aliases, WebTransport and IndexedDB. It
provides bounded, document-lifetime local/session preference maps for libraries;
these reset on every replacement and are not accepted-draft persistence.
Cookie, database, CacheStorage and service-worker persistence are unavailable.

The assembly script installs the stylesheet, the declared root and the verified
ESM text. The bootstrap/assembly/App script hashes are computed over their actual
text. There is no App HTTP entry, worker loader, CDN lookup or service worker.
The host must be served with its generated CSP as an enforced response header,
`frame-ancestors 'self'`, no CSP reporting destinations, no-store, and DNS-prefetch
off. Cloud's exact static-asset response implements this, including removal of
the general report-only policy. Other Cloud pages retain their own policies.

AV1 implements only the host lifecycle port. There is **no live App service
port, backend RPC, SDK admission, generation, state subscription or authority
identifier** in this renderer. The frontend-only reference adapter is test-only.
AV3 must bind the single App service port to the admitted descriptor's view,
owner/Room, generation and declared operations; App messages cannot choose them.
AV4 must accept only that port in real conversation chrome. An App can emit raw
`postMessage` events: isolation means they are ignored, not that the browser
prevents their delivery. The exact-parent test rejects an otherwise same-origin
sibling's valid signed attach. A real neighbouring conversation is still blocked
on those later slices.

Native keyboard copy/paste and a native file-input chooser work in the opaque
frame. Tests use real keyboard events and select a file through the browser's
file chooser event. They do not prove async Clipboard API permission, backend
file grants, exports, host clipboard service admission or Vault filling. Those
need the declared, gesture-bound host/broker paths. AV6 must add trusted field
identities/masking before any model capture claim; this round implements none.

## CSP frozen as the rendering candidate, rejected as sufficient containment

Both documents use:

```text
default-src 'none'; connect-src 'none'; frame-src 'none'; child-src 'none';
worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none';
style-src 'unsafe-inline'; img-src data: blob:; font-src data:;
media-src data: blob:
```

The trusted host adds `script-src 'unsafe-inline' 'wasm-unsafe-eval'`. Its
static policy cannot enumerate each later App's hashes: adding a host nonce or
hash would suppress `unsafe-inline` and also prevent independently hashed App
scripts under the inherited policy. The child's first meta CSP instead adds
only the three generated script hashes and `wasm-unsafe-eval`.

Inline styles support native layout/canvas libraries; data fonts/images and
image/media blobs support embedded assets/local drawing. No HTTP/WSS source,
external script, worker or JS `unsafe-eval` is permitted. Removing
`wasm-unsafe-eval` from both applicable policies makes byte compilation fail;
plain eval/Function compilation remains denied with it present. This is the
small rendering policy tested with the references, not a claim of a sufficient
network boundary or proof that every upstream feature works.

## Reproduction and evidence

From the paired OSS checkout, one command installs pinned Node dependencies,
builds both references, tests the verifier and Cloud response, builds the real
Cloud web entry/assets, runs Chromium checks and cleans its processes:

```sh
bash packages/app-view-host/test/run-av1.sh /root/work/agent-appview-cloud \
  /root/.codex/evidence/browser-resume-20260930/appview/r3/final \
  /root/.chariox/dev/appview-av1-final
```

Use new evidence/scratch directories on each run. Keep the scratch path short:
Chrome needs a TMPDIR of at most 60 bytes for its Unix socket. Prerequisites are
Corepack/pnpm/Node22, Chrome, tcpdump and the lane's bounded-workload wrapper.
Dependency installs use frozen locks without dependency install scripts; the
needed web/reference build commands run explicitly. `CHARIOX_TEST_CHROME`
selects another installed browser; `CHARIOX_BOUNDRUN` selects the wrapper on a
different builder. The current expected browser exit is **1** for the unmet
boundary; the command records each step's exit status in `steps.tsv`. A nonzero
browser exit is not converted into a passing known-failure assertion.

Evidence root: `/root/.codex/evidence/browser-resume-20260930/appview/r3/`.
`final-15579/` contains the newer browser run; `final/`
contains the complete one-command run. Each has:

| Check | Evidence |
| --- | --- |
| Signature/hash/entry tampering, Rust signing vector, Unicode and size limits; full inventory with portable license names | `verifier.log`; `verifier-before.log` / `verifier-after.log` and `inventory-before.log` / `inventory-after.log` in the evidence root for resumed fixes |
| Cloud static response policy, reports removed, malformed host fails clearly | `host-response.log`; `response-before.log` / `response-after.log` at the root |
| 36 network/navigation/fresh-document classes, rule-removal controls | `results.json` → `attacks`; named files under `netlog/`; `console.json` |
| Separate HTTP/TCP/UDP receivers; zero-byte strict sockets; only this drill's DNS names captured | `receiver.pcap`, `receiver-ports.json`, `capture-filter.txt`, `results.json` → `receiver`, `tcpdump.log`, `capture-process.json` |
| Cookie/local/session/database/cache/service-worker revisit controls | `results.json` → `isolation.persistence` |
| Native clipboard/file input, wasm versus eval, bootstrap immutability | `results.json` → `isolation` |
| Exact parent/source binding; signed sibling control; invalid signature never mounts | `results.json` → `sourceBinding`, `rejectedBeforeExecution` |
| Both real frontend libraries from the built Cloud entry, DPR 1/2, native canvas/fonts, drawing/formula | `excalidraw-dpr{1,2}.png`, `univer-dpr{1,2}.png`, `results.json` → `references` |
| Host header versus meta navigation socket comparison | Root `navigation-probe.mjs`, `navigation-probe.json` |

The reference fixture mounts the Cloud-built host/client into the real built
Cloud page with test-only launcher chrome and a neighbouring DOM canary. The
real sign-in/session/App command flow is not replaced or claimed as tested.
Excalidraw has pointer-driven drawing plus embedded-font text; Univer has its
actual canvas and formula engine plus native pointer input. Frontend SDK calls
use an in-memory adapter. The screenshots and library semantics are renderer
proof, not backend commit/reconnect/update durability or provider acceptance.

## What the TUI hosts and later exits still need

| Slice / surface | Missing real-path requirement |
| --- | --- |
| AV1 Web boundary | Enforce zero unauthorized TCP/DNS as well as HTTP/UDP; rerun the complete ordinary-profile matrix before declaring browser support |
| AV2 assets/admission | Kernel-authenticated descriptor, trusted key projection, release lease, inventory/chunk delivery, cancellation/revocation and cache rules |
| AV3 backend/state | Generation-fenced operation/state port and receipts, declared actions, committed state and bounded streams; no live UI-adapter authority |
| AV4 Web | Actual App-open routing into host alongside the real conversation, gestures/clipboard/files and reconnect/update behavior on the existing WSS connection |
| AV5 ordinary TUI | Default-browser loopback host bound to 127.0.0.1, exact Host/Origin, cross-site/upgrade rejection, one-use fragment handoff, credential-free assets and guarded exact-process cleanup |
| AV5 managed/remote TUI | Same static host and verification, assets over its existing encrypted kernel connection, correctly scoped browser handoff and relay lifecycle; no second App HTTP service on the kernel |
| AV6 | Kernel-authorized field handles and trusted model-observation masking; native accessibility/IME and real clipboard/file grant coverage |
| AV7 / MP parity | Real Web/TUI/provider/hosted-WSS opens, realistic network and duration, mobile/default-browser matrix and eventual old-path deletion after acceptance |

MP-09 idle semantics are unchanged; this renderer acquires no kernel activity
lease. Ordinary/managed placement does not select different renderer code, but
no live placement parity or MP item is closed by these source/renderer checks.
