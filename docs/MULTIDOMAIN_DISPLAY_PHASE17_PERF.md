# MP-08/MP-10/MP-11: phase 17 software display performance

Assigned base `f5d3b1da96e06a57a6f4b7b590abbbd2bb958885`, branch
`md/display-perf`. Local protocol447 / relay90 are unchanged. Work in progress;
no acceptance claim. Evidence:
`/root/.codex/evidence/browser-resume-20260930/display/phase17/`.

## MP-08/MP-10: source cadence and measured scope

The fresh base on a verified60Hz scroll source presents53.07fps and consumes
1.99 source-plus-pipeline cores. Pacing readback starts rather than completion
raises this to56.50fps, at2.05cores; concurrent independent row decoding reaches
58.28fps in an instrumented run. These are component experiments, with a real
encrypted local relay and optimized kernel libtest executable, not the real web
app. The final comparisons below must use their own clean source identities.

`scroll30` actually generates30updates/s (phase16:302updates/10.067s).
`video` plays a30fps canvas captureStream. A60fps presentation count on these
sources includes repeated/compression-changing pictures. The source-rate60Hz
row is a separate diagnostic; do not create duplicate motion packets merely to
match Selkies presentation counts. Idle media must remain zero and quiet settle
must remain RGB exact. The wheel source also needs separate delivered input and
source update counts; its30Hz offered input is not evidence of60unique pictures.

## MP-08/MP-10: interface handoff for the Cloud lane

The supplied Cloud `f6cfd0066d75844dbab795cdf715789ec5fa37d6` has the older
Room `chariox-display-v1` socket client. It lacks the kernel-browser447 presenter
integration and `scripts/e2e-stack`. The coordinator must supply the real app
integration source and harness before the required real-app red/green drill.
No Cloud checkout is modified by this lane.

Cloud integration needs only the following existing interfaces:

1. Bundle `apps/browser-display/presenter.mjs`, `stripe-presenter.mjs`,
   `decoder-worker.mjs`, `tile-cache.mjs`, and `scroll-prediction.mjs` from this
   OSS source. The decoder worker must resolve as an actual module-worker asset
   from the built app. WebCodecs requires a secure context (localhost is allowed).
2. Call `attachBrowserDisplay(canvas, transport, {tab_id,generation}, options)`
   after the real user-domain browser UI has opened/selected the kernel Tab.
   `transport.request({KernelBrowser:{command}})` resolves the normal encrypted
   kernel reply `{KernelBrowser:{result}}`. `onEvent(listener)` delivers the
   decrypted `kernel_browser_frame` event and returns its unsubscribe function.
   Keep it separate from durable terminal replay; display sequence is transient.
3. Implement `transport.subscribeDisplay(binding)` / `unsubscribeDisplay(binding)`
   with existing relay `client_subscribe` / `client_unsubscribe`,
   `subscription_scope:"kernel_browser_display"`, session_id=subscription_id,
   attachment_id=String(generation), and the existing client public key/target.
   Use the normal authenticated bootstrap and opaque encrypted relay transport.
   Cloud supplies no capture, media proxy, session state or input authority.
4. Require local protocol>=447 for this view and offer `chariox-stripes-v1`,
   `chariox-video-dependencies-v1`, a supported codec and PNG through the helper.
   Start with deviceScaleFactor:1;1920x1080 only supportsDPR1. Do not mutate the
   Tab viewport on resize outside the existing kernel negotiation. Match kernel
   `CHARIOX_KERNEL_BROWSER_DISPLAY=1`; leave production feature opt-in and use
   the real app's documented feature flag in the acceptance command.
5. `stream.start()` runs bounded credits; `stream.input()` sends observed-document
   bound click/text/key/scroll in CSS coordinates. Bind actual UI mouse/keyboard
   events and human takeover/release to these methods; actors come from
   `stream.actors()`. `onPresented(frame)` fires after validated atomic draw.
   Render errors/retry to the user. Retry closes the old stream and subscribes
   afresh; never replay a lost display_next credit. Await `stream.close()` on
   navigation/unmount/disconnect. Avoid simultaneous `next()` and `start()`.
6. Supply the real app entry/built bundle URL and flags to the shared e2e harness,
   real kernel/relay/CLI binaries, and public fixture origin. Drive click/type,
   source60Hz motion, exact settle, idle, unsupportedDPR, navigation, takeover,
   masking and dropped row-reference recovery through Playwright UI input.
   Run base RED and candidate GREEN with screenshots/console/log captures under
   this lane's evidence directory. A fixture HTML entry or direct IPC-only run
   cannot satisfy this gate. A provider is required for agent/takeover behavior.

The serialized shapes, credit/refusal rules and source/Vault/observation fences
remain the447/90 contract. No new protocol allocation is requested.

## MP-10 owner questions

The encoder default remains owner-selected. x264 and Cisco OpenH264 screen mode
will both be measured. Builder2 has no GPU; hardware comparison needs the owner
laptop. Missing real-app integration/harness blocks only real-app acceptance,
not profiling, fixes, focused tests or the software comparison.
