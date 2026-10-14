# Relay token rollover — MP-08 / MP-10 / MP-11

Token renewal must retain the existing authenticated socket, request routes,
subscriptions, display tunnels and provider streams. The relay remains transport
only. Normal authorization expiry/revocation and heartbeat freshness still apply.
A relay URL change or removal of its credential remains a real reconnect.

The kernel already sends `DaemonHeartbeat.registration` to authenticate a renewed
credential in-band. The incoming peer-response path must leave token changes to
that path, rather than enqueue `Close`. A config snapshot overtaken while building
registration must authenticate the snapshot credential on that same socket,
then pick up the newer credential on the following heartbeat. No serialized shape changes are required.

## Source regression checks — MP-08 / MP-11

Run the kernel tests matching `token_rollover` through the builder's Cargo slot
wrapper, with disk-backed TMPDIR and the documented resource limits. They cover:

- Renewed credentials after a peer reply, both idle and with active daemon
  requests: no queued Close, continued request admission.
- A second renewal while registration is being built: the snapshot credential
  is authenticated without reconnect, then the next heartbeat uses the newer one.
- Real scoped relay sockets and two kernel connectors: a multi-MB encrypted peer
  request completes intact, an existing waiting-room subscription retains its
  route, the relay receives the new credential and that same subscription delivers
  a subsequent inventory change.

Run the existing URL/removal reconnect and relay authorization suites as well.
These checks are supplementary. A large Ping is not an owner Copy transfer, and
an inventory subscription is not a complete browser View or provider turn drill.

The `relay_token_rollover_probe` example injects ordinary encrypted peer traffic
using the same `DaemonConfig::load_from_env` product profile as an owned running
kernel. Set the exact same lane-owned home and identity/listener environment.
Arguments are the public target kernel ID and payload size (1..4194304 bytes).
Its stdout contains integrity, byte count and timing only. Never pass credentials
as arguments, print profile files, fabricate an identity, or use a shared kernel's
home for this drill.

## Required live cells — MP-08 / MP-10 / MP-11

| Scenario ID | Required real proof |
| --- | --- |
| relay-token-rollover/TR-COPY | Force rollover during a multi-MB ordinary-owner Copy. Bytes, project/context identities and final outcome agree; no silent loss, duplicate import or manual retry. |
| relay-token-rollover/TR-VIEW | Force rollover during an active browser View subscription. Use the real-site list in the Browser/Computer plan at DPR 1 and 2. Zero blank/unavailable frames or disconnect notices. |
| relay-token-rollover/TR-AGENT | Force rollover while a real official-provider turn streams to Web and the real TUI. Same run/thread, continuous output, definitive settled outcome and usable next prompt. |
| relay-token-rollover/TR-HOSTED | Own hosted WSS stack, calibrated client uplink near 8 Mbit/s and RTT at least 60 ms, waiting room at DPR 1 and 2 with real TUI attached. Several renewals, zero unavailable/blank frames/disconnect notices, Copy completion, plus a multi-hour stability session. |

Show RED before the fix and GREEN after, retaining exact source and binary hashes,
commands/exits, screenshots and allowlisted console/log projections per step.
Use the product token-issuance/renewal operations; change a lifetime only in an
owned stack. Never alter staging, shared login state, authority scopes, clock or
heartbeat freshness to manufacture a pass. Source-only and local socket results
do not mark any live cell PASS. Missing compatible consumer source, a signed
release or another real resource must name the precise coordinator/owner action.

For waiting-room attribution, distinguish renewal alone from renewal overlapping
peer traffic: the peer-response reconnect branch is not reached by ordinary
browser `DaemonRequest` traffic. Report the first failing seam, not an assumed
common cause. A successful local test does not close MP-08/MP-10 or constitute
independent MP-11 relay-admission semantic review.

Stop the owned stack, kernel, provider, browser, TUI and proxy at the end. Remove
lane-generated identity state, scratch and copied binaries after retaining public
evidence. Never remove the original linked provider scope or another lane's state.

## MP-08 / MP-10 / MP-11: Apps r4 activation and reboot replay

Apps r4 (`b1110e79b7616c34ce36558f8fbbe831541c5f1e`, Cloud
`00658b85aa81575b7c144adc11f3904eeecf0170`) already retains the live socket
when a peer response observes a renewed credential. Frozen #996
(`669d315a4427746c1d6ae8b9460c5eb95673b692`) additionally binds the registration
credential to its continuity snapshot. Neither dispatches newly staged slice
activation confirmation on a socket that stays connected.

Focused regressions run the dynamic connector against a real local relay with
key-bound scoped admissions. They stage pending confirmation after connection,
then exercise heartbeat registration, token-refresh registration, and the race
where registration authenticates the runtime token before confirmation is staged.
These are supplementary source checks, not user-flow or signed-slice acceptance.
The follow-up sender services pending confirmation after successful registration
queueing on both timer paths, including ticks that need no token change. One
sender owns the existing bounded retry loop per connection and aborts on teardown.
Owner/key/nonce binding, credential expiry, peer-version gating and the serialized
protocol remain unchanged.

Live acceptance must replay the normal r4 TUI `/slice create ... --headed`,
`/room bind`, and `/slice start` flow on its signed inputs, r4+#996, then the
follow-up. Use the real built Web/TUI clients, hosted WSS, an approximately
8 Mbit/s client uplink, measured RTT >=60 ms, DPR1/2, and screenshots/logs for
each step. Normal warm startup must confirm within the existing 15-second bound
without a worker socket fault. A source test does not close this cell. Cold-build
bootstrap expiry and later hosted authorization denial are separate findings.

The r4 terminal soak recorded six closures inside a 776 ms window during a
second machine's deliberate reboot. Its observer retained no socket/target
identity; all 480 checkpoints retained a prompt and reported no unavailability
or page error. Retain that FAIL. Repeat the exact two-machine workflow with
credential-safe socket IDs, allowlisted outgoing target metadata and closure
reasons where available. Every closure remains counted; attribution is evidence,
not a waiver. Do not credit #996 with fixing this unisolated failure. A two-hour
soak is required if a remaining fix changes its path. Never reboot a shared
builder or substitute a synthetic reboot for the real machine fault.
