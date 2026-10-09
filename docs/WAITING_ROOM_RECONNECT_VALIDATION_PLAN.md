# Waiting-room reconnect acceptance (MP-08 / MP-10 / MP-11)

Scope: OSS #945 and Cloud #316, ordinary and managed waiting-room clients.
The coordinator allocated local protocol 488 for encrypted binding repair.
Relay and peer frame shapes stay unchanged. This scoped drill does not close
Browser/Computer display, provider-login recovery or the managed parity matrix.

## Required scenarios

`apps-tui-ux/WAITING-ROOM-RECONNECT` requires all of the following on the real
built web entry, Chrome at DPR 1 and 2, the compiled TUI, exact-head kernel and
real hosted WSS relay. Use product enrollment and linked provider profiles;
never modify a shared login. Shape both client paths to approximately 8 Mbit/s
and at least 60 ms RTT. Retain screenshots, public transport diagnostics,
visible frame observations, binary hashes/source commits and cleanup evidence.

1. Cold open while an enrolled kernel reports slowly: keep loading placeholders
   until a first authoritative report; do not show an empty inventory prematurely.
2. Reopen with account-scoped client cache: display cached/refreshing rows first,
   then reconcile additions and removals by stable ID without list flicker.
3. Kernel relay drop and reconnect: retain visible rows during the grace period,
   mark unavailable inventory reconnecting and return to live state.
4. Client network blips during immediate user-created session mutations:
   recover the current inventory in web and TUI without a decryption gap, duplicate
   bindings or lost updates. Each fresh encrypted subscription ID is distinct;
   session recovery retains its event cursor, inventory gets a fresh baseline.
5. At least two hours with both clients connected: no stuck inventory or lost
   connection; repeat a session-list mutation after the soak to prove liveness.

Show failure before the repair and success afterward, preserving the original
source identities. Source/WebCrypto fixtures supplement these cells and never
replace them. A required failing cell keeps this scenario FAIL. Missing hosted
capacity or a real resource keeps it BLOCKED with the coordinator/owner action
named; no fallback stack is counted as acceptance.

## Focused regression checks

Build each client from its own worktree. Run kernel-client subscription binding,
renewal and protocol tests, Cloud subscription lifecycle/real-WebCrypto and
waiting-room checks, TUI inventory checks and the kernel protocol shape/hash
suite. Run Rust through the shared cargo-slot helper and touch sources before
measured builds. Stop only lane-owned processes/containers/stacks, remove
lane-owned disposable runtime identities and scratch, and leave shared accounts,
keys, reviewer state, other lanes and build caches intact.
