# MP-08 / MP-11 — Kernel Cloud ownership and terminal auth (daemon protocol 478)

One Chariox account represents one user. My kernels lists every kernel of the account, grouped by machine; collaboration exposes invited sessions only. Each ordinary kernel enrolls independently. `/cloud link` and attached waiting-room Login ask the kernel to start device authorization with its actual kernel ID, machine ID and relay-key thumbprint. Approval returns a kernel credential, without a human session or machine credential. Kernels group under their machine in Cloud; sharing a root does not enroll another kernel.

The kernel stores its profile, credential and renewable relay token in `cloud-relay.json` under its private runtime state root, atomically with mode 0600. An explicit empty record prevents a legacy shared profile from reappearing after unlink. Separate accounts require separate roots/profiles. A state file belonging to another kernel fails loudly. `CHARIOX_HOME` selects CLI preferences and the CLI's persistent relay key, as well as the kernel root. CLI preferences contain public display metadata only, have mode 0600, and scrub predecessor credentials when loaded or saved.

`CloudRelayStatus` and enrollment/pairing responses contain public metadata and `kernel_enrolled`; they never return enrollment credentials, machine credentials or human sessions. `ConnectCloudRelay` configures the kernel itself and returns public status/profile, without its relay token. A second attached terminal reads that status from the kernel rather than relying on its local preferences. CLI owner-control requests require protocol 478.

An attached owner's terminal can pivot into another fresh, canonical kernel in the same account through a short kernel-issued terminal grant. It must supply its persistent CLI key thumbprint. The kernel checks the authenticated owner and the encrypted relay sender key; Cloud checks the exact target, account, key, action subset, realm and issuer generation. The grant expires within five minutes. Its transient subject is not persisted as a paired client or kernel profile identity. Revoking the issuing kernel retires its delegated grants. Shared-session viewers and relay peers cannot spend the kernel owner's directory/enrollment/delegation authority.

Hosted discovery intersects Cloud's authenticated My-kernels directory (all kernels of the account) with live relay presence. Discovery credentials carry only permitted kernel targets. The relay enforces those target pins for metadata counts and provider summaries as well as individual results; it retains no Cloud ownership policy. Kernel-mediated pivot is restricted to the same account.

An acknowledged `/cloud unlink` or kernel logout revokes only this kernel and then disconnects and clears its private record. Failed acknowledgement leaves the enrollment available for retry. Machine/client administration requires the corresponding Cloud control-plane authority; a kernel credential cannot revoke a whole machine or account client. Machine revocation in Cloud cascades to all kernels and their delegated tokens.

Legacy migration accepts only a previously registered kernel whose key matches Cloud's pin. It exchanges the shared machine predecessor for a private kernel credential, removes copied human/client authority, and retires legacy human sessions and tokens. Cloud makes the machine predecessor migration-only until existing sibling targets migrate, then revokes it. A new sibling must device-enroll independently. Dedicated managed bootstrap remains its existing separate grant path. Home-managed slice grants have immutable home-kernel and worker-key bindings and restricted bootstrap/runtime/recovery actions; the parent enrollment credential is never copied to the worker.

Operations that explicitly require human Cloud authority use the signed-in terminal CLIENT profile or browser session; they never borrow kernel enrollment authority. Detached terminal login, discovery and bootstrap use the private client credential store and refresh-family API. The browser Settings → Kernels & connections panel lists My kernels and signed-in terminals, with acknowledged unlink and revocation actions. Self-hosted kernel pairing requires separate operator-issued CLIENT transport authority; operator trust-realm discovery remains supported. The relay peer protocol is unchanged.

Focused validation: `cloud_kernel_ownership_local_router_drill` exercises device enrollment, public status, connect, sibling isolation, failed unlink preservation and acknowledged unlink through the real kernel router and a local synthetic Cloud HTTP fixture. Cloud DB tests cover credential generations, visibility, revocation and child bindings; relay tests cover live discovery target filtering. These fixture tests are supplementary regression checks. MP-08 / MP-10 / MP-11 acceptance also requires the real built clients, kernel and hosted relay, normal user device approval, official provider turns, DPR 1/2 captures and sustained renewal drills; fixture results alone do not establish that acceptance.

## Silent credential renewal (owner requirement, 2026-10-05 14:10 UTC)

Sign-in expiry must never interrupt active work. Kernel-delegated terminals renew before grant expiry through the issuing kernel's durable enrollment credential, independently of human sessions. The terminal retains a request-only connection to that issuing kernel after a visible pivot; it reauthenticates both active relay lanes in place, preserving the target, key, attachment and subscription. The relay accepts fresh token IDs/lifetimes only for the same authenticated client/key and bound target. Revocation still terminates active connections.

Detached CLIENT enrollment now returns a rotating refresh credential as well as a short access session. A refresh family has no fixed expiry; access expiry, including a long process absence, can be renewed silently. Rotation serializes against client/family/session locks, rechecks revocation, retains used credential hashes, and revokes the family/session/runtime tokens on reuse as a different rotation. A retried identical persisted rotation ID recovers only its still-current successor, so a lost response or process restart need not invalidate a legitimate client. Re-login retires expired predecessors too; client revocation stops every associated family.

The CLI client store is separate from public preferences and kernel state, under the CLI profile's relay directory. Atomic 0600 writes and directory synchronization protect the refresh credential and pending rotation ID. An OS lock (Linux flock, macOS lockf) serializes profile processes and releases on process death; independent profiles rotate independently. Detached login/bootstrap and background scheduling use this store through CloudClient. This path never transfers client refresh authority to a kernel. Signing in again is permitted only for revoked credentials or detected reuse, never a fixed refresh-family lifetime.


## MP-08 / MP-11 review corrections (protocol 478)

An enrolled kernel's terminal pairing link carries the noncredential marker
`cloud-client-token-required`, never its KERNEL transport token. The receiving
CLI bootstraps with the kernel owner's signed-in CLIENT profile and its persistent
relay key,
or an explicitly supplied CLIENT transport token via `--relay-token-env`.
Creation does not request an unkeyed grant. Cloud creation and redemption require
the authenticated kernel owner; shared-session membership and a pairing link
never grant enrollment authority. Redemption supplies the receiving
key and the kernel issues a short CLIENT grant bound to that key and exact
kernel target using its enrollment credential. The final paired client renews
through the kernel before the signed grant expires, preserving the same subject,
key and target while reauthenticating command and event connections in place.
The CLI reads kernel enrollment metadata for unlink even when the terminal is
also signed in; failed Cloud acknowledgement preserves the enrollment.
Generic client and machine invites reject scoped transport credentials in both Cloud and self-hosted
configurations. Self-hosted scoped relays retain
`operator-client-token-required` and their operator-provided CLIENT transport.

A lost refresh reply may recover a current successor whose original access
session has already expired. The CLI persists the successor and clears its old
pending operation before starting another rotation; that next operation is
persisted before its network request as well. A second lost reply therefore
resumes from the successor rather than replaying the predecessor forever.

Cloud token publication, kernel unlink and machine revocation hold common
destination Machine and target locks. Cross-machine delegation acquires issuer and destination locks
in sorted order, before credential locks, and rechecks identity/status after
acquisition. An unlink that follows publication sweeps the exact token; an
unlink that wins makes publication fail. Re-enrolling the same kernel ID does
not revive old token IDs. Machine revocation and machine logout also sweep
all incoming CLIENT grants to every kernel of that machine, independently of
the issuing kernel or client/browser session. The separate Settings React root
uses the shared query provider for its Kernels and Security account controls.

These focused source and local PostgreSQL/relay/browser regressions address
review findings. They do not establish fresh-machine MP-10 acceptance or close
MP-08 / MP-11 security-anchor review.

## MP-08 / MP-10 / MP-11 live Cloud relay drill

`pnpm --dir apps/cli run cloud-relay:drill` builds the CLI, enrolls a disposable
kernel through `/cloud link`, and separately signs the owner terminal in through
`/cloud login`. It restarts the enrolled kernel without editing its private
record, discovers it through the terminal's authenticated directory, and connects
using that terminal's key-bound CLIENT grant. Collaboration invitations and
membership use the private client adapter; local joins and workflow authorization
remain kernel-owned. Owner, peer and third-user session grants renew with the same
terminal key, session and exact kernel target. No printed relay token is parsed.

The live command requires a paired Cloud checkout (`CHARIOX_CLOUD_REPO`) with
installed dependencies and a disposable local PostgreSQL database (`DATABASE_URL`).
It builds/migrates that local checkout and starts its own API, relay and kernel;
it does not connect to hosted infrastructure. On a shared builder, set
`CHARIOX_RUST_COMPILE_LOCK` to its assigned compile lock and `CARGO_BUILD_JOBS=4`;
the drill builds the relay before launching its binary. Runtime state, generated
identities and client credentials use disposable directories outside the source
checkout and are removed on completion, failure or settled interruption. Public
result/failure evidence records both source commits and cleanup status under
`CHARIOX_DRILL_EVIDENCE_DIR` (default `~/.codex/evidence/live-cloud-relay/`).
`CHARIOX_CLOUD_KERNEL_CREDENTIAL_ONLY=1` ends after restart and remote session
creation/attachment/listing, omitting collaboration assertions.

`pnpm --dir apps/cli run cloud-relay:drill --check` checks built runtime imports
without starting services. `pnpm --dir apps/cli run test:cloud-relay-drill` also
exercises the actual command handlers and client adapters against synthetic local
Cloud/relay/kernel seams, including commands and encrypted events across several
grant expiries and safe child signalling. These checks do not establish a live
Cloud/PostgreSQL run, hosted transport or fresh-machine MP-10 acceptance.
