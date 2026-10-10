# BYOM over SSH (MP-07 / MP-08 / MP-11)

PR1 coordinator-requested base: OSS ownership `27aedc57e` on main `60c1ccb1e`
(local 439, relay 73);
BYOM retains local 479 and inherits relay 73, with no BYOM relay shape change. Owner request 2026-10-05 adds an owner-managed machine, not a
Cloud-disposable environment. The ordinary runtime remains the authority.
MP-11 follows the owner's narrowed behavioural/security-anchor scope.

## Path-1 map

| Step | Existing seam | BYOM reuse |
| --- | --- | --- |
| VM create/provision | `runtime/managed_environment_control.rs`: `CreateManagedEnvironment` → Cloud `/managed-environments`; Cloud infrastructure manager owns provider reconciliation | VM-specific; omit entirely |
| Signed package/install/upgrade | `scripts/package-managed-kernel-release.mjs`, `deploy/managed-kernel/{extract-release.py,verify-image-release.mjs,install-image.sh,upgrade-image.sh}` | Reuse bounded extractor + signature/artifact/builder verifier. Root installation/service/data-volume wiring is VM-specific |
| Ticket and new machine identity | `managed_bootstrap/{state.rs,cloud.rs,mod.rs}`: exchange/confirm; `config/identity.rs`: `load_or_create_managed_runtime_identity` | Normal product identity generation is reusable; current exchange is bound to a managed environment; root-owned envelopes are VM-specific |
| Service and relay | `managed_bootstrap/supervisor.rs`; `chariox-path1-managed-bootstrap.service`; `runtime/cloud_relay_{connection_executor,control}.rs` | Ordinary kernel/refresh/presence reusable; system service and broker wiring VM-specific |
| Project/context | `PrepareManagedEnvironmentContextTransfer`; `managed_context/{outbound_service.rs,kernel/import.rs,transfer/}`; CLI `waiting-room-managed-environment-launch-controller.ts` | Encrypted package/import and existing move prompt reusable; Cloud ticket currently requires environment ID |
| Provider accounts | `preflight_provider_account_exports`; `managed_context/kernel/{export.rs,import.rs}`; ordinary provider profiles/lease materialization | Existing Path-1 supports selected transfer. Owner update: BYOM does **not** sync provider credentials; request provider login on the target |
| Teardown | `RequestManagedEnvironmentLifecycle` and Cloud provider deletion; ordinary kernel quiescence | VM deletion/auto-stop omitted; stop/uninstall only the exact owned install |

## SSH path and settled defaults

The home kernel invokes the user's `ssh` executable with normal config and agent.
It stores only the SSH destination and install identity, never passwords/keys.
Use batch authentication and normal host-key checking; errors request that the
user establish SSH access with their existing client. No automatic sudo, package
installation, host-key bypass, or agent forwarding. The independent PR1 installer supports Linux x86_64
with Python 3, Node >=22 and a working `systemd --user`; unsupported platforms
fail before mutation. macOS launchd and Linux arm64 packaging are later slices.

Select a chosen kernel by immutable signed release digest (plus display version),
not by copying the running kernel's identity or mutable state. A selected Project
is a separate optional context plan. The default is the home kernel's approved
release for the probed target platform; an explicit approved release overrides it.
Release and builder public pins come from the trusted home release inventory,
never the uploaded package itself. Upload release bytes and public pins, run the
existing bounded extractor, then verify signature, all artifact hashes, source
identity, builder attestation and target before executing any uploaded binary.

Both entry points end with a fresh independent kernel owned by the same user,
using OSS #888 / Cloud #296 per-kernel credentials. The coordinator approved
source-kernel issue/revoke at `/v1/kernel-enrollment-tickets`, with purpose
`owner_managed_machine`, optional machine label and TTL at most 600 seconds.
The source credential uses `x-chariox-kernel-credential`; no browser credential
is copied. Ticket delivery is exclusively SSH stdin. The target uses a separate
`--owner-managed-enroll-stdin` bootstrap and the same #888 approval validation
and private profile storage as device enrollment, without relaxing Path-1 checks.
The proposed ticket variant of `/auth/device/poll` carries `ticket`, `kernelId`,
`machineId`, `publicKey`, `publicKeyThumbprint`, `kernelAlias` and expects the
ordinary `approved` profile plus `kernelCredential`. Cloud route alignment is
tracked in the lane status until byomcloud confirms that unspecified seam.
Cloud stores ownership/directory/tickets, never runtime context or terminal data.

The preferred self-setup entry point is **Chariox Setup**: one generic signed
installer, identical for every user, with no embedded enrollment secret. macOS
uses an app/.pkg with Developer ID signing/notarization (owner prerequisite;
CI stubs signing). Linux uses a static binary and/or .deb/AppImage; Windows is
later. First run opens existing device-flow approval (one click when signed in),
obtains/verifies the target release, installs CLI + kernel per-user, starts the
user service and enrolls it into My kernels. Detection, repair, upgrade and
uninstall use the same core as SSH push. No per-user executable downloads.

A public versioned `install.sh` remains available for a target without Chariox:
`curl -fsSL https://chariox.com/install.sh | sh`. It detects OS/arch, downloads
CLI + kernel, verifies signatures/digests against published runtime pins,
installs per-user and runs `chariox login`. After device login, the CLI or
waiting room offers “Set up a Chariox kernel on this machine?” when no local
kernel is found. A CLI-only installation gets the same offer later. Script
hosting and publication are coordinator/owner work; source and tests live here.

Cloud web “Add this machine” provides generic Setup download links and a
one-liner `curl -fsSL https://chariox.com/install.sh | sh -s -- --enroll <code>`.
The user must run it on the target; the browser never installs software.
The code is short-lived, single-use, user-bound, revocable before redemption
and never logged. Prefer stdin transport so the code is never exposed in `ps`;
an implementation using args must scrub it immediately. Cloud redemption
issues only that kernel's credential. Neither install script nor Setup embeds
provider credentials or private release keys.

After enrollment, “Copy a kernel here” selects a source kernel and target from
any client, reuses the existing export RuntimeInteraction (“Ready to move…”)
and encrypted kernel-to-kernel managed-context transfer, and leaves provider
logins per machine. Request login on the target when needed; never sync provider
credentials. Release installation, independent identity and optional Project /
selected kernel configuration copy remain distinct operations.

Each install ID has `$HOME/.local/share/chariox/ssh-machines/<id>` (release
roots) and `$HOME/.chariox/dev/ssh-machines/<id>` (explicit `CHARIOX_HOME`),
`chariox-ssh-<id>.service`, and an explicit distinct loopback port pair (kernel `port`, runtime MCP `port+1`). Preserve
ordinary HOME/PATH; no provider sandbox or workspace allowlist. Refuse symlinked
ancestors, foreign roots/units, missing user bus, and occupied ports. Repeating
the same digest/ID/port is idempotent; different selections require an explicit
upgrade. Multiple IDs coexist. Releases are verified content-addressed trees;
initial publication installs one atomic release pointer. Upgrade stops only this service,
verifies the replacement, starts/health-checks it, and rolls back on failure;
mutable state and identity stay in place.

After verified install the target must consume a short-lived one-time owner
machine enrollment ticket through a shared bootstrap exchange, retain its own
machine credential, and use ordinary relay registration/refresh. The home sees
same-user-owner, key-bound live presence plus a kernel ping before declaring ready.
Only then offer the existing “Ready to move <project> to <machine>” operation.
Cloud owns tickets and directory records only; runtime and context use encrypted
kernel/relay paths. No VM resource, billing or shutdown lifecycle attaches to BYOM.

Stop leaves state/releases intact. Uninstall disables the exact marked user unit
and removes only its marked release root; retained kernel state requires a
separate explicit purge and account identity retirement. Never stop/rewrite
`chariox-md-staging.service` or `~/.chariox/dev/md-staging`. Reserve a distinct
ID and port in the Mac-to-Omarchy drill.

## MP-07 / MP-08 / MP-11 lean slices

1. PR1 SSH push: kernel-owned approved release catalogue; `/machine add ssh
   <host> [--id <id>] [--port <port>] [--release <release>]` and `/machine remove
   <install-id>`; upload, target verify, install, one-time enrollment, owned
   service start and authenticated local relay readiness. Stop/uninstall retains
   identity/state. Repeat selects the same digest/root/port/identity. A different
   release is refused pending explicit upgrade support. Local 479 snapshot/hash
   and localhost source-kernel/SSH drill; Mac-to-Omarchy handoff is required.
2. PR2 Chariox Setup plus public install.sh and CLI login offer: shared install
   core, device-flow enrollment, repair/upgrade/rollback/uninstall. macOS app
   signing/notarization needs owner credentials kept off builders.
3. PR3 Web Add this machine: generic download links and revocable one-liner
   ticket UX; Cloud implements the same user-bound, single-use exchange.
4. PR4 Copy kernel here: existing export approval and encrypted managed-context
   flow adapted to independent targets. Provider login stays on the target.

PR1 release selection is an operator-owned `ssh-machine-releases.json` beside
this kernel's `config.toml`, with a default release ID and approved immutable
archive/digest plus independent public pins/fingerprints. Client requests never
choose raw paths or trust pins. Catalogue hosting/download UX is later work.
Multiple installs coexist by explicit ID and port; defaults derive stably from
SSH host. No sudo, VM provisioning/deletion, Cloud runtime proxy, automatic
provider credential sync or system-service mutation. macOS/arm64 install and
upgrade execution remain later slices. See lane LIVE_TEST.md for the catalogue
schema and coordinator commands.

## MP-07 / MP-08 / MP-11 reviewer corrections

Cloud ticket redemption uses the strict #302 ticket variant of `/auth/device/poll`: ticket, machineId, kernelId, publicKeyThumbprint, and an optional string kernelAlias. Account IDs and user IDs are separate; owner admission compares the returned userId to the source owner's userId, with the normal shared kernel/key/machine binding checks. An absent alias is omitted.

A ticket-issue failure cannot reserve an install. Failed deployment and corrected retries reconcile the source selection only after an SSH inspection positively proves no install root or loaded/on-disk service exists. Unknown access failures, edited units and published installs remain protected. Remove can clear a proven absent selection. These helper messages are internal SSH installer inputs, not new local/relay protocol shapes.

## MP-07 / MP-08 / MP-11 ownership rebase

The strict startup parser admits the private owner-managed stdin enrollment and
readiness commands before ordinary runtime initialization. Extra arguments are
refused. The inherited Browser/Computer/public-provider guards bind local 479
and retain relay 73; wire hashes remain unchanged except the aggregate snapshot
that includes the local version. Local source/mock drills do not establish live
Cloud, signed distribution, service-manager or fresh-machine acceptance.
