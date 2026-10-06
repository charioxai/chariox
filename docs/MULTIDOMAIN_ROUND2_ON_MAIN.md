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
