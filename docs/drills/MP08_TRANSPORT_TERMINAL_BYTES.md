# MP-08 / MP-09 / MP-10 / MP-11: terminal bytes over the relay

`transport/TR-11` previously encrypted JSON number arrays, multiplying terminal
bytes even when the outer relay frame was binary. Local protocol 1008
(provisional) uses CXT1 metadata and raw bytes for live and replayed terminal
output. Record ids, history sequences, recipient filtering and encryption do
not change. Kernel gateways restore the ordinary record shape for their caller.
The native client negotiates the relay's existing binary capability and accepts
raw encrypted events and replies, including chunked delivery. Otherwise base64
JSON envelopes still exceed the terminal byte budget despite CXT1 plaintext.

Focused regression checks:

```sh
pnpm build:kernel-client
node --test packages/kernel-client/dist/browser-relay-crypto.test.js
node --test packages/kernel-client/dist/ipc-relay-binary.test.js
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel --lib terminal_output_is_raw_on_live_and_replayed_encrypted_relay_events
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel --lib terminal_output_wire
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel --lib terminal_events_restore_the_ordinary_record_shape_at_the_gateway
```

The encrypted emission/replay test fails against the preceding kernel: the
decrypted terminal payload is JSON rather than CXT1. The client tests fail
against the preceding clients because their encrypted event receivers cannot
decode CXT1. These are fixture regressions, not live acceptance.

For live acceptance, pair a built TUI and the hosted browser client with a real
kernel through its own hosted WSS relay. Shape the client link to 8 Mbit/s and
at least 60 ms RTT. Run the official native provider terminal for 60 seconds
with `seq` and `ls --color` output while another agent pane is idle. Repeat in
the browser at DPR1 and DPR2. Record terminal byte counts at the source and
relay byte counts at the viewer, retaining captures and logs outside the repo.
Require at most 1.4 relay bytes per terminal byte, intact record order and
content, and no streaming while the terminal pane is hidden. Disconnect and
resume the viewer to verify replay also restores exactly the same records.
Record every missing dimension explicitly; fixture checks do not establish
provider execution, hosted traffic, pane visibility or the full parity matrix.
