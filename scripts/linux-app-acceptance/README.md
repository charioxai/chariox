# Private Linux installed-artifact acceptance

This B6 drill uses a fresh owned QEMU VM. It never changes the builder's runtime
enrollment, AppArmor profiles, storage helper, domains, or provider accounts.
Run QEMU, Cargo and other heavy commands through the builder's `slot-run` helper.
Rust 1.88.0 is the baseline. No hosted-CI environment is synthesized.

The included fixture is a signed **text graph**. It verifies installer mechanics:
trust/digest refusal, normal root provisioning, idempotency, upgrades, runtime
leases, cleanup, installed ownership and modes. It cannot run an App and does
not qualify release signing, native containment, reference Rooms or workflows.
Use a separately reviewed executable runtime for those legs; retain both sets
of receipts and their scopes.

## Pin sources independently

Record the PR base, OSS candidate, Cloud candidate, protocol, runtime inventory,
image SHA-256 and test results in external evidence. This drill can live on an
older PR base while testing a private candidate assembled from g plus reviewed
#727/Cloud #273 patches. The normal installer scripts are an additional required
input: Linux installer #599 (`apps/p1-linux-installer`), already included in the
validated g commit `065c99b7f4f9e63ecf42040ceb8c193e33267068`. They are absent
from this PR base and #727; use that g commit or explicitly include #599.
Call the result a branch candidate, never live h. Keep candidates
inside the assigned checkout, excluded from Git, and all generated state and
build output in owned external scratch.

## Provision the owned VM

Use a signature-verified stock Linux cloud image with systemd/cgroup v2 and
AppArmor, an ordinary `b6` user without sudo or supplementary groups, and QEMU
TCG if KVM is unavailable. Set the hostname to `b6-linux-acceptance`. Allocate
only the task's own disk/seed/socket/serial paths, ports and memory. A tested
configuration is Ubuntu 26.04, systemd 259, two vCPUs, 4 GiB RAM and a 24 GiB
qcow2 overlay.

Install `qemu-guest-agent` in the guest and expose it on an owned host Unix
socket through `org.qemu.guest_agent.0`. `guest.py` submits guest-exec commands
without SSH credentials or copied provider auth. It reports timeouts with the
guest PID; a timeout does not cancel that command. Clean up that owned process
before retrying a mutation. The client synchronizes every new connection using
[QGA's delimited handshake](https://www.qemu.org/docs/master/interop/qemu-ga-ref.html#command-guest-sync-delimited)
so a stale reply from a timed-out client cannot become the next command result.
Truncated output fails the drill.

Mount the task's own `share/` read-only at `/mnt/b6share`, using QEMU's `b6share`
9p tag. Cloud-init may provision packages and mount this share. Supply all of:

- `bin/chariox-kernel`, `bin/chariox-app-runtime-install`,
  `bin/chariox-app-storage` and `bin/chariox-app-package`, built from the candidate;
- the candidate's complete `deploy/local-linux/` and `deploy/managed-kernel/`
  directories (the root installer also needs its Docker admission unit);
- `installer-fixture/{v1,v2}/` and the separate public `installer-fixture.json`
  authority record, generated below.

The VM must have no existing Chariox enrollment or installation. Do not reuse
another lane's VM, filesystem or keys. Install-script retries leave acknowledged
state intact; retain failures and use the normal uninstall path to reset an owned
VM before rerunning the complete fresh-install drill.

## Generate and run the mechanics fixture

All three paths are absolute. The output directory must be new, outside Git,
and owned by this task. Evidence must already exist in the assigned directory.

```sh
node scripts/linux-app-acceptance/fixture.mjs \
  /absolute/owned/candidate /absolute/owned/share/installer-fixture \
  /absolute/assigned/evidence
cp /absolute/assigned/evidence/installer-fixture.json /absolute/owned/share/
bash scripts/linux-app-acceptance/installer.sh /absolute/owned/vm/guest.sock \
  > /absolute/assigned/evidence/installer-mechanics.log 2>&1
```

The Ed25519 runtime fixture key is generated in memory and never persisted.
Only its public key, inventory digests and source commit enter evidence. The
ordinary App publisher is separate: generate it in the assigned evidence
directory, pack fresh owned reference copies, enroll its public key only in the
throwaway kernel, and remove the private seed before evidence handoff.

The installer drill leaves v2 enrolled and the storage helper/lingering enabled
inside the VM. Start the ordinary user's kernel through `install-user.sh`,
record its ready response and delegated cgroup controllers, and test boot,
offline/idempotent operation and uninstall there. This text fixture supplies no
App data or due wake, so kernel startup/reboot cannot qualify data/wake recovery.

## Remaining executable-runtime matrix

Record PASS/FAIL/BLOCKED per leg. Contract tests, accelerated clocks, direct
backend calls and stub brokers are scoped evidence, not replacements for:

- Online/offline installed runtime, upgrade/uninstall, boot data and due wakes,
  mixed supported clients and actionable protocol/SDK incompatibility.
- Topology 1 and separately protected topology 7; real web/local/relay viewers
  sharing the same managed Tab/backend, native text/click/scroll, official
  provider takeover/reconnect and exact Todo/Documents results.
- One accepted reminder occurrence and one actual automation workflow run.
- A relay-client joint update with the view open: atomic catalog/generation,
  retained data/config/drafts, queued events and kernel refusal of the old
  bridge/API/cache. DOM context destruction alone does not prove refusal.
- Native 60s refresh, 5min freshness and 60min offline expiry, bounded by campaign
  end, with draft retention. Do not shorten defaults to close this row.
- A visible missing workflow binding after uninstall, followed by recovery.

Critical Approve, real Slack/Google account authority, CI-key release signing and
clean-Mac notarized installer acceptance remain owner/CI legs. Do not mint or copy
credentials to unblock them. Preserve the incomplete rows in the ledger.

Before handoff, stop only the task's VM/containers/kernel, uninstall its fixtures,
remove unused large build outputs, write the assigned evidence `REPORT.md` and
checksums, and obtain the Mac evidence receipt before builder retirement.

## Local Docker enrollment on noexec /run

Run `noexec-docker-enrollment.sh INSTALLER INSTALLER_ARGS...` as root inside
an owned Linux VM, forwarding the reviewed public input pins documented in
`deploy/local-linux/LOCAL_DOCKER_DEV.md`. It executes the real enrollment, checks
that `/run` remains `noexec`, and rejects Buildx scratch left after success.
Keep the baseline failure and successful regression output in task evidence.
