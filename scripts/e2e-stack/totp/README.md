# MP-08 / MP-09 / MP-10 / MP-11: Vault TOTP drill

`site.mjs` is a loopback-only local enrollment/login gate. Its independent
Node crypto implementation rejects reused TOTP periods and consumed recovery
codes. Keys and accepted codes stay in memory. The owner supplies the manual
key through the normal protected credential interaction.

Run the supplementary site tests with `node --test site.test.mjs`.

`live.mjs` requires the real S8 kernel, an existing product-linked Codex profile,
a lane-owned state directory and an external evidence directory. It enrolls its
own disposable kernel through the hosted validation fixture, creates the Room
in **the new terminal-os web UI**, and drives protected entry there. A real
Codex `gpt-6-luna` / `low` agent enrolls the authenticator and signs in twice in
different periods. No TUI or old web UI is used.

```sh
DPR=1 \
CHARIOX_TOTP_KERNEL=/absolute/owned/chariox-kernel \
CHARIOX_TOTP_PROFILE=/absolute/product-linked/codex \
CHARIOX_TOTP_STATE=/absolute/lane/state-dpr1 \
CHARIOX_TOTP_EVIDENCE=/absolute/evidence/dpr1 \
node scripts/e2e-stack/totp/live.mjs
```

Repeat with `DPR=2` and distinct state/evidence paths. The optional
`CHARIOX_TOTP_ORIGIN` defaults to the private hosted validation stack. Run node,
Bun and Chromium under the lane's required memory scope. The driver never
copies provider credentials. On the root builder, it copies only browser source
assets into its own scratch and runs the real browser host as `ubuntu`, with
Chromium's sandbox enabled. The Playwright viewer is a separate test browser.

The harness records kernel/head identity, screenshots, owner interaction
counts, private boolean value-absence checks and cleanup. It stops only its own
kernel/browser processes and removes its own state/workspace/scratch. The
explicit policy fence is tested separately on the base by
`secret::tests::totp_inputs_compute_codes_instead_of_returning_the_seed`;
the base rejects `injection.kind=totp`.

These are local-fixture regressions. They do not establish real-service
`apps-autonomy/AU-10` acceptance, which also needs a real account and service,
two periods and absence checks on all model-bound channels.
