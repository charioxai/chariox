# MP-03/MP-08/MP-10/MP-11: fresh-VM Start restoration replay

This replays b201's M20-11 failure: Save succeeds, the original container and
home volume are removed, and normal Start must restore the saved home through
a fresh protected generation. Use a coordinator-supplied disposable VM only.
The previous VM was deleted; the source regression is not live acceptance.
DEV enrollment is unsigned and does not close any MP item.

## MP-03/MP-10: public inputs and host preparation

Transfer only the reviewed public source, compatible native kernel, standard
Dockerfile worker image, standalone Node 22 and Buildx executables, and their
public hash/provenance receipt. Build the worker on an adequately sized builder;
the earlier 8 GiB VM could not safely compile the kernel. Verify native binaries
against the worker's Bookworm ABI; do not reuse the earlier ABI-incompatible
builder-native binary. Do not copy accounts, configuration, keys or archives.

On the new VM, as root, set these absolute paths and public pins from that
receipt. `WORKER_IMAGE` must be an immutable `sha256:` image ID. The worker
revision identifies the actual image source and may differ from the installed
helper source; record both without relabelling either.

```sh
export SOURCE=/opt/chariox-b201-source
export PUBLIC=/opt/chariox-b201-public
export NODE="$PUBLIC/node/bin/node"
export BUILDX="$PUBLIC/docker-buildx"
export KERNEL="$PUBLIC/chariox-kernel"
export PATH="$PUBLIC/node/bin:/usr/bin:/bin"
: "${WORKER_IMAGE:?public receipt image ID}"
: "${WORKER_KERNEL_SHA256:?public receipt kernel hash}"
: "${WORKER_REVISION:?public receipt runtime-source-revision label}"
: "${DOCKER_SHA256:?reviewed host /usr/bin/docker hash}"
: "${NODE_SHA256:?reviewed standalone Node hash}"
: "${BUILDX_SHA256:?reviewed standalone Buildx hash}"
cloud-init status --wait
git -C "$SOURCE" rev-parse HEAD
sha256sum "$KERNEL" "$NODE" "$BUILDX" /usr/bin/docker
"$KERNEL" --print-local-daemon-protocol-version
docker image inspect --format '{{.Id}} {{index .Config.Labels "io.chariox.runtime-source-revision"}}' "$WORKER_IMAGE"
docker run --rm --read-only --network none --memory 64m --pids-limit 16 \
  --entrypoint /usr/bin/sha256sum "$WORKER_IMAGE" /opt/chariox-slice/bin/chariox-kernel
useradd --create-home --shell /bin/bash --groups docker b201replay
python3 "$SOURCE/deploy/local-linux/provision-docker-admission-locks.py"
apparmor_parser -r "$SOURCE/apps/kernel/slice-linux-docker/chariox-slice-provider.apparmor"
python3 "$SOURCE/deploy/local-linux/install-local-docker-dev.py" \
  --allow-provider-sandbox-compatibility --source "$SOURCE" --user b201replay \
  --worker-image "$WORKER_IMAGE" --worker-kernel-sha256 "$WORKER_KERNEL_SHA256" \
  --worker-runtime-revision "$WORKER_REVISION" --docker-cli-sha256 "$DOCKER_SHA256" \
  --node-runtime "$NODE" --node-runtime-sha256 "$NODE_SHA256" \
  --buildx-runtime "$BUILDX" --buildx-runtime-sha256 "$BUILDX_SHA256"
cd "$SOURCE"
pnpm install --frozen-lockfile
pnpm --workspace-root run build:kernel-client
```

Keep loader/enrollment refusal as RED; do not chmod/adopt incompatible ancestry
or disable admission/probes. Record the new host's SSH public fingerprint,
source commit, binary hashes, image ID/revision, installer exit and resources.
The supplied public kernel must match the generated client's protocol. A
pre-fix enrollment contains immutable old helpers: install the fixed source on
a fresh VM/user, rather than editing its protected source in place.

## MP-03/MP-10: bounded isolated archive fixture

Only the operator checks protected archives or injects the corrupt candidate.
The ordinary replay user does not read or export them. This fresh root-owned
directory must not already exist. Validate the grant before publishing it.

```sh
mkdir -m 0755 /opt/chariox-b201-fixture
install -o root -g root -m 0555 "$SOURCE/deploy/local-linux/storage-qualification-fixture.py" \
  /opt/chariox-b201-fixture/storage-qualification-fixture.py
python3 - <<'PY'
from pathlib import Path
p = Path('/opt/chariox-b201-fixture/sudoers')
commands = ', '.join('/usr/bin/python3 /opt/chariox-b201-fixture/storage-qualification-fixture.py ' + action
                     for action in ('verify', 'corrupt', 'quarantine'))
p.write_text('b201replay ALL=(root) NOPASSWD: ' + commands + '\n')
p.chmod(0o440)
PY
visudo -cf /opt/chariox-b201-fixture/sudoers
install -o root -g root -m 0440 /opt/chariox-b201-fixture/sudoers /etc/sudoers.d/chariox-b201-fixture
```

## MP-08/MP-10: executable Save/Start/Backup replay

Run once in this enrollment with fresh state. The wrapper clears ambient
provider configuration, scopes empty provider homes, sets an absolute
`CHARIOX_HOME` through M20, and lets the ordinary kernel start its broker.
No provider login is required. The original deleted VM address is not reusable.

```sh
sudo -u b201replay env \
  PATH="$PUBLIC/node/bin:/usr/bin:/bin" \
  CHARIOX_STORAGE_QUALIFICATION_ISOLATED_HOST=1 \
  M20_KERNEL_BINARY="$KERNEL" M20_SLICE_IMAGE="$WORKER_IMAGE" \
  M20_RUNTIME_ROOT=/home/b201replay/.chariox/dev/storage-qualification/restore-1 \
  M20_ARTIFACT_DIR=/home/b201replay/.codex/evidence/storage-qualification/restore-1 \
  M20_STORAGE_FIXTURE_HELPER=/opt/chariox-b201-fixture/storage-qualification-fixture.py \
  CHARIOX_SLICE_APPARMOR_PROFILE=chariox-slice-provider \
  bash "$SOURCE/deploy/local-linux/replay-storage-qualification.sh"
```

Record the exact command and exit. Sample VM memory/disk every five seconds;
budget a 2 GiB slice plus the 512 MiB restore helper and home kernel, and retain
at least 10 GiB disk reserve. Signal only a verified owned drill PID greater
than 1 if a resource limit approaches. Never signal a process group or PID 1.

The replay stops the slice after making the corrupt candidate, writes public
`corrupt-candidate-response.json`, and waits up to ten minutes for the isolated
root operator. In a second root terminal, select the **disposable** backup ID
and generation from the observed candidate capture and protected capture
coordinates. Confirm they differ from the known-good baseline. A user manifest
name alone is not authorization. Then pin those exact coordinates:

```sh
: "${DISPOSABLE_BACKUP_ID:?operator-selected disposable backup ID}"
: "${DISPOSABLE_GENERATION:?operator-selected generation directory name}"
python3 /opt/chariox-b201-fixture/storage-qualification-fixture.py \
  authorize-corruption "$(id -u b201replay)" "$DISPOSABLE_BACKUP_ID" "$DISPOSABLE_GENERATION"
```

This command is never in the user sudo grant. It verifies protected metadata,
archive digest/size and inode and creates an exclusive root-owned mode-0600
authorization file. Existing authorization is not replaced. The user helper
must match its pinned ID, full generation path, digest, size and inode; changing
a user manifest to `corrupt-candidate` cannot authorize a baseline archive.
Absent authorization times out as RED without mutation or restore. Do not copy
the authorization from another run or erase it to authorize an unknown target.

Required reached phases: protected capture verification; container/home
removal; **normal Start**; running restored worker; browser/download/editor and
identity checks; named backup capture; corrupt-candidate refusal/preservation;
restore by name; repeated restore by ID; offline service-worker check; cleanup.
The new controller must select `chariox-slice-…-home-g…` after removal and resolve
its initialization journal. A later named-backup restore must start a fresh
transaction, preserving its normal kernel acknowledgement contract.

If any phase fails, retain its first error and mark subsequent phases
NOT_REACHED. A manifest's declared assertion list is not execution evidence.
Use only allowlisted public projections/metadata, never dump enrollment,
controller config, identity files, Vault data or archive contents. Keep operator
receipts limited to generation phase, volume names and archive verification
results; do not publish the restore environment.

Protected DEV expects the broker-owned corrupt archive to remain in place with
zero quarantine files after refusal and no container replacement. Ordinary
unprotected local storage retains its separate quarantine expectation. This
fixture follows the existing topology policy; it does not change that policy.

## MP-03/MP-10/MP-11: cleanup and limits

Inspect the manifest cleanup and independently check **all** exact owned home
generations, workspace volumes, containers, state/backup images and listeners.
The fixture's default-volume absence check alone does not prove generated-volume
cleanup. Preserve protected private roots and identity backups when product
Delete leaves them; do not repair/remove protected parents to repeat a run.
DEV owned-uninstall has no supported contract yet. Remove only this isolated
fixture's sudoers grant after its owned commands settle. Retain public evidence,
record remaining exact owned resources for coordinator retirement, then write
`VM_DONE` in the lane status. The coordinator deletes the VM.

Restart the isolated Docker daemon only after owned kernels/helpers have
settled. Check normal kernel-owned broker launch and ListSlices again, recording
engine/socket identity. A stable inode does not prove socket-regeneration
acceptance. Source tests, DEV replay and restart checks leave signed Path-1,
ordinary-versus-managed comparison and independent exact-head review open.
