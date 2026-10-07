# MP-08 / MP-10 / MP-11 — kaext round 4 IN PROGRESS

## 2026-10-07 02:22 UTC — MP-11: evidence-backed P1 dispute and serializer preservation
- Exact c0 base already calls local::redact_client_response_value from serialize_frame (kernel_protocol.rs:862), recursively removing remote_execution.relay_token from every response/event/replay. Existing snapshot serializer regression covers it. The review's claimed bypass is not reproducible on this base.
- Real base-equivalent binary, real official outside Codex, real CLI grant/TUI passkey, real product slice/private relay and real worker Codex: initial/updated/retained-replay snapshots and terminal/provider completion PASS with no exposed credential. Stored agent.created receipt confirms the actual binding credential is populated (boolean-only forensic query). Actual kernel build source ee43566ec; its kernel/relay source trees are identical to c0. New exact-base build is queued for independent provenance.
- Original generated RED helper was an identity adapter, not the actual pre-existing serializer; do not credit it as live leak proof. Corrected baseline uses the actual global redactor: Unix credential case must pass, broader header/unknown-variant/one-writer contract must fail until the new boundary is present.
- Corrected owner control: owner wire replies already redact too; raw runtime credential-presence control proves projection does not mutate stored credentials. Preserved shared browser artifact chunk budget through serialize_frame_value.

## 2026-10-07 02:18 UTC — MP-08 / MP-10 / MP-11: implementation, validation running
- One socket boundary handles every response/cache, live/coalesced/replay event, terminal stream and control write. Shared exhaustive policy covers 389 response variants and 21 event variants; unknown payloads fail closed. Structured remote credentials and literal registry header values are projected at delivery, with free-form terminal bytes retained.
- Fail-first base + identity helper: exit101, three generated boundary failures; 32 other filtered tests pass. Unix fixture stopped at its owner credential control and is NOT credited as leak proof. Corrected fixture/replay assertions queued with final checks/builds under the compile flock.
- Real official outside Codex + real compiled TUI + product-linked acct-686 + product-created Docker slice running. Earlier GLIBC/namespace/setup/replay diagnostics retained honestly. The slice has a real provider profile, but its command projection currently omits the binding credential; investigating the real RED precondition. No provider401 observed.
- Disposable slice uses documented sandbox compatibility, derivative worker image and operator-only signal guards. Exact binary/support provenance and validation-only deltas are recorded outside the repo. Protected hosts/services and other lanes untouched.

## 2026-10-07 — MP-11: subscription credential boundary
- Required base verified: c0dd72e77fab48bd902cdb46f7f91f94a4156b1d, clean worktree. Required plans and frozen AGENTS read. Prior handoff retained in external round4 evidence.
- Initial writer analysis missed the shared serializer redactor; corrected below. Adding a final caller-aware delivery policy and generated inventory remains useful class hardening.
- Resource starting sample: disk94GiB, MemAvailable20GiB. Every Cargo command will use the shared compile flock and scoped disposable homes.

## Coordinator asks — MP-08 / MP-10 / MP-11
- No serialized shape change intended; no protocol allocation requested.
- Hosted real-machine/network acceptance needs approved resources; protected relay/Apps hosts remain prohibited. Local real provider/TUI drill will run; no full MP acceptance claim.
