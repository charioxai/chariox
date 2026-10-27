# MP-08 / MP-10 / MP-11: initial terminal view drill

`AttachSessionView` uses the ordinary kernel admission, attachment, state and
history paths. With protocol 518/126, it returns caller-visible state and the
latest turn for the selected six-pane desk, with titles/previews and no tool or
thought body download. State and history reads run concurrently after attachment.
The event subscription remains the ordinary subscription. Protocol 508 is the
minimum for the bundled request; 508 kernels retain the earlier four-turn
bundle. Protocol 518 adds scoped `GetClientCatalogRevisions` replies. Earlier
clients retain their ordinary attachment/history paths.

Use the exact built web entry, compiled Chariox TUI, kernel and relay, with a
real product-linked provider. Record source commits, binary hashes and build
commands. Run the browser at DPR 1 and 2 over hosted WSS on a real WAN with
approximately 8 Mbit/s client uplink and at least 60 ms RTT. Keep screenshots
and logs outside the source checkout. Capture only request/response names,
timestamps, counts and sizes from transport instrumentation, never grants or
payload contents.

1. Through the real composer, review several actual project files with separate
   tool calls and a final assessment. Keep both completed and active turns in
   the room. Repeat with short, medium and long histories.
2. Hard reload five times. Record navigation-to-first-entry and input-to-visible
   timings, false first-prompt states, long tasks and request names. The bundled
   path must issue one `AttachSessionView` for initial attachment, state and
   visible history. A separate initial state or outline request is a regression.
   Record unrelated background requests separately. On medium and long rooms,
   require the initial response <=64 KiB and kernel handler <=50 ms. Report active
   native and settled opens separately; their difference must be <=100 ms.
   The attach request must begin by DCL + one measured wire RTT (and <=0.7 s),
   with a settled first row <=1 s over WAN.
3. Switch panes five times per agent using real pointer input. Open an individual
   tool body on its first click, reload, and verify the body and disclosure state.
   In the default TUI selection mode use Ctrl+O. With the supported
   `CHARIOX_TUI_MOUSE=on` mode, also exercise individual-row pointer disclosure.
   Compare the visible order and entry identities with the kernel's ordinary
   history projection; counts alone do not prove content or ordering.
4. Attach the compiled local and remote TUI while the provider is producing
   output. Start the latency clock before invoking the client. Verify buffered
   output reaches the live pump after bundled history and that replay does not
   duplicate it. Finished TUI turns stay expanded.
5. Repeat with more than six agents and a preferred agent on a later desk.
   Switching to another desk must load its ordinary history. With a second
   legitimate user who is not a room member, verify denial creates no attachment.
   External-agent grants must not obtain this terminal view.
6. Warm reload the same signed-in session. Verify the private cached view paints
   by DCL +50 ms (web), and warm-open P95 <=800 ms WAN / <=300 ms local. Feed the
   order/full-answer oracle from a nonempty normal kernel outline on every run.
   Record cached and canonical screenshots, false-empty and blank-frame counts,
   and sequence reconciliation. Paging must continue before any restored older
   prefix, without duplicates. Clear the view on sign-out/account switch; stale
   cache writes must not restore a previous principal. Cached controls remain inert.
7. Open the real composer: an existing catalog appears immediately. After its
   four-hour TTL or explicit provider/account invalidation, record the ordinary
   scoped revision request. Unchanged model/account/command inventories must not
   be downloaded again; changed inventories reconcile in the background. Record
   older-kernel fallback separately, and never log credentials or grants.
8. Exercise reconnect and a multi-hour session. Record socket close reasons and
   timestamps against renewal attempts; preserve failures rather than hiding
   them with reloads. Require the lane's full usability and provider matrix
   before declaring acceptance. Run one final >=3 h session on the final head.
   For echo, report measured TCP/wire RTT alongside P95; require P95 <=RTT +60 ms.
   Loaded-builder measurements are diagnostic, not quiet-host timing acceptance.

Keep a fail-first run from the parent source: initial attachment, state and
history previously required separate client flights. A one-flight result proves
the request-path improvement, not cold-entry latency, stability, managed parity
or the MP-11 security-anchor review. Focused admission, preferred-desk, replay,
protocol snapshot/hash and client bootstrap tests are supplementary checks.
