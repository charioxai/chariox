# MP-08/MP-10/MP-11 Room concurrency and placement validation

This is the b216 local ledger and remote repetition runbook. It supplements
Drills C/E and the six-row placement matrix in
`BROWSER_COMPUTER_USE_END_TO_END_PLAN.md`. Source/helper tests and one local
run do not close MP-08, MP-10 or MP-11. The frozen runtime source is
`9334141d420f8a32393f206102c5b8b4a1b0b609` (local 411 / relay 70).

## MP-08/MP-10/MP-11 b216 results, 2026-10-04

Evidence is retained outside Git under
`/root/.codex/evidence/browser-resume-20260930/b216/`. The test artifact was
compiled in the private `cargo-target-b216` directory, with `fresh=false`,
SHA-256 `a62fc4ae1b25565a7ee418458ddb516a0ccc046549c6c74d954cc08d2a70b49f`.
Harness head `e60568c1472393e8a05afb5123ba0d83d3ea17cd` has unchanged Rust/Cargo
inputs relative to the frozen runtime. The artifact, harness and image are
separate identities in the receipts.

- MP-08/MP-10/MP-11 focused Node fixtures: 61/61 PASS, zero skips. These cover
  evidence validation, placement configuration/owned cleanup, TUI notices,
  stdio ordering/cancellation and exactly-once promotion after failure.
- MP-08/MP-10/MP-11 focused Rust artifact: 18 unique checks PASS. The first
  permission check aborted on Linux's default test stack; its recorded
  32 MiB-stack retry passed. The two-client credential-interaction test and
  the Claude permission bridge are separate proofs, not one live three-client
  permission drill.
- MP-08/MP-10/MP-11 live timing: GREEN with 51 ms physical read overlap,
  112 ms different-tab fill overlap, serialized same-tab fills, two exact
  takeover cancellations, 12 terminal Actions and selected-Tab retention.
  Container/listener/process cleanup is GREEN. The first attempt aborted on
  the debug test-thread stack before controller assertions; retain that RED
  result alongside the GREEN retry. The runner now records and applies a
  bounded 32 MiB `RUST_MIN_STACK` for this large debug test.
- MP-08/MP-10/MP-11 image provenance: all 52 `browser*.mjs` files in the
  lane-created image match frozen G2 bytes. The substrate remains F image
  `sha256:e76b80392f3368efaceed6e6636cc4d736fe57c25be0206ca4cf8f132a168f64`;
  overlay image is
  `sha256:b406939e827fc99c176ce76bcbadd0aa9ea1473141ab7a8ff4d2b0e09c1e04a8`.
  This does not prove a complete G2 image, renderer sandbox, production launch
  or signed release. No display-backend implementation was changed.
- MP-08/MP-10/MP-11 cleanup: the exact lane-created image, disposable test
  state and private Cargo target were removed after ownership/process checks.
  No real provider profiles were used. Periodic receipts include 96 samples;
  the earlier `free -h` compile observation reached approximately 33 GiB
  available memory. All observations stayed above the 16 GiB floor. Final
  available memory/disk are 50.369 / 227.228 GiB. Full frontend/provider/hosted
  acceptance is still open.

| MP-08/MP-10/MP-11 placement row | b216 result and limit |
| --- | --- |
| Home Environment / home agent | PASS: `room_home_local_slice` public Browser/Computer/Action/display fixture; simulated physical slice |
| Home Environment / different home slice agent | PASS: `room_slice_cross_placement` home admission and distinct worker/membership records; OS isolation unproven |
| Home Environment / remote worker agent | NOT RUN as an exact physical placement; configuration/evidence negatives pass, no substituted topology credited |
| Remote Environment / home agent | PASS: `controller_worker_mcp::home_room_agent_uses_remote_environment_worker_browser_and_web_view`; real same-host relay, synthetic controller/display/provider |
| Remote Environment / Environment-worker agent | PASS: `controller_worker_mcp::room_browser_on_environment_worker_serves_same_worker_agent_and_web_view`; home admission and wrong-Room/forged-lease denial |
| Remote Environment / different worker agent | PASS: `controller_worker_mcp::room_browser_on_environment_worker_serves_remote_agent_and_web_view`; home authority and wrong-Room/worker denial, no physical remote host |

MP-08/MP-10/MP-11 client ledger: the TUI notice fixtures and public Action/Tab
projections pass, as do authenticated takeover/release, relay Computer
cancellation, wrong-worker/stale-binding denial and first-reply-wins shared
interaction fixtures. Live rendered Web/local/remote TUI, one shared permission
interaction across all three, reconnect and official3-provider realism are
NOT RUN. No product-linked official3 profiles or client frontend were supplied
to this lane; the bootstrap agent profile was not used. Managed repetition
awaits an owner-issued VM path. Host-browser focus-scoped access awaits the
multidomain contract; no membership-derived user-domain grant was added.

## MP-08/MP-10/MP-11 local evidence boundaries

| Seam | Local proof | Remaining acceptance |
| --- | --- | --- |
| Concurrent reads, same-tab serialization and different-tab mutations | Timing drill uses three synthetic Room agents and the real controller/CDP, with page-acknowledged gates | Official-provider realism is separate; admissions alone do not prove queued promotion |
| Queued promotion | Held fill, one queued same-tab fill, independent-tab fill; first completes before queued fill starts, then final DOM values are checked | Repeat on the reviewed complete Environment and managed machine |
| Immediate takeover | Kernel takeover cancels active and queued same-tab actions; independent action survives; exact physical effects are checked | Web-triggered takeover and all-client projection |
| Selected Tab | Explicit activation precedes background actions; final kernel focus must retain the same stable Tab | Web/local/remote TUI visual observation |
| Placement authority | Focused router tests use a real same-host relay, home admission, leased identity and public Room/Action/display bindings | Synthetic provider/controller/display fixtures do not establish physical cross-slice isolation or a rendered Web View |
| Foreign Room and forged lease | Home-admission negative fixtures must leave Tab and Action history unchanged | Repeat each placement with official agents and a fresh managed VM |
| Client and permission authority | Kernel owns Tab/Action/Room and RuntimeInteraction; TUI notice fixtures consume shared events | One live permission interaction observed and answered across Web/local/remote TUI, with reconnect |
| Host-browser user domain | Outside the frozen slice-backed Room contract | Focus-scoped user-domain authorization must be supplied by multidomain work; Room membership alone grants no host-browser access |

MP-11 source inspection: Room placement and worker-MCP admission use the same
runtime path regardless of managed placement. This bounded inspection does not
approve every managed branch or the historical inventory. Do not introduce a
clone browser, worker-owned Room authority or another relay/session authority.

## MP-08/MP-10/MP-11 reproducible local timing

Use a clean lane checkout and a private target directory. On builder2, every
Cargo invocation uses the coordinator's compile lock, four jobs maximum, and
the assigned memory floor of 16 GiB. Sample `/proc/meminfo` and `/` before and
during compilation; settle only owned work if headroom approaches the floor.
Build `cargo test -p chariox-kernel --lib --no-run --locked --message-format=json`
under `flock /root/.chariox/dev/browser-resume-20260930/locks/rust-compile-1.lock`.
Keep Cargo stdout/stderr and identify the emitted test executable from its
`compiler-artifact` record. Retain source commit/tree, command, exit, executable
SHA-256, exact ignored symbol listing and resource samples before deleting the
private target. A matching protocol number is insufficient provenance.

Run with absolute paths and an engine-resolved image ID:

```sh
node apps/cli/scripts/live-browser-controller-concurrency-drill.mjs \
  --test-binary "$B216_TEST_BINARY" \
  --test-binary-sha256 "$B216_TEST_SHA256" \
  --memory-floor-gib 16 \
  --image "$B216_IMAGE_ID" \
  --output /root/.codex/evidence/browser-resume-20260930/b216/timing
```

The pin prevents accidental executable replacement; the exact-symbol preflight
rejects zero tests before provisioning. Neither attests how the binary was
built. Preserve the build receipt separately. If a component-only image overlays
G2 controller files onto F, record both identities and copied-file hashes; do
not call it a complete G2 image or signed-release acceptance. Remove only the
lane-created image after verifying its exact ID and ownership labels.

## MP-08/MP-10/MP-11 placement and client repetition

| Environment | Agent | Required route |
| --- | --- | --- |
| Home headed slice | Home kernel | Agent runtime tools → home admission → bound Environment |
| Home headed slice | Different home slice | Leased agent → home admission → browser slice, without direct file/process access |
| Home headed slice | Remote worker | Authenticated active lease → home admission → home Environment |
| Remote headed slice | Home kernel | Home admission → Environment worker |
| Remote headed slice | Environment worker | Worker forwards to home admission even when execution is colocated |
| Remote headed slice | Different slice/worker | Agent worker → home admission → Environment worker |

For MP-08/MP-10/MP-11 remote repetition, the coordinator must supply an
owner-issued disposable VM through the reviewed normal product provisioning
path. Do not contact the reserved relay or Apps machine. Use a reviewed signed
aggregate, with Cloud and native/Web protocol minima bound to that release;
never allocate protocol numbers in a lane. Verify ordinary/managed source and
release receipts before comparing results.

1. Create lane-owned Rooms/slices with public product commands and persist a
   private ownership ledger outside repositories. Use
   `live-room-placement-setup.mjs --create` and the canonical six-row config.
   Use product-created leases/identities and Chariox-linked accounts only. No
   manual credential transfer, bootstrap agent profile or raw account-store
   inspection. Configure the checkout's Docker slice provisioner explicitly.
2. Attach Web, local TUI and remote TUI to each home Room through the ordinary
   transport. Record the same Room, Environment, generation, stable Tab, Action
   IDs and authoritative actor IDs. Capture safe UI projections, not config,
   account files or complete connection envelopes.
3. Run provider Browser and Computer actions with
   `live-room-placement-matrix.mjs --execute-live --config <private-config>`.
   Its PASS leaves its explicit `unexecutedGates` open, including rendered Web
   parity, takeover, ordering, reconnect and forged-lease denial.
4. Run mixed Codex/OpenCode/Claude Drill E on official harnesses separately.
   Retain provider identities, prompt/Action IDs and timing. Show a queued
   mutation actually promotes after its predecessor; an `allStarted` admission
   barrier or successful prompt submission is insufficient. Use deterministic
   controller timing for physical overlap and cancellation claims.
5. Run Drill C unchanged: a TUI prompt drives the same Web-visible browser;
   Web takeover appears as one human actor and ownership change in both TUIs.
   Trigger one provider permission request. All clients must observe the same
   kernel RuntimeInteraction ID; answer once, then prove one resolution and
   no duplicate interaction on reconnect. Disconnect/reconnect each client and
   retain provider run/thread, Room, Environment and Tab identities.
6. For every placement, try a wrong Room, forged/stale lease, wrong worker and
   substituted Environment. Assert denial before browser effects or Action
   history mutation. Computer/display and OS isolation need independent proof;
   a Browser-only pass cannot substitute. In the user domain, additionally
   prove focus-scoped authorization and deny a mere Room-member request.
7. Clean up through the invocation-owned ledger on success, failure and
   interruption. Dynamic-port collisions recreate only the owned slice, at
   most three retries. Assert exact owned processes/listeners/containers,
   volumes, disposable identities and heartbeat targets are gone; retain only
   safe evidence and explicitly protected provider assets. Record final
   memory/disk samples. Never prune shared Docker state or reviewer services.

MP-08/MP-10/MP-11 builder1-only campaign receipts are unavailable on builder2.
They remain coordinator inputs, not local validation. The owner-issued VM,
product-linked official3 profiles, client frontend and multidomain focus grant
are prerequisites for the affected rows only; continue independent local work.
