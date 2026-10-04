# MD-DISPLAY-01/02/03/04 — Phase 2 handoff being validated

Local branch `agent/display`, base `9334141d420f8a32393f206102c5b8b4a1b0b609`.
Research only; coordinator publishes; owner selects the production design.
Full historical curves + current seam proposal are in
[transport comparison](docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md).
Historical Phase 1/2 source identities remain bound to their original receipts;
they do not cover changed current runtime, viewer or Selkies files.

## MD-DISPLAY-02/03 reviewer mapping

1. Launch error/readiness cleanup: `runtime.mjs` + `owned-process.mjs` and
   `lifecycle.test.mjs` (missing Chrome/Xvfb, readiness timeout).
2. Selkies callback failure: `selkies-packets.mjs` latches unsupported stripe,
   JSON parse and send errors; awaited start/stop/global failure checks reach
   run cleanup. Packet tests cover all three and supported frame forwarding.
3. Campaign statuses: children with 1/124/130/null/signal fail parent; interruption
   is explicit 130. Immediate completion is registered before launch returns.
4. Provenance: doc/status/handoff no longer assert historical hashes match final
   files. Focused clean-source live coverage will be recorded separately.

MD-DISPLAY-02/03 incident audit: invalid/unowned signals are refused; each group
member is verified before a negative group signal. No retained run establishes
cleanup at incident time; no sender attribution claimed. Initial inventory empty.
No REVIEW_INBOX.md present. Post-review live validation pending, so this checkpoint
is not a final ready handoff. No production protocol, providers, secrets, Cloud,
shared services, GitHub writes or deployment work.
