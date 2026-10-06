# MP-08 / MP-10 / MP-11: multidomain round 2 on main

Base: `e325afa580d81954e2c179757fc53fa02ed2a2b3` (#892). The actual source
declares local protocol435 / relay73; the coordinator prompt labels it426/73.
This successor uses the explicitly authorized **local443 / relay86**.

## Included source heads

- `md/stack-on-main`: `f3ad77defc0e8df37a8ef15240354218859c0e74`
- `md/hardening-on-main`: `6a03ee3b627117df448b409223e4e9b63835ac90`
- `md/refusals`: `dcbbfa23a3f5cf3c0483a69d3d1b0acc477fbc6c`
- `md/access-model`: `f1e507207a4a3c211d778175ce0b44bab62298bd`
- `md/access-clients`: `94a38a655039f5952fa3cd3eda5ca7747dd305a0`
- `md/dom-mirroring`: `5954b100825f18a1465a2061ce2b897d73922eb7`
- `md/security-fixes`: `4655248eccbce5eb943c8040f03a4b86d9821f8c`

The supplied staging head4e393b031 is not available on this builder. This
reconstructs its named components and preserves main's existing MP-11,
kernel-access, workflow, Browser artifact and bounded Computer contracts.
There is no claim of staging tree identity or deployed acceptance.

MP-08/MP-11 conflict resolutions retain cancellation during explicit startup,
read-only observation after stop, retained grant epochs, typed refusal
provenance, secret-field input guards, mirror input guards, private inherited
CDP pipes, shadow protection and actor-lock ordering together. Clients for the
unreleased multidomain features require443; main Browser artifacts retain
their released435 minimum. Snapshot/hash guards bind the combined443/86
shape, with released provider DTO snapshots unchanged.

MP-10: the DOM component drill uses the product host controller and its private
CDP pipe for source Chromium. Its reference viewer uses Playwright's sandboxed
fixture-owned pipe; it no longer expects the product to expose a loopback CDP
endpoint. The viewer is test infrastructure, not another runtime authority.

Focused source checks and the DOM fidelity/latency matrix establish their
listed seams only. Hosted Cloud/relay/WAN, official-provider, signed-release,
OS-level IME, macOS and fresh ordinary/Path-1 acceptance remain separate gates.
Current security-anchor semantic review follows the owner-narrowed MP-11 scope;
non-security source does not require per-blob semantic admission.

## MP-08 / MP-10 / MP-11 validation

Validated implementation source: `8ab18e96c9f4040ccb0b7c8e25fb32f1cb306275`.
The following documentation commit changes this record only. Commands, exact
source identities, exit codes, resource samples, RED receipts and cleanup are
retained in the external mdmain lane evidence; no screenshots enter Git.

- Rust1.88: workspace/all-target `cargo check --locked` passes at7762795c0;
  only two test assertions change afterward. Final test build, formatting and
  workspace/all-target/all-feature clippy pass at8ab18e96c. Clippy has875
  inherited diagnostics on both frozen main and the candidate, zero regressions.
- Focused Rust: protocol snapshots194; kernel browser44 (10 opt-in ignored);
  Notes16 (2 ignored); grants5; typed refusals2; controller87; App views10
  (1 ignored); visible region4. Other focused passes include command cache39,
  relay peer33, shared App events2, signal guards3, crypto1, forwarded-peer
  denial5, relay protocol3 and actual peer-identity admission11. Filters overlap;
  counts are per suite. An earlier identity filter selected zero and was corrected.
- Node: controller/Notes/DOM metrics/display145; shared client1170; focused
  CLI31; Bun1.4.2 real OpenTUI renderer3; protocol/publication policy12. All three
  touched TypeScript package checks pass. Their source is unchanged after420978890.
- DOM: seven fixture groups at DPR1/2 pass all14 fidelity/latency rows and24
  protection/event-path checks. Worst row input p50/p95=48.8/63.4ms. The executed
  controller/renderer bytes are unchanged after the c29205809 matrix source.

MP-08/MP-11 integration repairs preserve retained grants across focus changes
and retire queued operations on explicit revocation. Exact trusted grant-policy
messages map to bounded typed refusals; controller/transport text remains Other.
Observed same-origin mirror frames inspect the live secret-field leaf, while
opaque/direct frame input stays denied. MP-10 viewer scrollbars match the source
browser, and presenter/drill protocol pins use443/86. Source claims and component
limits above still apply; these passes do not close MP acceptance items.
