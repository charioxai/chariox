# BYOM / Chariox Setup (MP-07 / MP-08 / MP-11)

PR1 and PR2 are rebased onto coordinator-requested OSS ownership `27aedc57e`
on main `60c1ccb1e`. Local protocol 479 is retained; relay 73 comes from that base. MP-11 follows the
owner's narrowed behavioural and security-anchor scope. Cloud remains control
plane; each independent kernel owns its runtime and provider execution.

## MP-07 / MP-08 Path-1 reuse

| Step | Existing code | BYOM |
| --- | --- | --- |
| VM provision | `runtime/managed_environment_control.rs`, Cloud `/managed-environments` | Omit provider provisioning/billing/deletion |
| Signed install | `scripts/package-managed-kernel-release.mjs`, `deploy/managed-kernel/{extract-release.py,verify-image-release.mjs}` | Reuse extractor and image signature/artifact/builder checks; replace root/system-service wiring |
| Identity/enrollment | `config/identity.rs`, `runtime/cloud_relay_login_executor.rs`; managed VM admission in `managed_bootstrap/` | Generate normal independent identity; use #888/#302 owner enrollment, keep managed VM admission unchanged |
| Service/relay | `managed_bootstrap/supervisor.rs`, `runtime/cloud_relay_{connection_executor,control}.rs` | Per-user systemd/launchd service; ordinary relay grants, registration and heartbeat |
| Project/context | `managed_context/{outbound_service.rs,kernel/import.rs,transfer/}`, `PrepareManagedEnvironmentContextTransfer` | Reuse encrypted export/import and approval in PR4; managed-environment-bound ticket needs independent-target adaptation |
| Provider accounts | `managed_context/kernel/{export.rs,import.rs}`, ordinary lease profiles | Owner update: no BYOM credential sync; login on target |
| Teardown | `RequestManagedEnvironmentLifecycle`, kernel quiescence | No VM deletion/auto-stop; stop or uninstall only the marked install |

## MP-07 / MP-08 / MP-11 shared installer

The source kernel runs the user's SSH config/agent, with host-key checking,
batch authentication and no agent forwarding. Chariox stores no SSH password
or key. PR1 supports Linux x86_64 with Python 3, Node 22 and systemd --user.
An operator-owned `ssh-machine-releases.json` beside source `config.toml`
selects the approved immutable release/version and independent public trust
pins; clients never supply arbitrary paths or pins. Selected Project context
is a separate optional transfer, not a copied kernel identity.

The source's own kernel credential issues a 600-second ticket via
`POST /v1/kernel-enrollment-tickets` and sends it over SSH stdin only. Target
`--owner-managed-enroll-stdin` redeems the #302 variant of `/auth/device/poll`:
`ticket`, `machineId`, `kernelId`, `publicKeyThumbprint`, optional string
`kernelAlias`; response is the ordinary approved profile + kernel credential.
No raw publicKey or null alias. Cloud account IDs and user IDs differ; compare
expected owner to userId, retaining shared machine/kernel/key validation.
Unused tickets revoke via DELETE. Hosted relay uses wss; bounded loopback mocks
are test-only. Ready requires authenticated local status matching enrolled
identity and a connected ordinary relay path.

Install ID selects distinct release root
`~/.local/share/chariox/ssh-machines/<id>`, private state
`~/.chariox/dev/ssh-machines/<id>`, service and loopback kernel/MCP port pair.
No sudo. Refuse foreign/edited units, symlinked ancestors and occupied/default
ports. Repeat retains identity and release. Failed readiness restores only service
start/enable changes made by that invocation; existing work stays running. Source reservations reconcile only
a positively absent target; uncertain or published installs stay protected.
Ticket-issue or invalid-selection failure cannot reserve a target.

PR2 generic Setup selects a signed distribution version/platform, verifies the
bundle and nested runtime inventory under an independent compiled public pin,
and uses the same marked installer core. Standalone device approval uses #888
kernel enrollment; CLI login enrolls the terminal separately and offers Setup
when no fresh local heartbeat exists, including isolated install roots.
`--enroll` accepts stdin or a hidden prompt only, never a code argument. No
per-user secret is embedded. Browser “Add this machine” is PR3's ticket UX.

Explicit repair restores only missing marked service files. Upgrade verifies
under original pins, activates atomically, checks unchanged ready identity,
and rolls back on failure; interrupted-journal recovery is tested. Forced death
can leave an exclusive lock requiring operator settlement. Uninstall stops only
this service/removes verified release bytes and links; private state and Cloud
identity remain. A validated, stopped/disabled install records removal intent
before unit unlink. Inspection retains its published ownership, and remove can
retry after reload failure; foreign units and active jobs remain refused.
CLI login resolves its marked install from the invoking release and forwards
that validated ID/port to Setup, including named and nondefault-port installs. Never adopt staging unit/root or delete the existing computer.

## MP-07 / MP-08 / MP-11 lean slices and acceptance

1. PR1: `/machine add ssh HOST` / `/machine remove ID`, source kernel request,
   signed SSH install, enrollment/readiness, local 479 snapshot/hash and actual
   localhost sshd + real-kernel drill against strict mock Cloud/relay.
2. PR2: generic Linux executable + unsigned .deb; macOS arm64 executable/app
   skeleton + launchd adapter; versioned signed `install.sh`, CLI/TUI login
   offer, lifecycle, unsigned CI artifacts and external owner signing hook.
3. PR3: Cloud generic downloads, browser code issue/revoke and one-liner UX.
4. PR4: Copy kernel/selected Project via existing export RuntimeInteraction and
   encrypted kernel transfer; provider login remains per machine.

See `apps/setup/README.md` and lane `LIVE_TEST.md` for commands/artifact contract.
Linux arm64/Windows, real macOS service/signing/notarization, signed public
publication, production Cloud and Mac-to-LAN acceptance remain coordinator
steps. No owner decision blocks source work. Private owner signing inputs stay
off builders. Local fixtures establish their named seams, not MP closure or
fresh-machine parity acceptance; Path-1 bootstrap checks are unchanged.

## MP-07 / MP-08 / MP-11 ownership rebase

The strict startup parser admits SSH, self-ticket and device stdin enrollment
and readiness as distinct typed commands, rejecting extra arguments before
runtime initialization. Inherited Browser/Computer/public-provider guards bind
local 479 and retain relay 73; the aggregate version/hash is reconciled. Local
source/mock drills do not establish live Cloud, signed distribution, real user
services or fresh-machine acceptance.
