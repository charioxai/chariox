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

The zero-network contract is unchanged. On 2026-11-05 at 15:42Z the coordinator
required three layers: opaque frame plus inherited response CSP, a restricted
browser launch, and a zero-network capture covering the whole browser tree.

| Browser / conditions | Renderer | Network boundary | Product support |
| --- | --- | --- | --- |
| Chrome 155.0.8059.39, Linux headless, unrestricted fresh ordinary profile | Both Apps at DPR1/2 | FAIL: frame-triggered TCP speculation and DNS | None |
| Chrome for Testing 155.0.8059.79, unrestricted profile | Both Apps at DPR1/2 | Same FAIL | None |
| Chrome155.39, dedicated restricted launch/profile | Both Apps at DPR1/2 with data/blob stylesheet CSP | Frame controls improved; whole-tree gate remains FAIL on the IPv6 route socket; full matrix recorded below | None |
| Firefox, Safari/WebKit, Edge, macOS/Windows default browsers, native WebViews | NOT RUN | NOT RUN | None |

The original captures did not record PIDs; historical PIDs cannot be recovered
from those pcaps alone. `enforce-attribution/` reproduces the same seven classes
with startup-to-exit syscalls and native frame/renderer PID marks.

The seven reproduced failing cases are content initiated: iframe/self/replace/meta/
link navigation and explicit preconnect opened six TCP sockets at the owned
loopback receiver; DNS-prefetch emitted the exact signed test frame's synthetic
name. HTTP payload and STUN datagram counts were zero, which did not pass the
socket/DNS contract. Fresh syscall captures attribute the TCP work to Chrome's
Network Service; DNS work uses both that service and the browser process's
system-resolver pool. The GPU and frame renderer are separately identified.
The originating frame is known from its signed test operation and unique
receiver/name; the process making the syscall is a separate fact.

`chromium-launch.mjs` freezes the requested dedicated-browser arguments:
no proxy; map every hostname to `~NOTFOUND` except localhost/127.0.0.1/::1;
disable background networking, component updates/extensions, default apps,
domain reliability, sync, crash reporting, phishing detection, Safe Browsing
updates, pings, optimization services, media routing, translation, prefetch and
prerender services. Actual complete argv, including Playwright defaults, is in
each PID map. Chrome155 no longer provides the old `disable-preconnect` switch.
The isolated profile also sets `net.network_prediction_options=2`, turns off
Safe Browsing and search suggestions, and disables DoH. Flags alone did not
stop the six loopback speculative sockets; prediction-disabled preferences did.
The resolver rule stops the explicit frame DNS-prefetch query.

A remaining browser-internal operation is Network Service UDP `connect` to
`[2001:4860:4860::8888]:443`. It occurs with the complete restricted launch and
prediction-disabled profile. Chromium's
[HostResolverManager IPv6 reachability check](https://raw.githubusercontent.com/chromium/chromium/main/net/dns/host_resolver_manager.cc)
uses this hard-coded address for a route check: connect and inspect the local
address, without sending a DNS request or datagram. Captured syscall files and
NetLog reachability events are the evidence; the source explains their purpose.
The [Chrome tooling flag guide](https://raw.githubusercontent.com/GoogleChrome/chrome-launcher/main/docs/chrome-flags-for-tools.md)
and [prediction preferences implementation](https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/preloading/preloading_prefs.cc)
provide the launch/profile rationale. Neither documentation nor an ignored flag
is treated as proof that a service stopped.

**The whole-tree zero-non-loopback-socket gate remains FAIL.** A diagnostic
report proposes only the exact Network Service UDP-connect route probe as an
exception, with no send operation, no DNS and no other destination/process.
It never applies that exception to acceptance. The lane coordination request
recommends retaining the original failing gate until the probe is eliminated
or the owner explicitly accepts that precise narrower gate. AV1 is not accepted
under either gate in this round.

Use fresh **ordinary** profiles: private profiles suppress preconnect/DNS
controls. Every strict network case runs at actual DPR1 and DPR2. `strace -ff`
starts before Chrome exec, follows all child processes/threads through shutdown,
and records all socket calls; CDP plus `/proc` labels processes and threads.
Native User Timing marks identify each live frame's renderer PID in the trace,
including the separate private contexts used for storage/native input and source
binding. Their browser also uses the restricted launch/profile.
The gate includes browser startup/shutdown, so background work is not hidden
outside a chosen frame interval. A filtered pcap additionally records the owned
receivers and synthetic App DNS questions without collecting other lanes' data.
Unknown destinations/process attribution and any incomplete thread capture fail
the gate. DNS summaries count send operations; sendmmsg may batch several wire
questions, whose packet count is available in the pcap. The root-only test
option `--no-sandbox` concerns Chromium's OS sandbox; the HTML sandbox is present.

An ordinary Web page cannot enforce these launch settings or disable other
browser services. Recommended integration default: AV4/AV5 must own a dedicated
browser process and fresh restricted profile, or fail closed. This AV1 candidate
does not add that product integration or change the kernel. No ordinary Web
support, live Web/TUI/relay acceptance or automatic streamed fallback is claimed.

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
| `style` | Stylesheet text with embedded assets, installed as a data URL |

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
style-src data: blob:; img-src data: blob:; font-src data: blob:;
media-src data: blob:
```

The trusted host adds `script-src 'unsafe-inline' 'wasm-unsafe-eval'`. Its
static policy cannot enumerate each later App's hashes: adding a host nonce or
hash would suppress `unsafe-inline` and also prevent independently hashed App
scripts under the inherited policy. The child's first meta CSP instead adds
only the three generated script hashes and `wasm-unsafe-eval`.

Stylesheets are data URLs, including the trusted host layout; no inline-style
CSP grant is present. Native libraries still render using their CSSOM layout
operations. Data/blob fonts/images/media support embedded assets/local drawing. No HTTP/WSS source,
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
  /root/.codex/evidence/browser-resume-20260930/appview/r3/enforce-complete \
  /root/.chariox/dev/appview-av1-complete
```

Use new evidence/scratch directories on each run. Keep the scratch path short:
Chrome needs a TMPDIR of at most 60 bytes for its Unix socket. Prerequisites are
Corepack/pnpm/Node22, Chrome, strace, tcpdump and the lane's bounded-workload wrapper.
Dependency installs use frozen locks without dependency install scripts; the
needed web/reference build commands run explicitly. `CHARIOX_TEST_CHROME`
selects another installed browser; `CHARIOX_BOUNDRUN` selects the wrapper on a
different builder. The current expected browser exit is **1** for the unmet
boundary; the command records each step's exit status in `steps.tsv`. A nonzero
browser exit is not converted into a passing known-failure assertion.

Evidence root: `/root/.codex/evidence/browser-resume-20260930/appview/r3/`.
`final/` and `final-15579/` retain the original failing ordinary-profile runs.
`enforce-matrix/` records 72/72 frame-traffic checks and controls at DPR1/DPR2,
4/4 reference renders, zero strict DNS and zero frame receiver sockets. All 76
strict browser runs retain only the IPv6 route socket, so the whole-tree gate
fails. `enforce-layer-probe/` independently tests resolver, frame CSP and idle
browser behavior. `enforce-final/` is the first clean-head reproduction. `enforce-complete/`
adds restricted-launch whole-tree capture for the storage/native-input/source-
binding/signature contexts as well, with native renderer PID marks and explicit
parent URLs for all captured frames. Both are honest nonzero gate runs.
The earlier `enforce-{probe,launch-probe,prefs-probe}/` captures isolate the
original attribution, flags-only result and additional prediction preference.
Each final run has:

| Check | Evidence |
| --- | --- |
| Signature/hash/entry tampering, Rust signing vector, Unicode and size limits; full inventory with portable license names | `verifier.log`; `verifier-before.log` / `verifier-after.log` and `inventory-before.log` / `inventory-after.log` in the evidence root for resumed fixes |
| Cloud static response policy, reports removed, malformed host fails clearly | `host-response.log`; `response-before.log` / `response-after.log` at the root |
| 36 network/navigation/fresh-document classes at each DPR, receiving controls and strict header/sandbox audit | `results.json` → `attacks`; named files under `netlog/`; `console.json` |
| Whole browser tree, actual frame/renderer PID, requested flags/effective profile, complete socket syscalls and unchanged zero-network result | `process-tree/<case>/pid-map.json`, `frame-processes.json`, `launch.json`, `syscall.<tid>`, `network.json`; `attribution.json` |
| Independent resolver/direct-IP and frame-CSP controls; idle browser with no App | `results.json` → `launch`, `process-tree/launch-*` |
| Capture parser rejects DNS, unknown senders, descriptor reuse, batched destinations and incomplete captures | `verifier.log`; root `enforce-gate-before.log`, `enforce-gate-after.log`, `enforce-completeness-before.log`, `enforce-all-unit.log` |
| Data/blob-only enforced response policy, old unsafe-inline policy fails first | Root `enforce-policy-before.log`, `enforce-policy-after.log` |
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
| AV1 Web boundary | Eliminate the exact browser IPv6 route socket or obtain an explicit owner decision on that precise exception; zero-network gate remains FAIL |
| AV2 assets/admission | Kernel-authenticated descriptor, trusted key projection, release lease, inventory/chunk delivery, cancellation/revocation and cache rules |
| AV3 backend/state | Generation-fenced operation/state port and receipts, declared actions, committed state and bounded streams; no live UI-adapter authority |
| AV4 Web | Enforceable dedicated browser launch; actual App-open routing into host alongside the real conversation, gestures/clipboard/files and reconnect/update behavior on the existing WSS connection |
| AV5 ordinary TUI | Dedicated restricted-browser loopback host bound to 127.0.0.1, exact Host/Origin, cross-site/upgrade rejection, one-use fragment handoff, credential-free assets and guarded exact-process cleanup |
| AV5 managed/remote TUI | Same static host and verification, assets over its existing encrypted kernel connection, correctly scoped browser handoff and relay lifecycle; no second App HTTP service on the kernel |
| AV6 | Kernel-authorized field handles and trusted model-observation masking; native accessibility/IME and real clipboard/file grant coverage |
| AV7 / MP parity | Real Web/TUI/provider/hosted-WSS opens, realistic network and duration, mobile/default-browser matrix and eventual old-path deletion after acceptance |

MP-09 idle semantics are unchanged; this renderer acquires no kernel activity
lease. Ordinary/managed placement does not select different renderer code, but
no live placement parity or MP item is closed by these source/renderer checks.
