# MP-08 / MP-09 / MP-10 / MP-11 — signed client view delivery (AV2)

This first slice delivers a v2 release's signed, self-contained reference entry
over the client's existing authenticated kernel connection. Local999 / peer438
are reserved for this slice. It introduces no App-open cutover or backend bridge.
The coordinator's round7 interaction explicitly defers reference execution and
backend semantics to AV3: both real reference Apps require backend startup calls.

`ClientAppView` is a terminal-owner request family:

- `Open { installation_id }` verifies the active stored v2 package against current
  enrolled trust. It creates a fresh view ID bound permanently to that installation,
  exact generation and delivered package digest, and returns canonical signed
  manifest/inventory/signature plus the authenticated publisher key/content digest.
- `Read { view_id, offset, max_bytes }` reads only the signed entry, never a caller
  pathname, backend file or arbitrary RPC. Raw chunks are at most64KiB and the
  complete entry is at most16MiB. Every read checks owner, active generation,
  release digest and current trust through the existing durable writer, verifies
  the held release's exact file bytes, and repeats admission after the read.
- `Check { view_id }` rechecks that same binding without changing its generation.
- `Close { view_id }` releases the delivery lease. Each lease also expires after
  60seconds; its timer is cancelled on early close, keeping tasks bounded alongside
  the registry. Limits are32 views per owner and128 per kernel, with the existing
  bounded App blocking-I/O admission. No worker or managed activity fence is held.

The shared client loader reads serially, checks chunk sequence/length/EOF and
validates the publisher signature and complete entry hash before returning any
mountable bundle. Its memory LRU holds at most256MiB, keyed by the authenticated
kernel/owner, installation, release digest, path and hash. Cache reads are copied
and rehashed; corrupt entries are evicted. Even a cache hit requires a fresh
Open descriptor and final Check. Partial or mismatched transfers close their
lease and never return executable code. The Cloud build pins the shared loader
and emits its standalone asset beside the AV1 host; its Web adapter supplies the
real connection's authenticated owner key and rejects connection changes.

No legacy renderer fallback is added. The old App-open path remains until its
explicit later cutover slice. A delivery lease ends at transfer completion; full
live page revocation, replacement staging, backend ports, TUI hosts, realistic
network/performance/soak and ordinary-versus-managed acceptance remain later work.
The new path cannot mutate backend state or extend managed idle activity.

Focused regressions cover canonical wire shapes/hash and owner-field rejection,
real signed controls and bounded reads, close/expiry/uninstall/trust revocation,
terminal-owner router admission and agent refusal, chunk corruption and cache
scope/bounds. They supplement the live drill, not replace it.

The reproducible live entry is Cloud's
`apps/web/scripts/app-view-delivery-live.mjs`. It boots the exact supplied kernel,
pairs it normally using the public device approval mutation as setup, installs a
real signed reference package, and imports the production Web client and delivery
adapter into the actual built hosted waiting-room page. Asset bytes travel only
through that client's encrypted hosted WSS connection. DPR1/2 verify signature,
entry hash,64KiB bounds, fresh authorized cache hits and rejection after Close.
`--base` proves that the same real path rejects the new request before the change.
The helper serves only test entry code, never App release bytes or kernel keys;
all owned processes/routes are closed in finally. This is an asset-transport
exit, not reference rendering, pairing-UI or full App-open acceptance.

## Round7 validation receipt

The final real-kernel run delivered the signed Excalidraw entry at DPR1 and2:
7,788,348 bytes,119 chunks per cold load, each <=65,536 raw bytes. Both rows
verified signature and file hash, obtained a fresh authorized descriptor without
extra chunks on a cache hit, and rejected reads after Close. The exact base
rejects the new request over the same hosted WSS at both DPRs; the normal signed
installation/delivery workflow on that base also fails at the new feature gate.
The final candidate's helper route, browser, kernel and owned process scope all
close cleanly. Evidence: `appview/r7/av2-live-green4/results.json`, base4 and
base-standard receipts, and `AV2_IDENTITY.json` under
`/root/.codex/evidence/browser-resume-20260930/`.

The built waiting-room page hosts the production Web transport adapter in this
drill. Its normal shell remains Connecting; it does not demonstrate a connected
Room, normal App-open UI or reference execution. Pairing setup uses the normal
public approval POST because the pinned validation API predates the current
consent-details UI route. That API image and the exact current Web assets are
recorded separately. These limits do not turn this transport-only exit into
App-open or ordinary/managed usability acceptance.

Validation includes72 host tests (including30 route-probe tests),34 package v2
tests,5 kernel delivery tests,322 protocol shape tests,4 client conformance tests,
Web build/lint, shared-source pins and focused Clippy on changed Rust lines.
Runtime source is pinned at d9ff1ac758; subsequent Rust changes are test-only.
