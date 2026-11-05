# MP-08 / MP-09 / MP-10 / MP-11 — AU-11 development drill

Run against a private build of the kernel, relay and the production Cloud new-UI
bundle on an owned valenv stack. No old UI,
TUI, component fixture entry or provider stub is used. The real kernel enrolls on the hosted WSS relay through the product device flow.
The page opens `/waiting-room?ui=os` and submits `/sudo` from the Prompt bar;
approval uses its masked Chariox passkey field. Fixture sign-in/enrollment is setup
through the product dev-auth APIs; runtime actions all use the new shell.

The coordinator explicitly authorized a disposable local service for this round.
It issues API keys; an agent creates a deploy resource with one, records/lists it,
revokes it through the stored request, and verifies its replacement works. The
service deliberately echoes the key on revoke. Only status may reach the model.
The generated password must appear automatically in the project's inventory.
Both Codex gpt-6-luna low and Claude haiku-5-5 run at DPR1 and DPR2, one prompt per
cell. The same script with `--expected-red yes` runs DPR1 on the base kernel and
requires the absent-tool path, with no confirmed revoke. Use no more than 20
provider turns per round; count retries too.

Run with node through the lane's boundrun.sh, supplying absolute paths:

Video capture needs Playwright's ffmpeg installed in a lane-owned cache. Set
`PLAYWRIGHT_BROWSERS_PATH` to that cache for both installation and the driver.
Use `--providers claude-p --dprs 2` for a focused rerun without repeating other cells.

```sh
node scripts/e2e-stack/resource-register/live.mjs \
  --home /own/chariox/dev/drill-state \
  --evidence /own/evidence/drill \
  --cloud /own/cloud-checkout \
  --stack pr944-vreg \
  --kernel-bin /own/bin/chariox-kernel \
  --relay-bin /own/bin/chariox-relay \
  --codex-scope /product-created/codex/profile < /coordinator/input-file
```

Use only a stack you own. The driver reads its dev-auth input privately and never
prints it. Account setup uses only product `LinkProviderAccountProfile` and the #917
setup-token enrollment request, fed on stdin. Secret file contents are never printed. Evidence
contains screenshots, recordings, console errors, public history, resolved model IDs,
key invalidation/replacement status and cleanup. A private boolean absence check
covers supplied service keys/passkey in public history and kernel logs. Focused
Rust tests separately check the opaque generated value itself. The fixture uses
its own CHARIOX_HOME, terminates its exact owned PIDs and removes its state.

This proves the local development scenario only. Real-service AU-11 is NOT RUN
until Miguel supplies authorized accounts. The register is agent-facing only,
per the owner's 2026-11-02 decision. The tools are exercised in the new shell's
real agent transcript; no Vault-panel rows or UI mock are part of the feature.

The current production self-hosted bootstrap does not advertise the kernel protocol,
so it rejects `/sudo` before dispatch. That route remains a recorded New-UI gap;
the hosted entry is used with coordinator approval.
