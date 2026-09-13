# Managed kernel Linux validation and release runbook

Status: preparation only, 2026-09-13. The source under test is the exact OSS
commit `70e1f4c5916747bb42d69f722cf02968d51382eb` (the catalog fix plus the
rustfmt-only correction). This runbook was prepared without Cargo, a build,
Docker, CI, deployment, service mutation, or credential access.

The authorized original rented Linux host may run the non-destructive source
track: a clean independent worktree, direct Cargo compilation, and read-only
source/Node checks inside the bounded external target/cache below. It must not
touch the protected home kernel, `/var/lib/chariox/home`, current release, or
shared services. Use disposable targets for the invasive track: image/account
installation, rootless Docker/resource/lifecycle drills, managed upgrade,
live-provider drills, and any deployment. Signed release building remains on a
separately controlled builder. `install-image.sh` and `upgrade-image.sh` are
root operations; an override root does not isolate their `systemctl`, account,
or filesystem effects. The commands in this document are future gates and
were not run during preparation. An approved real deployment remains a
separate, explicitly gated operator action and is not authorized by this
runbook.

## Source and task-local paths

The source checkout is the Git worktree. Keep generated Cargo targets/caches,
evidence, build output, and release output outside that source worktree. These
are the exact task-scoped paths used by the commands below:

```sh
export SOURCE_COMMIT=70e1f4c5916747bb42d69f722cf02968d51382eb
export KERNEL_VALIDATION_ROOT=/var/lib/chariox/validation/kernel-validation-prep-20260913
export KERNEL_WORKTREE="$KERNEL_VALIDATION_ROOT/worktree"
export CARGO_HOME="$HOME/.cargo"
export CARGO_TARGET_DIR="$KERNEL_VALIDATION_ROOT/cache/cargo-target"
export EVIDENCE_ROOT="$KERNEL_VALIDATION_ROOT/evidence/$SOURCE_COMMIT"
export BUILD_OUTPUT="$KERNEL_VALIDATION_ROOT/build/$SOURCE_COMMIT"
export RELEASE_OUTPUT="$KERNEL_VALIDATION_ROOT/release/$SOURCE_COMMIT"
export VALIDATION_LOCK="$KERNEL_VALIDATION_ROOT/validation.lock"
```

The retained `CARGO_HOME` is operator-owned and must resolve outside
`/var/lib/chariox/home`; stop if it overlaps that protected path. The
validation root, worktree, target, evidence, and release paths must all be
task-scoped and non-shared. `CARGO_HOME` is the retained reusable operator
cache; preserve it and serialize any writes through the validation lock.

The worktree must be an independent checkout at `SOURCE_COMMIT`, with no staged,
unstaged, or untracked source changes. On a fresh validation host, create it
from that host's existing product-managed OSS Git store; do not reuse a
protected or another agent's worktree:

```sh
case "$KERNEL_VALIDATION_ROOT" in
  /var/lib/chariox/home|/var/lib/chariox/home/*) echo "validation root overlaps protected home" >&2; exit 1 ;;
esac
case "$(realpath -m "$CARGO_HOME")" in
  /var/lib/chariox/home|/var/lib/chariox/home/*) echo "Cargo home overlaps protected home" >&2; exit 1 ;;
esac
mkdir -p "$KERNEL_VALIDATION_ROOT" "$CARGO_TARGET_DIR" "$EVIDENCE_ROOT"
git -C <existing-oss-git-store> worktree add --detach "$KERNEL_WORKTREE" "$SOURCE_COMMIT"
test "$(git -C "$KERNEL_WORKTREE" rev-parse HEAD)" = "$SOURCE_COMMIT"
test -z "$(git -C "$KERNEL_WORKTREE" status --porcelain)"
```

Reuse the retained operator `CARGO_HOME`; do not create a new dependency cache
per source SHA, delete the existing cache, or inspect credential files. The
external target is task-scoped and may be reused on reruns for the same
operator/toolchain; do not share it with a release build or another Cargo
process. The focused command deliberately uses `--locked` without forcing
`--offline`: Cargo can reuse a valid cache, while a cache miss can follow the
operator's normal approved registry policy instead of being forced into an
avoidable offline failure. If a no-network repeat is explicitly desired after
the cache is known complete, add `--offline`; if a required package is missing
or a permitted fetch fails, preserve the cache, record the exact dependency and
error in evidence, and stop unless a separately approved dependency-fetch
window is available. This runbook does not install packages or dependencies.

## Host gate and resource policy

The focused test needs a Linux C compiler/linker, `pkg-config`, OpenSSL and
DBus development files, and `protobuf-compiler`; the pinned release Dockerfile
names `build-essential`, `libssl-dev`, `libdbus-1-dev`, `pkg-config`, and
`protobuf-compiler`. Require these to be preinstalled. Also require Git,
`flock`, `systemd-run`, cgroup v2 CPU/memory/PIDs controls, and Node.js 22 or
newer for the release scripts. The resource-drill track additionally needs
Python 3, `jq`, `sudo`, and the disposable target's rootless-Docker
prerequisites.
Do not install anything in this runbook.

There is no `rust-toolchain.toml` and no `rust-version` field in the checked-in
Cargo manifests. The repository's effective CI/release minimum is Rust
`1.88.0`, and the signed release image pins
`rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0`.
Those are the compatibility/release pins, not a demand to replace the
operator's retained toolchain. The authorized operator toolchain is Rust/Cargo
`1.98.1`; verify that exact pair before a source compile and record the verbose
host/target details:

Before a future validation window, check the host without starting a build:

```sh
test "$(uname -s)" = Linux
uname -m
command -v rustc cargo node git flock systemd-run jq
export RUSTUP_TOOLCHAIN=1.98.1
test "$(rustc --version | awk '{print $2}')" = 1.98.1
test "$(cargo --version | awk '{print $2}')" = 1.98.1
rustc -Vv
node --version
test -r /sys/fs/cgroup/cgroup.controllers
grep -qw cpu /sys/fs/cgroup/cgroup.controllers
grep -qw memory /sys/fs/cgroup/cgroup.controllers
grep -qw pids /sys/fs/cgroup/cgroup.controllers
free -h
df -h "$KERNEL_WORKTREE" "$KERNEL_VALIDATION_ROOT" "$CARGO_HOME"
```

Reserve at least 12 GiB of RAM for the test scope plus 4 GiB for the host and
at least 10 GiB of free disk for the external target and evidence. Stop if
either reserve is unavailable. Hold `VALIDATION_LOCK` for every Cargo, build,
package, or upgrade operation; no two such operations may run concurrently.
The original rented host is acceptable for the focused source track only when
the protected-kernel/shared-service inventory remains untouched and the test
process is inside one of the bounded scopes below.

## Focused kernel regression

The actual focused Cargo test is:

```sh
cd "$KERNEL_WORKTREE"
export CARGO_BIN="$(command -v cargo)"
flock -n "$VALIDATION_LOCK" \
  systemd-run --user --scope --quiet --wait --collect \
  --property=MemoryMax=12G \
  --property=MemorySwapMax=0 \
  --property=CPUQuota=150% \
  --property=TasksMax=512 \
  --property=RuntimeMaxSec=30min \
  --setenv=CARGO_HOME="$CARGO_HOME" \
  --setenv=CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  --setenv=CARGO_BUILD_JOBS=2 \
  --setenv=CARGO_INCREMENTAL=1 \
  --setenv=RUSTUP_TOOLCHAIN=1.98.1 \
  --setenv=RUST_TEST_THREADS=1 \
  --setenv=RUST_MIN_STACK=16777216 \
  "$CARGO_BIN" test --locked --manifest-path apps/kernel/Cargo.toml \
    --lib terminal_command_catalog_includes_room_environment_status -- --nocapture
```

This is the `#[cfg(test)]` test in
`apps/kernel/src/runtime/terminal_command_catalog.rs`. It loads the
`include_str!` JSON fragments, checks the `/room` child order, and checks the
`room-bind` value, description, group kind, kernel target, session surface,
examples, and search aliases. `--lib` and the name filter reduce test execution;
Cargo still compiles the kernel library and its normal dependencies, including
the path dependencies `chariox-relay` and `chariox-event-protocol` and bundled
`rusqlite`.

`apps/kernel/Cargo.toml` declares no `[features]` section, so no `--features`
flag is needed: use the default Linux dependency set and the locked workspace.
The Windows-only dependency section is not selected on Linux. The test command
is not the root `test:daemon` or a full workspace test.

The user-systemd scope and the root-managed transient service are the two
bounded max-2/incremental Linux paths described here. The checked-in
`apps/cli/scripts/lib/local-rust-fault-drill-runtime.mjs` helper is serial
(`CARGO_BUILD_JOBS=1`) and has no Linux memory cap; the managed release
Dockerfile's release and dev Cargo invocations are also serial, set
`CARGO_INCREMENTAL=0`, and have no host memory property. Do not claim either
existing path provides this policy or silently fall back to an unbounded Cargo
invocation when user cgroup delegation is unavailable.

If the operator user lacks delegated controllers, run the same non-destructive
test as a transient root-managed systemd service. This creates only a
task-named validation unit, runs Cargo as the operator, and does not stop,
reload, enable, or restart a product service. Do not run this block during
preparation; verify the protected-home exclusion and the exact toolchain first:

```sh
export OPERATOR_USER="$(id -un)"
export OPERATOR_GROUP="$(id -gn)"
export CARGO_BIN="$(command -v cargo)"
export OPERATOR_PATH="$(dirname "$CARGO_BIN"):/usr/local/bin:/usr/bin:/bin"
test "$(rustc --version | awk '{print $2}')" = 1.98.1
test "$(cargo --version | awk '{print $2}')" = 1.98.1
test "$(realpath -m "$KERNEL_WORKTREE")" != /var/lib/chariox/home
case "$(realpath -m "$KERNEL_WORKTREE")" in
  /var/lib/chariox/home/*) echo "worktree overlaps protected home" >&2; exit 1 ;;
esac
sudo systemd-run --system \
  --unit=chariox-kernel-catalog-test-20260913 \
  --wait --collect \
  --property=User="$OPERATOR_USER" \
  --property=Group="$OPERATOR_GROUP" \
  --property=WorkingDirectory="$KERNEL_WORKTREE" \
  --property=MemoryMax=12G \
  --property=MemorySwapMax=0 \
  --property=CPUQuota=150% \
  --property=TasksMax=512 \
  --property=RuntimeMaxSec=30min \
  --setenv=PATH="$OPERATOR_PATH" \
  --setenv=CARGO_HOME="$CARGO_HOME" \
  --setenv=CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  --setenv=CARGO_BUILD_JOBS=2 \
  --setenv=CARGO_INCREMENTAL=1 \
  --setenv=RUSTUP_TOOLCHAIN=1.98.1 \
  --setenv=RUST_TEST_THREADS=1 \
  --setenv=RUST_MIN_STACK=16777216 \
  /usr/bin/flock -n "$VALIDATION_LOCK" \
  "$CARGO_BIN" test --locked --manifest-path "$KERNEL_WORKTREE/apps/kernel/Cargo.toml" \
    --lib terminal_command_catalog_includes_room_environment_status -- --nocapture
```

The root-managed alternative requires the system manager to expose the CPU,
memory, and PID controllers; if it cannot apply any property, fail closed.
Record the transient unit's exit status and peak memory/CPU/task readings.
Neither path is an invasive install or lifecycle drill, and both keep all
generated output in the external target/cache.

## Discovery seam and snapshot impact

The CLI already sends `/room bind SLICE` through the shared terminal-command
catalog discovery/autocomplete seam. This gate must remain catalog-only; do not
add a second command list or parallel command dispatch logic. The separate
public CLI contract tests are in
`apps/cli/src/room-binding-command.test.ts` and exercise the existing parser
and `handleRoomSlashCommand` request path.

The catalog revision is a SHA-256 of the enriched catalog nodes, so adding the
child changes the reported catalog revision by design. The local daemon
protocol remains 323 and the protocol-shape test uses a synthetic catalog
fixture; no serialized protocol shape or fixed terminal-catalog snapshot needs
an update. If a downstream consumer records a revision, record the new value as
derived evidence rather than pinning a second catalog or weakening the focused
assertion.

## Project-setup branch integration

Agent92 owns `codex/project-environment-setup-20260913`. Do not fetch, modify,
merge, or test that branch during this preparation. After Agent92 publishes a
reviewed branch, make a new integration worktree; keep this runbook worktree
and the source pin unchanged:

```sh
export PROJECT_SETUP_BRANCH=codex/project-environment-setup-20260913
export INTEGRATION_WORKTREE=/var/lib/chariox/validation/kernel-project-setup-integration-20260913
git -C <existing-oss-git-store> fetch origin \
  "refs/heads/$PROJECT_SETUP_BRANCH:refs/remotes/origin/$PROJECT_SETUP_BRANCH"
git -C <existing-oss-git-store> worktree add --detach "$INTEGRATION_WORKTREE" \
  "origin/$PROJECT_SETUP_BRANCH"
git -C "$INTEGRATION_WORKTREE" merge --no-ff --no-commit "$SOURCE_COMMIT"
```

Resolve and review the merge, then record its resulting full SHA before running
gates. If the branch already contains `SOURCE_COMMIT`, the merge should be
empty; do not duplicate the catalog implementation. Run the branch's named
public contract tests at that resulting merge commit. For the current public
Room binding contract, the exact future Node gate is:

```sh
cd "$INTEGRATION_WORKTREE"
flock -n "$VALIDATION_LOCK" pnpm --filter @chariox/cli run build
flock -n "$VALIDATION_LOCK" node --test apps/cli/dist/room-binding-command.test.js
```

The build and Node test above are future-only and were not run here. They need
preinstalled workspace dependencies and use the CLI's Babel TypeScript
transform; do not run `node --test` directly on the `.ts` source and call it a
pass. If the forthcoming branch adds differently named public contract tests,
run only those explicit generated `dist/*.test.js` paths in addition to the
Room binding test. Source-only JSON/grep checks are evidence of file content,
not Rust or TypeScript contract execution.

## Signed release preparation (future, isolated builder only)

The release path is intentionally separate from the bounded debug/test path.
`scripts/build-managed-kernel-release.mjs` materializes the exact Git tree and
invokes the pinned `rust:1.88-bookworm` Docker builder. Its Dockerfile hard-codes
one release Cargo job and `CARGO_INCREMENTAL=0`; the script itself has no
`MemoryMax`, max-2, or incremental-cache option. Run it only on a disposable,
resource-capped builder whose Docker daemon and build containers are inside the
same cgroup. A systemd wrapper around a rootful daemon is not proof of a
container memory cap.

With the source and dependency inputs already present on that isolated builder,
the source-identity and build command are:

```sh
export BUILDER_SIGNING_KEY=/var/lib/chariox/keys/openship-builder-ed25519
test "$(git -C "$KERNEL_WORKTREE" rev-parse HEAD)" = "$SOURCE_COMMIT"
test ! -e "$BUILD_OUTPUT"
mkdir -p "$(dirname "$BUILD_OUTPUT")"
flock -n "$VALIDATION_LOCK" \
node "$KERNEL_WORKTREE/scripts/build-managed-kernel-release.mjs" \
  --source-repository "$KERNEL_WORKTREE" \
  --source-commit "$SOURCE_COMMIT" \
  --builder-signing-key "$BUILDER_SIGNING_KEY" \
  --output "$BUILD_OUTPUT"
```

The builder key must be an external root-owned Ed25519 PKCS8 key with safe
permissions. Never print, copy, or include a private key in arguments,
environment evidence, logs, or the release. If the builder cannot prove the
Docker cgroup limit, stop; do not substitute a host build or change the
Dockerfile's deterministic release settings.

Package only those attested outputs and sign the package with a separate
external release key:

```sh
export TRUSTED_BUILDER_PUBLIC_KEY=/var/lib/chariox/keys/openship-builder-public
export RELEASE_SIGNING_KEY=/var/lib/chariox/keys/managed-release-ed25519
export SOURCE_DATE_EPOCH="$(git -C "$KERNEL_WORKTREE" show -s --format=%ct "$SOURCE_COMMIT")"
test ! -e "$RELEASE_OUTPUT"
mkdir -p "$(dirname "$RELEASE_OUTPUT")"
SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
flock -n "$VALIDATION_LOCK" \
node "$KERNEL_WORKTREE/scripts/package-managed-kernel-release.mjs" \
  --kernel "$BUILD_OUTPUT/chariox-kernel" \
  --supervisor "$BUILD_OUTPUT/chariox-managed-bootstrap" \
  --relay "$BUILD_OUTPUT/chariox-relay" \
  --builder-attestation "$BUILD_OUTPUT/build-attestation.json" \
  --builder-attestation-signature "$BUILD_OUTPUT/build-attestation.sig" \
  --trusted-builder-public-key "$TRUSTED_BUILDER_PUBLIC_KEY" \
  --signing-key "$RELEASE_SIGNING_KEY" \
  --source-repository "$KERNEL_WORKTREE" \
  --source-commit "$SOURCE_COMMIT" \
  --output "$RELEASE_OUTPUT"
```

Save the single `sha256:` line printed by the packager as `RELEASE_DIGEST`,
then verify the generated `rootfs` with the trusted release public key kept
outside the image:

```sh
export RELEASE_DIGEST=sha256:<64-lowercase-hex-digest-printed-by-packager>
export TRUSTED_RELEASE_PUBLIC_KEY=/var/lib/chariox/keys/managed-release-public
node "$KERNEL_WORKTREE/deploy/managed-kernel/verify-image-release.mjs" \
  "$RELEASE_OUTPUT/rootfs" "$RELEASE_DIGEST" "$TRUSTED_RELEASE_PUBLIC_KEY"
jq -e --arg commit "$SOURCE_COMMIT" \
  '.sourceCommit == $commit' \
  "$RELEASE_OUTPUT/rootfs/usr/lib/chariox/release-manifest.json"
```

The verifier checks the manifest digest/signature, exact artifact set and
digests, builder attestation files, and safe regular-file/tree shape. Record
the source commit/tree, all three binary digests, builder attestation digest,
release digest, signer fingerprints, and verifier exit status.

## Disposable install, upgrade, and rollback

`prepare-hetzner-image.sh` installs packages, creates accounts, changes systemd
state, and provisions rootless Docker. It is future-only on a marked disposable
x86_64 image builder; it must never run on the protected home host. The
resource probe and rootless lifecycle drill are separate future gates and must
remain disposable, offline, and explicitly owned. On that disposable Linux VM,
the existing resource probes are:

```sh
python3 "$KERNEL_WORKTREE/scripts/managed-rootless-resource-drill.py" cgroupfs
python3 "$KERNEL_WORKTREE/scripts/managed-rootless-resource-drill.py" systemd
sudo python3 "$KERNEL_WORKTREE/scripts/managed-rootless-lifecycle-drill.py" <ordinary-test-user>
```

They validate cgroup-backed rootless Docker limits (the shipped probe caps its
own service at 384 MiB, half a CPU, and a 90-second deadline). These commands
start or stop only drill-owned resources and were not run in this preparation;
they are not a substitute for the focused Cargo gate.

For an already provisioned, drill-owned managed target, `upgrade-image.sh` is
the signed offline upgrade seam. It verifies the current and new release,
protocol transition policy, receipt identity, and slice-build-context facade;
it then stops the managed supervisor, atomically publishes the release/receipt
links, starts the supervisor, checks fresh loopback presence/health, and
automatically rolls back on activation, health, or final receipt failure. Run
only on that isolated target, with the target's root-owned trusted keys and
receipt; never point these arguments at the protected home kernel:

```sh
flock -n "$VALIDATION_LOCK" \
sudo "$KERNEL_WORKTREE/deploy/managed-kernel/upgrade-image.sh" \
  "$RELEASE_OUTPUT/rootfs" \
  "$CURRENT_RELEASE_DIGEST" \
  "$RELEASE_DIGEST" \
  "$CURRENT_TRUSTED_PUBLIC_KEY" \
  "$NEXT_TRUSTED_PUBLIC_KEY"
```

The script's default receipt and install paths are target-local. If a
drill-owned allocation-worker receipt is used, pass its explicit
`CHARIOX_MANAGED_UPGRADE_RECEIPT`; do not let it discover or alter another
home's receipt. On failure, distinguish `restored previous managed kernel
release` from `rollback remains pending`. A pending transaction is evidence to
retain for recovery, not a reason to delete `.managed-kernel-upgrade` or flip
`current` by hand.

If an authorized manual rollback is required and the previous signed rootfs is
retained, invoke the same transaction seam in reverse. The new release key is
trusted for the installed current release and the old key is supplied for the
rollback target:

```sh
flock -n "$VALIDATION_LOCK" \
sudo "$KERNEL_WORKTREE/deploy/managed-kernel/upgrade-image.sh" \
  "$PREVIOUS_RELEASE_ROOTFS" \
  "$RELEASE_DIGEST" \
  "$PREVIOUS_RELEASE_DIGEST" \
  "$NEXT_TRUSTED_PUBLIC_KEY" \
  "$CURRENT_TRUSTED_PUBLIC_KEY"
```

Confirm the restored `current` digest, receipt digest, protocol, signed
context facade, fresh presence, and health before declaring rollback complete.
Retain the previous release through the validation window; no service is
stopped, started, enabled, or restarted by this preparation.

## Evidence and cleanup

Create evidence only below `EVIDENCE_ROOT` with mode `0700`. Each report must
include the exact source commit/tree, clean-worktree result, command and
environment (including `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=1`, and all
cgroup limits), Rust/Node versions, cache/target paths, exit status, test names,
resource peak/current readings, disk headroom, and cleanup inventory. Release
evidence additionally includes attestation/signature verification, manifest
source identity, release digest, upgrade precondition/current/target digests,
health result, and rollback result. Redact all tokens, cookies, prompts,
provider state, and private-key material.

The current preparation has only source-level evidence. It must not be reported
as a passing Rust test, TypeScript execution, signed release, or deployment.
After a successful isolated run, retain the signed release and prior release
until the rollback window closes. Then, under the same lock, remove only the
task-scoped staging output and evidence selected for deletion; never prune a
shared Cargo cache, Docker state, service, release, or protected home. On any
failed or interrupted upgrade, preserve the transaction and evidence until
recovery is independently verified.
