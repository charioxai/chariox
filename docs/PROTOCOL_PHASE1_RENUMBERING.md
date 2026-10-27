# Apps Phase 1 protocol renumbering above release F

Release F (main `f1c402b82`) shipped local daemon protocols 368–376 and relay peer
protocols 58–68 on top of the shared base (local 367, relay 57). Apps Phase 1,
still unreleased, had used the same numbers independently. On 2026-10-03 Phase 1
was renumbered above release F before main was merged into it. The OSS and Cloud
repositories use this one mapping:

- local daemon protocol: every Phase 1 number N with 368 ≤ N ≤ 406 becomes N + 9
  (368 → 377 … 401 → 410 … 406 → 415);
- relay peer protocol: Phase 1's 58 becomes 69;
- release F's own numbers (local 368–376, relay 58–68) do not change.

| Phase 1 before | Now | Change |
| --- | --- | --- |
| local 368 | local 377 | App plans per release |
| local 369 | local 378 | release inputs digest for bound releases |
| local 370 | local 379 | Room browser bar |
| local 371 | local 380 | App agent panel placement (`ui.agentPanel`, `AGENT_PANEL_PROTOCOL`) |
| local 372 | local 381 | install operation phase `queued` |
| local 383 | local 392 | critical approvals need the Chariox passkey |
| local 384 | local 393 | `AgentInstance.failed_requests` |
| local 385 | local 394 | `RevokeAppFileGrants` |
| local 388 | local 397 | per-turn agent substitutes |
| local 396 | local 405 | native approval origin |
| local 398 | local 407 | `AppWorkerPhase::Quarantined` |
| local 400 | local 409 | App clipboard copy-out and link opening |
| local 401 | local 410 | saved App data snapshot restore; Phase 1 main’s `LOCAL_DAEMON_PROTOCOL_VERSION` |
| local 393 (KA) | local 402 | connection classes and audit attribution |
| local 394 (KA) | local 403 | shared owner passkey popups |
| local 395 (KA) | local 404 | process-bound access grants and lifetime settings |
| local 403 (KA PR7) | local 412 | local token and terminal authority enforcement |
| local 404 (KA PR8a) | local 413 | terminal sudo turn and critical receipts |
| local 406 (KA PR8b) | local 415 | external sudo and Meta migration notice; current local protocol |
| relay 58 | relay 69 | workflow event capability flags dropped; the current `RELAY_PEER_PROTOCOL_VERSION` |

The remaining Phase 1 numbers map the same way. KA PR10 adds shell sudo
entry routing using the existing protocol-415 shape. Relay stays at 69;
no new allocation is used. Builds and evidence recorded before 2026-10-03
report the original numbers: for example candidate `a3bd5b394` reported local
372, which is now 381.
