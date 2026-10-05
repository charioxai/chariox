# BYOM over SSH (MP-07 / MP-08 / MP-11)

Frozen source: `74e50b787a5919ee5c3d580c5b088989fd4a1adf` (local 416,
relay 70). Owner request 2026-10-05 adds an owner-managed machine, not a
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
using the per-kernel credential and delegated grant model of OSS #888 / Cloud
#296 (not present at frozen base 74e50b787). Read-only inspection of #888 at
`e224df83c5f7c8c9047fe26e7cccd2acf39e835e` confirms key-bound KERNEL device
enrollment and public status; it is a prerequisite identity, not this lane base. Do not transfer a machine credential
or home Cloud session. SSH push asks the source kernel for a short-lived one-time
target enrollment ticket; self-setup uses existing browser device approval.
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
`chariox-ssh-<id>.service`, and an explicit distinct loopback port. Preserve
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

## Lean slices and blocking decision

1. **PR1 — SSH push** (MP-07 / MP-08 / MP-11): source-kernel transport,
   shared per-user verified installer, one-time target enrollment, start/relay
   readiness, TUI add/remove, tests, localhost SSH drill, Mac→Omarchy handoff.
   The independent installer is implemented first; the ticket/ownership bridge
   and client wiring require the agreed Cloud contract.
2. **PR2 — Chariox Setup** (MP-07 / MP-08 / MP-11): generic installer app +
   shared core, device-flow enrollment, public versioned install script with
   tests, CLI login/waiting-room setup offer. Shared detection/idempotency,
   repair/upgrade/rollback/uninstall. Signing/notarization is owner-supplied.
3. **PR3 — web Add this machine** (MP-08 / MP-11): generic download links,
   one-liner, Cloud revocable single-use user-bound enrollment-code API.
4. **PR4 — Copy kernel here** (MP-08 / MP-10 / MP-11): source/target picker,
   existing reviewed export/context import, ordinary target provider login,
   multi-client/reconnect drills. No provider credential syncing.

Owner/coordinator decision: supply/assign the #888/#296 integration and confirm the Cloud owner-managed ticket
issue/exchange and context-ticket contract. At this source the managed bootstrap
requires an environment ID and protected `/etc/chariox/bootstrap` envelope;
`BootstrapConfig` enforces `CHARIOX_HOME=HOME/.chariox`. Existing account machine
pairing consumes an opaque `/machines/pair` response and cannot establish a
safe target per-kernel credential exchange. Do not weaken Path-1 checks or invent
Cloud responses. Enrollment/context-dependent work is blocked on this seam;
the installer layer is independent and continues. Local protocol **444** is
reserved for new add/remove shapes and snapshots; relay **87** only if needed.
No new protocol shape is needed for the independent install layer. This
checkpoint provides `apps/kernel/ssh-machine/{transport,remote}.mjs`; it verifies
and installs, supports repeat/stop/remove, retains mutable state and refuses
start until enrollment exists. It has no TUI/kernel request wiring yet. It
requires integration into the home kernel's owner-admitted operation service
and trusted release catalogue; the raw file inputs are internal deployment
inputs, not a client authority. Upgrade/repair/start/Cloud readiness are designed
above and not implemented here.

Passing fixtures/localhost transport do not close MP-07/MP-08/MP-10 or supply
current MP-11 security review, signed fresh-machine parity or hosted acceptance.
