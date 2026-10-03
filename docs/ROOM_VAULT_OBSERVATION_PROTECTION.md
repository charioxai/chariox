# MP-08 / MP-10 / MP-11: Vault observation protection

Coordinator provisional decision #8 (2026-10-03) selects known-value redaction
by default, with fail-closed observations when masking cannot be guaranteed.
This implements the shared kernel/controller contract for Browser and Computer;
managed placement selects no different policy. It does not close an MP item.

The home kernel and bound worker register a resolved value before physical
input, including input that subsequently fails or is cancelled. Registration
persists a private, value-free quarantine marker before input is dispatched.
The values remain in zeroizing Room-scoped memory. Values are never stored in
the marker, diagnostic messages or interaction. Navigation, a failed insertion,
and human clearance do not discard the known-value registry.

MP-08/MP-11 redaction covers exact values, lower/upper case, URI/JSON/HTML
escaping, Base64/Base64url and hexadecimal forms. The controller scrubs raw CDP
snapshot strings before compaction, including copied accessibility text, DOM
strings and isolated-frame snapshots. Its event journal and response envelope
are scrubbed too. Both kernels scrub controller responses before Room projection
or relay return. The authenticated tool boundary scrubs results and sensitive
errors; terminal fanout, shared history append, Recall and provider-switch
context reconstruction also protect text before their output bounds. A bound
slice maps its worker-local provider session IDs to the provisioner-authorized
home Room registry. Forwarded MCP/script/connector results are scrubbed before
caching, result auditing and relay return. Recovered cached results cannot be
replayed after clearance; callers must request a fresh invocation.

Unframed terminal byte streams are withheld for the protected session lifetime,
even after live-view clearance: chunk-local matching cannot safely redact a value
split across two chunks. This limits terminal visibility after Vault insertion.
Complete tool results and framed history records retain known-value text
redaction; streamed assistant/reasoning history is withheld as well.

MP-08/MP-11 pixels are withheld after any insertion: arbitrary page handlers,
canvas, native unmasked fields and delayed echoes make a field rectangle
insufficient to guarantee a safe image. This implementation uses the fail-closed
fallback instead of claiming that a field-only mask covers those renderings.
OCR/find-text observations are also withheld because their source pixels cannot
be guaranteed safe. Text Browser observations remain usable with redaction while
the registry is available. Room locks exclude physical input during capture and
storage; unrelated Rooms retain independent locks.

The next affected agent observation requests one ordinary, critical kernel
`RuntimeInteraction`. The user must remove sensitive content from all Room tabs
and desktop windows, then explicitly resume observations. Denial, dropped
interaction or the 30-second timeout keeps observations withheld. Home clearance
holds the exclusive Room fence through the interaction and the authenticated
bound-worker clearance, so another insertion cannot race an old approval.
New insertion reinstates the pixel fence. The human live viewer remains the
place to inspect and clear the view.

MP-08/MP-11 restart and restore do not reconstruct plaintext from persisted
markers. Previously existing Rooms and restored markers require fresh human
clearance for text as well as pixels. Replacement controllers require clearance
when they have lost their pre-compaction registry. Old screenshot artifacts
remain unreadable after clearance: reads and OCR require the current worker
epoch and observation revision. Recovered history content and references from
before this kernel lifetime are withheld before Recall/handoff reconstruction.
Clearance of today's view is not approval to replay an old image or transcript.

MP-08/MP-11 local protocol 378 / relay peer 69 adds the home-authorized
`clear_secret_observation` controller command and `secret_observation_cleared`
response. Existing clients use their existing interaction projection and reply;
there is no client-side clearance authority or client minimum-version change.
Protocol snapshots hash both new wire shapes.

MP-10 focused source checks:

```sh
node --test apps/kernel/slice-linux-docker/docker/browser-controller-snapshot.test.mjs
flock /root/.chariox/dev/browser-resume-20260930/locks/rust-compile.lock \
  cargo test -p chariox-kernel --lib room_secret_observation -- --test-threads=1
```

Run with the reserved builder toolchain, four Cargo jobs and an explicit,
lane-owned `CHARIOX_HOME` outside the repository. The fail-first controller
fixture reproduces vaultg's copied AX value and tests pre-compaction protection.
Rust coverage exercises Room isolation, restart, denied/approved clearance,
durable-marker failure, worker session mapping, split terminal chunks, cached
replay refusal and the capture/input fence. Evidence stays external.

MP-10 acceptance still requires exact signed aggregate replay of vaultg's Browser
text/OCR/history/database disclosure, malicious pixels, Computer approval,
restart/restore and old artifacts, then independent review and fresh-machine
ordinary/Path-1 comparison. Source checks establish only the paths they execute.
Provider-native tools or provider-owned historical contexts that bypass these
kernel-owned observation APIs require separate acceptance evidence; this source
change does not inspect or rewrite protected provider account state.
