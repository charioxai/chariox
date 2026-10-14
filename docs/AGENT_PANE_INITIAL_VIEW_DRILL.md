# MP-08 / MP-10 / MP-11: initial terminal view drill

`AttachSessionView` uses the ordinary kernel admission, attachment, state and
history paths. It returns the caller-visible state and four recent turns for
the selected six-pane desk, including the existing bounded content prefetch.
The event subscription remains the ordinary subscription. Protocol 508 is the
minimum for this bundled request; older clients retain their existing path.

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
   Record unrelated background requests separately.
3. Switch panes five times per agent using real pointer input. Open an individual
   tool body on its first click, reload, and verify the body and disclosure state.
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
6. Exercise reconnect and a multi-hour session. Record socket close reasons and
   timestamps against renewal attempts; preserve failures rather than hiding
   them with reloads. Require the lane's full usability and provider matrix
   before declaring acceptance.

Keep a fail-first run from the parent source: initial attachment, state and
history previously required separate client flights. A one-flight result proves
the request-path improvement, not cold-entry latency, stability, managed parity
or the MP-11 security-anchor review. Focused admission, preferred-desk, replay,
protocol snapshot/hash and client bootstrap tests are supplementary checks.
