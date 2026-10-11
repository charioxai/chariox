# MP-08 / MP-10 / MP-11 — owner A/B runtime scripts

These scripts start the Chariox kernel/browser and the qualified Selkies wheel
baseline on one quiet Ubuntu 26.04 CCX43. The web app and relay use the matching
prebuilt hosted validation stack. The owner guide has the two links, five steps,
public sites and rating table; the operator guide covers enrollment and cleanup.

The distributable kit also needs these public, prebuilt assets beside the scripts:

- `bin/chariox-kernel`, `bin/chariox-cli` and their actual `bin/OSS_HEAD`.
- The matching built `packages/kernel-client` as `sdk/`, plus its runtime dependencies.
- The qualified standalone Python as `baseline/python/`, wheel environment as
  `baseline/runtime-venv/`, native wheel libraries as `baseline/runtime/`,
  `baseline/selkies.lock.json` and license notices as `licenses/`.
- `runtime-libs/`, `ARTIFACTS.json` with exact source/image/file identities, and
  `SHA256SUMS` covering every shipped file except itself.

Build these assets before transferring the kit. Native builds use the approved
cargo slot with `cargo build --release --locked -p chariox-kernel
--bin chariox-kernel --bin chariox-cli --features native-display`. Pin the Cloud
bundle to those runtime display assets. Seal the kit with SHA-256 after copying
assets; exclude runtime state, provider profiles, auth files and keys. OpenH264
is fetched directly from Cisco at startup and is not redistributed in the kit.

`ab-up.sh` accepts only the public settings listed in `OPERATOR.md`. Normal owner
use rejects fixture authentication. Rehearsal must explicitly set
`AB_REHEARSAL=1`; `AB_SKIP_INSTALL=1` is only for a host with dependencies already
installed. Startup and cleanup receipts do not establish latency, CPU parity,
Mac usability, provider/Vault coverage or multi-hour stability. Preserve RED
results and source identities alongside the archive hash.

Scripts keep generated identities in disposable state, isolate both browser
profiles, retain exact child PID/start identities, and remove only their own
marked state, workspace and firewall rule. The gate helper edits only the owned
validation stack's Caddy fragment under the stack lock.
