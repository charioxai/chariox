# MP-08 / MP-09 / MP-10 / MP-11 — client App view feasibility

AV1 result, 2026-11-05 (round 6): **PASS, accepted WITH the named
`chromium-network-service-ipv6-route-probe` exemption** on both pinned built-Cloud
matrices at DPR1 and DPR2. This is a renderer/security proof, not an admitted App-open path.
Session, conversation, relay and TUI opens remain dependent on AV2–AV5.

The implementation is in `packages/app-view-host`. Cloud vendors its five runtime
modules under `apps/web/src/app-view-host`, checks their SHA-256 pins during the
normal web build, and emits `/assets/app-view-host/host.html` and `client.mjs`.
`sync-app-view-host.mjs --check` also compares those copies to the paired OSS
checkout. No kernel/protocol shape, App admission, managed shutdown policy,
streamed renderer or Vault/observation behavior changes here.

## Complete-matrix acceptance (round 7)

The shared final assessment now requires the exact 36 attack names at both
DPR1 and DPR2, once each. Missing, duplicate, unexpected or filtered rows cannot
accept containment, including an empty unmatched filter. This gate is independent
of the drill invocation flags. Seven fail-first regressions rejected the old
assessment; all 30 route-probe regressions remain intact.

Both pinned built-Cloud reruns passed all 72 attacks and four reference renders:
Chrome155.39 audits133 exempt connects and Chrome155.79 audits132. The real
fetch-only run (two attack rows) and unmatched filter (zero rows) both exit1
with `complete-attack-matrix`; renderer checks still pass. Exact identities and
receipts: `/root/.codex/evidence/browser-resume-20260930/appview/r7/AV1_IDENTITY.json`.
The signed-reference AV2 transport slice is separate; these AV1 captures do not
prove kernel delivery, backend state or the App-open cutover.

## Whole-browser gate with one named exemption

Miguel authorized this exception on **2026-11-05 at 19:53 CET**. Chromium's
internal IPv6 route-reachability check remains present with the restricted launch
and profile. Round 4 found no applicable probe-skipping Chrome155 launch flag;
an IPv6-disabled private network namespace still records the failed connect.
Pinned source inspection and native/namespace/feature captures are in
`/root/.codex/evidence/browser-resume-20260930/appview/r4/PROBE_RESEARCH.md` and
its sibling capture directories. Chromium's
[HostResolverManager IPv6 reachability check](https://raw.githubusercontent.com/chromium/chromium/155.0.8059.79/net/dns/host_resolver_manager.cc)
uses the hard-coded destination to inspect the local route without sending data.

The explicit allowlist entry is `chromium-network-service-ipv6-route-probe` in
`test/network-assessment.mjs`. **Every condition must hold**:

1. The operation is `connect()` on an `AF_INET6 SOCK_DGRAM` socket created by
   the process identified as `network.mojom.NetworkService` by the existing
   process/thread map. Browser, renderer, GPU, unknown and inferred child
   identities cannot qualify. Socket creation and identity must be recorded.
2. The destination is exactly `[2001:4860:4860::8888]:443`.
3. The socket is connect-only for its entire captured lifetime: no `sendto`,
   `sendmsg`, `sendmmsg`, `write`, `writev`, `sendfile` or `splice`, including
   through duplicated descriptors or after reconnecting it. The receiver/packet
   capture must show zero strict receiver packets and zero packets to the probe
   address. Since every other non-loopback destination fails the syscall gate,
   no other external destination can survive the final assessment.
4. The startup-to-exit trace is complete, with no unknown destination,
   unidentified sender or incomplete file. The packet capture must finish
   successfully with zero kernel drops; missing evidence fails closed.
5. Every DNS send, TCP operation, other destination/port, send on a probe socket,
   or probe from another process still fails. Deliberately weakened receiving
   controls remain outside the strict conjunction and must still demonstrate
   the denied traffic. No general UDP, failed-call or browser-process exception
   exists. Successful and failed route connects are both audited.

`test/drill-result.mjs` applies that assessment to **every** strict attack,
reference, isolation and idle/literal/DNS/frame-CSP launch capture. Raw
`network.json` findings and the original zero-non-loopback result remain visible;
they are not rewritten into zero sockets. The final result records the named
exemption with each exact PID/TID, socket identity, destination, syscall result,
trace file and source line. `attribution.json` repeats that audit. The filtered
pcap adds the probe destination to the owned receivers and synthetic App DNS
names, without collecting unrelated lanes' resolver payloads.

`chromium-launch.mjs` freezes the dedicated-browser arguments: no proxy;
map hostnames to `~NOTFOUND` except loopback; disable background networking,
updates/extensions/default apps, sync, domain reliability, crash/phishing/Safe
Browsing updates, pings, optimization/media/translation/prefetch/prerender services.
Actual complete argv is in each PID map. A fresh ordinary profile also sets
`net.network_prediction_options=2`, disables Safe Browsing, search suggestions
and DoH. Flags alone did not stop speculative sockets; the profile preferences
did. Resolver mappings block the explicit frame DNS-prefetch query.

The unrestricted ordinary Chrome155.39 and .79 baselines failed with
frame-triggered TCP/DNS from iframe/self/replace/meta/link navigation, preconnect
and DNS-prefetch. Attribution separates the originating signed frame operation
from the Network Service/browser resolver threads making the syscalls. GPU and
frame renderer PIDs are independently identified by CDP/proc and native User
Timing marks. Fresh ordinary control profiles are essential: private profiles
suppress preconnect/DNS controls. The root-only `--no-sandbox` option concerns
Chromium's OS sandbox; the HTML sandbox remains present.

Cloud's development server serves `.mjs` as JavaScript. The response test uses
`createStaticServer`, the same path as web dev; a real module-loader regression
rejects the old octet-stream response and imports `mountAppView` after the fix.

An ordinary Web page cannot enforce browser launch settings. AV4/AV5 must own a
dedicated browser process and fresh restricted profile, or fail closed. This
AV1 candidate adds no product integration and claims no ordinary Web support,
live Room/backend/provider/Web/TUI/relay acceptance or streamed fallback.
Firefox, Safari/WebKit, Edge, other operating systems and native WebViews are
NOT RUN. AV2–AV5 remain deferred.

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

`mountAppView(container, {hostUrl, bundle, trust, call, timeoutMs})` returns
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
   A failure closes the port; after `mounted` the port stays open and carries the
   page's backend calls (below). `mounted` means verified document appended,
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

## Page-to-backend call (AV3-AV6 slice 1)

`window.chariox.call(method, input)` is the page's only service surface, installed
by the bootstrap before any App script as a frozen, non-configurable global. It
returns a promise for the JSON result, or rejects with an `Error` carrying `code`.
The call travels page -> host -> embedding client over two private MessagePorts:

1. After the App frame loads, the host transfers one port to it (`chariox.app-view.port`,
   accepted once, only from the host window). Calls made earlier wait for it.
2. The page sends `{type:"chariox.app-view.call", id, method, input}` on that port.
   The host reads it with `readCall` (method 1-256 characters, input JSON of at most
   1 MiB, at most 16 calls in flight, else an `INVALID_CALL` or `TOO_MANY_CALLS`
   answer), renumbers it and relays it to the client port from the attach message.
3. The client's `call(method, input)` runs under the client's own authority and
   answers `{type:"chariox.app-view.result", id, ok, result | error:{code,message}}`.
   Only a code and a sentence the client names reach the page; any other failure
   becomes `CALL_FAILED` without detail. Stopping the host answers `VIEW_CLOSED`.

The client holds the connection and the view lease; the page never does. Declared
methods, input schemas and admission stay the kernel's: Web sends `CallUserAppView`
over its existing link and the kernel refuses an unknown method or a wrong input.
There is still no generation fence, state subscription or `action`/`view` surface.
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

## View lease and backend-served views (AV3-AV6 slice 2)

A view a client renders itself (`OpenUserAppView` without `kernel_browser`) is leased to
that client for 60 s. Any owner `CallUserAppView` renews it; the built-in method
`chariox.lease` renews it and answers `{lease_ms}`, which Web calls every quarter of the
lease while the window lives. A closed or crashed tab never closes its view, so the
kernel ends it when nothing renews it (`UserAppViews::lease/renew`, one watcher task per
view); a client whose renewal is refused opens a new view, which is also how a reload
re-attaches. Kernel-browser views have no lease: their Tab owns their lifetime.

An App whose backend serves its view declares its actions once, in the signed `ui-actions`
document, and hands that document to the Rust SDK's `view::View` (`View::declare(..)`;
`register()` in `Guest::register`, `serve(..)` in `call_tool`). The SDK registers and answers the
tools the kernel reads the view through (`view_actions`: the registered actions in the shape of the
signed declarations; `view_state`: the state the App builds when asked; `view_changes` when the App
offers the feed), so the App never names them; the packer lists them in the manifest beside the
declared actions. `app_view_catalog/invoke/state/batch` for an installation the agent holds then run
with no page, Tab or browser: the declared name and input schema and the approval effect are
checked from the signed declarations, the backend's `view_actions` answer must equal the signed
declaration of that action on every call (`view_mismatch` / `action_not_registered`), and the
action runs as the backend tool of the same name under the agent's own App grant.

`app_view_changes` reads the backend's feed (AV3-AV6 slice 3). The App commits each change with
`view::transact_with_change(tx, delta)`, in the same transaction as its own writes: the SDK keeps
the head revision and the last 128 changes (8 KiB each, so the feed stays within 1 MiB) in the
App's durable state. `view_changes {since}` answers `{latest, dropped, changes: [{seq, delta}]}`
like a page's feed (`dropped` when the reader missed evicted changes: read the state again). The
kernel hands a reader at most 128 changes in at most 1 MiB, in revision order and not past the
head, whatever a backend answers (`view_error` otherwise); `wait_ms` makes it ask again every
250 ms until a change exists or the wait ends. An App that offers no `view_changes` tool answers
`changes_unavailable`.

Excalidraw is such an App (AV3-AV6 slice 4): its backend runs every declared action on the
saved scene (`add_elements`, `connect_elements` and `update_elements` repeat the library's
converter, text layout and arrow binding in Rust, and are tested against 78 requests that the
real library answered in Chrome), serves `view_state` as the scene summary and writes the feed
in the same transaction as every save, its own and the window's. The page has no action
registrations any more: with the window open it asks the feed once a second and merges the saved
scene into its canvas with Excalidraw's reconciliation, so the agent's edits show live and the
person's edits and the agent's both survive. A deleted element stays in the saved scene,
marked, for an hour so that a window which has not yet seen the deletion removes it. An agent
exports JSON; PNG and SVG need the window's renderer. Univer's actions and the kernel's own
page-registration path (`controller_call`, `browser_controller_app_view`, the Room and User
page branches) remain until Univer's backend serves its view.

## CSP frozen as one layer of containment

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

Round 6 completed both pinned Chrome matrices on OSS `8b93dfb54b` and Cloud
`e52b13392a`, using identical hashed Cloud host/client assets. This accepts only
AV1's renderer/security proof **WITH** `chromium-network-service-ipv6-route-probe`;
it does not assert zero sockets or real Room/backend/provider acceptance.
The tested configuration is Linux headless, fresh ordinary dedicated restricted
profiles, root-only `--no-sandbox`, and actual renderer DPR1/DPR2.

| Browser | Frame/policy | Positive controls | References DPR1/2 | Strict captures | Audited exempt connects | Gate / exit |
| --- | --- | --- | --- | --- | --- | --- |
| 155.0.8059.39 | 72/72 | 72/72 | 4/4 | 81/81 | 129 | PASS / 0 |
| 155.0.8059.79 | 72/72 | 72/72 | 4/4 | 81/81 | 123 | PASS / 0 |

Both completed captures have zero strict receiver packets, zero packets to the
probe destination, zero kernel capture drops, zero DNS sends, unknown destinations,
unidentified senders or incomplete traces, and no send/write on a probe socket.
Isolation/native input/compilation, source binding, signature-before-execution and
independent launch controls pass. The 57 host regressions pass, including 30
shared-assessment route-probe cases and a real offline pcap-reader regression.
The initial fail-first route tests rejected both permitted connect results on
the base while retaining all 24 forbidden variants; the final suite adds four
duplicated-descriptor write variants. Process-signal lint passes.

Evidence root: `/root/.codex/evidence/browser-resume-20260930/appview/r6/`.
Completed runs are `verified-15539/` and `verified2-15579/`. Their `results.json`
and `attribution.json` retain all 252 exact exemptions (PID/TID, destination,
socket identity, result and source trace); `packet-assessment.json` retains wire
counts, phase labels and capture integrity. `FINAL_RESULTS_IDENTITY.json` joins
source, browser, asset and per-capture identities. Each run's `steps.tsv` records
all build/test/browser steps exiting 0. Screenshots, console, PID maps, NetLogs,
syscalls and pcaps remain alongside the results.

Two attempts are explicitly excluded: `final-15539/` stopped at the offline
pcap reader's host confinement error after rendering; `verified-15579/` was
interrupted with SIGTERM before final assessment. Neither is acceptance evidence.
The reader was fixed with a fail-first regression by feeding capture bytes via
stdin, without changing evidence permissions or the gate. A fresh detached full
.79 run completed. `EXCLUDED.md` in each failed directory preserves the details.

Historical round 3/4 results below retain their original, stricter gate labels;
they are not relabeled as new runs or evidence of acceptance.

From the paired OSS checkout, one command installs pinned Node dependencies,
builds both references, tests the verifier and Cloud response, builds the real
Cloud web entry/assets, runs Chromium checks and cleans its processes:

```sh
bash packages/app-view-host/test/run-av1.sh /root/work/agent-appview-cloud \
  /root/.codex/evidence/browser-resume-20260930/appview/r6-reproduction \
  /root/.chariox/dev/av1-reproduction
```

Use new evidence/scratch directories on each run. Keep the scratch path short:
Chrome needs a TMPDIR of at most 60 bytes for its Unix socket. Prerequisites are
Corepack/pnpm/Node22, Chrome, strace, tcpdump and the lane's bounded-workload wrapper.

Round 4 completed captures are in `appview/r4/final-15539/` and
`appview/r4/final-15579/` beneath the evidence root above. Tested source is OSS
`f68df518db` and Cloud `e52b13392a`; both runs use the same hashed built assets.
Each passes 72 frame/policy checks, 72 positive controls and four reference
renders. Each includes all 81 strict captures and exits 1 solely for the
whole-browser zero-network gate. Their 130/131 non-loopback operations are the
Network Service IPv6 route connects; DNS sends, unknown destinations,
unidentified senders and incomplete traces are zero. `FINAL_SOURCE_IDENTITY.json`
and `FINAL_RESULTS_IDENTITY.json` retain source, assets and per-capture results.
The earlier `r4/complete-15539/` run was interrupted before its final assessment
and is explicitly excluded. Host regressions pass 26/26 and affected Cloud
response/dev-server tests pass 2/2, with fail-first logs for both review fixes.

Dependency installs use frozen locks without dependency install scripts; the
needed web/reference build commands run explicitly. `CHARIOX_TEST_CHROME`
selects another installed browser; `CHARIOX_BOUNDRUN` selects the wrapper on a
different builder. The expected browser exit is **0** only when the full gate with the named
route-probe exemption passes; the command records each step's exit status in `steps.tsv`. A nonzero
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
The final OSS runtime/test head is `e6e204fc6a364256ed4e744d11d7ccd1325bdb75`;
Cloud is `8d6c93365fff491fe7c8713abb476e41ab11c121`. On each engine the
complete result is 72/72 frame/response-policy checks, 72/72 positive controls,
4/4 reference renders and passing auxiliary checks. All 81 strict browser
captures have zero DNS/unknown/incomplete/unidentified operations; only the
specified browser route socket remains. Both browser commands exit 1 solely
for `whole-browser-tree-zero-network`. At that time no narrower gate was accepted.
`ENFORCEMENT_IDENTITY.json` records exact source/artifact hashes and results.
`ATTRIBUTION.md` maps the seven baseline classes to actual renderer/PID/TID and
destinations. `enforce-complete-15579/` repeats the frozen assets on the newer
engine. `all-write-idle/` additionally traces write/writev/sendfile/splice in an
idle browser without App content, to cross-check the route probe's lack of data.

Receiver identity and test phase are separate. The independent CSP-removal
positive control deliberately uses the same receiver as its strict pair.
Consequently `strictReceived` includes its one `/launch-frame-csp-control` hit;
this is labeled as a control phase. Every actual strict phase has zero receiver
traffic. Per-case rows and `launch` results preserve that distinction; the gate
does not silently ignore a strict request.

Historical round 3/4 checks and their evidence:

| Check | Evidence |
| --- | --- |
| Signature/hash/entry tampering, Rust signing vector, Unicode and size limits; full inventory with portable license names | `verifier.log`; `verifier-before.log` / `verifier-after.log` and `inventory-before.log` / `inventory-after.log` in the evidence root for resumed fixes |
| Cloud static response policy, reports removed, malformed host fails clearly | `host-response.log`; `response-before.log` / `response-after.log` at the root |
| 36 network/navigation/fresh-document classes at each DPR, receiving controls and strict header/sandbox audit | `results.json` → `attacks`; named files under `netlog/`; `console.json` |
| Whole browser tree, actual frame/renderer PID, requested flags/effective profile, complete socket syscalls and raw zero-network result and named-exemption assessment | `process-tree/<case>/pid-map.json`, `frame-processes.json`, `launch.json`, `syscall.<tid>`, `network.json`; `attribution.json` |
| Independent resolver/direct-IP and frame-CSP controls; idle browser with no App | `results.json` → `launch`, `process-tree/launch-*` |
| Capture parser rejects DNS, unknown senders, descriptor reuse, batched destinations and incomplete captures | `verifier.log`; root `enforce-gate-before.log`, `enforce-gate-after.log`, `enforce-completeness-before.log`, `enforce-all-unit.log` |
| Data/blob-only enforced response policy, old unsafe-inline policy fails first | Root `enforce-policy-before.log`, `enforce-policy-after.log` |
| Separate HTTP/TCP/UDP receivers and per-phase connections; this drill's DNS names and the probe address captured | `receiver.pcap`, `receiver-ports.json`, `capture-filter.txt`, `results.json` → `receiver`, `tcpdump.log`, `capture-process.json` |
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
| AV1 Web boundary | PASS on both pinned built-Cloud matrices WITH the named connect-only probe exemption and zero strict wire packets. Accepted only as the AV1 renderer/security proof. |
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
