#!/bin/sh
set -eu

MARKER_VALUE=managed-remote-kernels-image-builder-v1
MARKER_PATH=/.chariox-managed-image-builder
path1_data_volume_dropins_bypassed=0
claimed_probe_root=
probe_root_identity=

fail() {
  echo "prepare-hetzner-image.sh: $*" >&2
  exit 1
}

assert_unmounted_probe_root() {
  probe_root=$1
  [ -d "$probe_root" ] && [ ! -L "$probe_root" ] || fail "probe data-root is not a real directory"
  probe_mounts=$(findmnt --noheadings --raw --output TARGET) || fail "could not inspect probe mounts"
  probe_mounted=$(printf '%s\n' "$probe_mounts" | awk -v root="$probe_root" '$0 == root || index($0, root "/") == 1 { print "mounted" }') \
    || fail "could not parse probe mounts"
  [ -z "$probe_mounted" ] || fail "probe data-root contains a mount"
}

claim_empty_probe_root() {
  assert_unmounted_probe_root "$1"
  probe_entry=$(find "$1" -mindepth 1 -print -quit) || fail "could not inspect probe data-root"
  [ -z "$probe_entry" ] || fail "probe data-root contains pre-existing data"
  probe_root_identity=$(stat -c '%d:%i' "$1") || fail "could not identify probe data-root"
  claimed_probe_root=$1
}

clear_owned_probe_root() {
  assert_unmounted_probe_root "$1"
  [ -n "${probe_root_identity:-}" ] && [ "$(stat -c '%d:%i' "$1")" = "$probe_root_identity" ] \
    || fail "probe data-root identity changed"
  # Called only after the storage services are verified inactive. The directory
  # was empty before this invocation's probes; never erase an inherited store.
  find "$1" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} + || fail "could not clear probe-owned engine metadata"
  claimed_probe_root=
}

assert_path1_unit_has_no_dropins() {
  drop_in_paths=$(systemctl show --property=DropInPaths --value "$1") \
    || fail "could not inspect effective systemd drop-ins for $1"
  [ -z "$drop_in_paths" ] \
    || fail "Path-1 service $1 has systemd drop-ins: $drop_in_paths"
}

assert_path1_builder_storage_pristine() {
  [ "$managed_provider_topology" = path1 ] || return 0
  for builder_unit in \
    chariox-data-volume-admission.service \
    chariox-slice-disk-quota-allocator.service \
    chariox-rootless-docker.service; do
    builder_state=$(systemctl show --property=ActiveState --value "$builder_unit") \
      || fail "could not inspect image-builder unit state for $builder_unit"
    [ "$builder_state" = inactive ] \
      || fail "image-builder storage unit $builder_unit is not inactive"
  done
  builder_data_root=/var/lib/chariox-docker/data
  if [ -e "$builder_data_root" ] || [ -L "$builder_data_root" ]; then
    [ -d "$builder_data_root" ] && [ ! -L "$builder_data_root" ] \
      || fail "image-builder Docker data-root is not a real directory"
    if mountpoint --quiet "$builder_data_root"; then
      fail "image-builder Docker data-root is already mounted"
    else
      builder_mount_status=$?
      [ "$builder_mount_status" -eq 32 ] \
        || fail "could not inspect the image-builder Docker data-root mount"
    fi
  fi
  builder_binding_state=/var/lib/chariox-data-volume
  if [ -e "$builder_binding_state" ] || [ -L "$builder_binding_state" ]; then
    [ -d "$builder_binding_state" ] && [ ! -L "$builder_binding_state" ] \
      || fail "image-builder data-volume state is not a real directory"
    builder_binding_entry=$(find "$builder_binding_state" -mindepth 1 -print -quit) \
      || fail "could not inspect image-builder data-volume binding state"
    [ -z "$builder_binding_entry" ] \
      || fail "image-builder data-volume state already exists; refusing to reuse its binding"
  fi
  builder_observation_state=/run/chariox-data-volume-observation
  if [ -e "$builder_observation_state" ] || [ -L "$builder_observation_state" ]; then
    [ -d "$builder_observation_state" ] && [ ! -L "$builder_observation_state" ] \
      || fail "image-builder data-volume observation state is not a real directory"
    builder_observation_entry=$(find "$builder_observation_state" -mindepth 1 -print -quit) \
      || fail "could not inspect image-builder data-volume observation state"
    [ -z "$builder_observation_entry" ] \
      || fail "image-builder has an existing data-volume observation; refusing to reuse it"
  fi
}

restore_path1_dropin() {
  builder_link=$1
  builder_target=$2
  builder_label=$3
  builder_parent=${builder_link%/*}
  [ -d "$builder_parent" ] && [ ! -L "$builder_parent" ] \
    || { echo "prepare-hetzner-image.sh: $builder_label drop-in directory is unsafe" >&2; return 1; }
  if [ -L "$builder_link" ]; then
    builder_actual_target=$(readlink "$builder_link") \
      || { echo "prepare-hetzner-image.sh: could not inspect $builder_label drop-in" >&2; return 1; }
    [ "$builder_actual_target" = "$builder_target" ] \
      || { echo "prepare-hetzner-image.sh: $builder_label drop-in changed during image preparation" >&2; return 1; }
  elif [ -e "$builder_link" ]; then
    echo "prepare-hetzner-image.sh: $builder_label drop-in path became obstructed" >&2
    return 1
  else
    ln -s "$builder_target" "$builder_link" \
      || { echo "prepare-hetzner-image.sh: could not restore $builder_label drop-in" >&2; return 1; }
  fi
}

restore_path1_data_volume_dropins() {
  [ "$path1_data_volume_dropins_bypassed" -eq 1 ] || return 0
  builder_restore_status=0
  restore_path1_dropin \
    /etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf \
    ../../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf \
    "rootless Docker" || builder_restore_status=1
  restore_path1_dropin \
    /etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf \
    ../../../../usr/lib/chariox/current/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf \
    "quota allocator" || builder_restore_status=1
  systemctl daemon-reload || builder_restore_status=1
  if [ "$builder_restore_status" -eq 0 ]; then
    path1_data_volume_dropins_bypassed=0
    return 0
  fi
  return 1
}

cleanup_path1_data_volume_bypass() {
  builder_cleanup_status=$?
  builder_cleanup_failed=0
  trap - EXIT HUP INT TERM
  if [ "$path1_data_volume_dropins_bypassed" -eq 1 ] || [ -n "${claimed_probe_root:-}" ]; then
    builder_services_stopped=1
    builder_storage_units='chariox-rootless-docker.service chariox-slice-disk-quota-allocator.service'
    if [ "$managed_provider_topology" = path1 ]; then
      builder_storage_units="$builder_storage_units chariox-data-volume-admission.service"
    fi
    for builder_unit in $builder_storage_units; do
      systemctl stop "$builder_unit" || builder_services_stopped=0
    done
    for builder_unit in $builder_storage_units; do
      builder_active_state=$(systemctl show --property=ActiveState --value "$builder_unit") || builder_services_stopped=0
      if [ "$builder_active_state" != inactive ]; then
        builder_services_stopped=0
      fi
    done
    if [ "$builder_services_stopped" -eq 1 ]; then
      if [ -n "${claimed_probe_root:-}" ]; then
        # Isolation lets a failed ownership/mount check preserve the original
        # failure and still attempt drop-in restoration. Never claim here.
        if (clear_owned_probe_root "$claimed_probe_root"); then
          claimed_probe_root=
        else
          builder_cleanup_failed=1
        fi
      fi
      restore_path1_data_volume_dropins || builder_cleanup_failed=1
    else
      echo "prepare-hetzner-image.sh: probe data and any bypassed drop-ins preserved because image-builder storage services could not be stopped" >&2
      builder_cleanup_failed=1
    fi
  fi
  if [ "$builder_cleanup_status" -eq 0 ] && [ "$builder_cleanup_failed" -ne 0 ]; then
    builder_cleanup_status=1
  fi
  exit "$builder_cleanup_status"
}

bypass_path1_data_volume_dropins() {
  [ "$managed_provider_topology" = path1 ] || return 0
  builder_rootless_dropin=/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf
  builder_allocator_dropin=/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf
  builder_rootless_target=../../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf
  builder_allocator_target=../../../../usr/lib/chariox/current/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf
  [ -L "$builder_rootless_dropin" ] \
    && [ "$(readlink "$builder_rootless_dropin")" = "$builder_rootless_target" ] \
    || fail "signed rootless Docker data-volume drop-in is missing or changed"
  [ -L "$builder_allocator_dropin" ] \
    && [ "$(readlink "$builder_allocator_dropin")" = "$builder_allocator_target" ] \
    || fail "signed quota allocator data-volume drop-in is missing or changed"
  path1_data_volume_dropins_bypassed=1
  rm -- "$builder_rootless_dropin" "$builder_allocator_dropin"
  systemctl daemon-reload
}

if [ "$(id -u)" -ne 0 ]; then
  fail "run as root on the disposable Hetzner image builder"
fi
if [ "$#" -ne 3 ]; then
  fail "usage: prepare-hetzner-image.sh <release-rootfs> <release-digest> <trusted-public-key>"
fi
if [ ! -f "$MARKER_PATH" ] || [ -L "$MARKER_PATH" ] \
  || [ "$(sed -n '1p' "$MARKER_PATH")" != "$MARKER_VALUE" ]; then
  fail "refusing to modify a host that is not marked as the disposable image builder"
fi

release_rootfs=$1
release_digest=$2
trusted_public_key=$3
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
provider_versions=$script_root/provider-versions.env

case "${CHARIOX_MANAGED_PROVIDER_TOPOLOGY-}" in
  path1)
    managed_provider_topology=path1
    managed_bootstrap_service=chariox-path1-managed-bootstrap.service
    other_managed_bootstrap_service=chariox-managed-bootstrap.service
    [ -n "${CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY:-}" ] \
      || fail "Path-1 preparation requires CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY outside the release rootfs"
    [ -f "$CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY" ] \
      && [ ! -L "$CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY" ] \
      || fail "trusted builder public key must be a regular file"
    ;;
  shared_host|legacy_shared_host)
    managed_provider_topology=shared_host
    managed_bootstrap_service=chariox-managed-bootstrap.service
    other_managed_bootstrap_service=chariox-path1-managed-bootstrap.service
    ;;
  '') fail "CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host" ;;
  *) fail "CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be path1 or shared_host" ;;
esac

printf '%s\n' "$release_digest" | grep -Eq '^sha256:[0-9a-f]{64}$' \
  || fail "release digest must be a SHA-256 digest"
if [ ! -f "$provider_versions" ] || [ -L "$provider_versions" ]; then
  fail "provider version file must be a regular file"
fi
# shellcheck disable=SC1090
. "$provider_versions"
codex_version=${CHARIOX_CODEX_VERSION:-}
opencode_version=${CHARIOX_OPENCODE_VERSION:-}
claude_version=${CHARIOX_CLAUDE_VERSION:-}
for version in "$codex_version" "$opencode_version" "$claude_version"; do
  printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
    || fail "provider versions must be exact numeric releases"
done
[ "$(uname -m)" = "x86_64" ] || fail "the managed staging image must use x86_64"

os_release=/etc/os-release
if [ -L "$os_release" ]; then
  [ "$(readlink "$os_release")" = "../usr/lib/os-release" ] \
    || fail "the image builder has an unexpected os-release symlink"
  os_release=/usr/lib/os-release
fi
if [ ! -f "$os_release" ] || [ -L "$os_release" ]; then
  fail "the image builder has no trusted regular os-release file"
fi
# shellcheck disable=SC1091
. "$os_release"
[ "${ID:-}" = "ubuntu" ] || fail "the managed staging image must use Ubuntu"
[ "${VERSION_ID:-}" = "26.04" ] || fail "the managed staging image must use Ubuntu 26.04"

export DEBIAN_FRONTEND=noninteractive
apt-get update
# Bubblewrap remains installed for Docker-slice inner defense and explicit
# shared-host images; Path 1 does not use it as the provider boundary.
apt-get install -y --no-install-recommends \
  dbus-user-session \
  acl \
  e2fsprogs \
  bash \
  bubblewrap \
  busybox-static \
  build-essential \
  ca-certificates \
  cloud-init \
  curl \
  docker-buildx \
  docker.io \
  fuse-overlayfs \
  gh \
  git \
  jq \
  libdbus-1-dev \
  libssl-dev \
  lsof \
  nodejs \
  npm \
  psmisc \
  pkg-config \
  protobuf-compiler \
  python3 \
  ripgrep \
  rsync \
  rootlesskit \
  slirp4netns \
  socat \
  uidmap \
  unzip \
  util-linux \
  xfsprogs \
  zstd

systemctl disable --now docker.service docker.socket >/dev/null 2>&1 || true
systemctl mask docker.service docker.socket >/dev/null
docker_service_state=$(systemctl is-active docker.service || true)
docker_socket_state=$(systemctl is-active docker.socket || true)
"$script_root/remove-stale-rootful-docker-socket.sh" \
  /var/run/docker.sock \
  "$docker_service_state" \
  "$docker_socket_state" \
  /usr/bin/lsof

node_major=$(node -p 'Number(process.versions.node.split(".")[0])')
[ "$node_major" -eq 22 ] || fail "Ubuntu image did not provide the required Node.js 22 runtime"
docker buildx version >/dev/null || fail "Docker Buildx is unavailable"

"$script_root/install-image.sh" \
  "$release_rootfs" "$release_digest" "$trusted_public_key" "$managed_provider_topology"
assert_path1_builder_storage_pristine
if [ "$managed_provider_topology" = path1 ]; then
  assert_path1_unit_has_no_dropins "$managed_bootstrap_service"
  assert_path1_unit_has_no_dropins chariox-disposable-worker-bootstrap.service
  runtime_builder_key=/etc/chariox/trusted-builder-public-key
  [ -f "$runtime_builder_key" ] && [ ! -L "$runtime_builder_key" ] \
    || fail "Path-1 image is missing its independent runtime builder key"
  [ "$(stat -c '%u:%a' "$runtime_builder_key")" = "0:644" ] \
    || fail "Path-1 runtime builder key ownership or mode is unsafe"
  node "$script_root/managed-kernel-upgrade-state.mjs" compare-builder-pins "$CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY" "$runtime_builder_key" \
    || fail "Path-1 runtime builder key differs from the independent input"
fi
# The general installer journals even a no-op home migration. A fresh image
# must not carry that runtime bookkeeping into every future machine.
migration_journal=/var/lib/chariox/home-migration.json
migration_complete=/var/lib/chariox/home-migration-complete
if [ -e "$migration_journal" ] || [ -L "$migration_journal" ] \
  || [ -e "$migration_complete" ] || [ -L "$migration_complete" ]; then
  for migration_file in "$migration_journal" "$migration_complete"; do
    [ -f "$migration_file" ] && [ ! -L "$migration_file" ] \
      && [ "$(stat -c '%u:%a' "$migration_file")" = "0:600" ] \
      || fail "image installer left unsafe migration state"
  done
  printf 'complete\n' | cmp -s - "$migration_complete" \
    || fail "image installer left incomplete migration state"
  jq -e \
    --arg stateRoot /var/lib/chariox \
    --arg legacyHome /var/lib/chariox/home \
    --arg managedHome /home/chariox \
    --arg managedState /home/chariox/.chariox \
    --argjson uid "$(id -u chariox)" \
    --argjson gid "$(id -g chariox)" \
    'keys == ["charioxGid", "charioxUid", "entries", "legacyHome", "managedHome", "managedState", "required", "rootIdentity", "schemaVersion", "stateRoot"]
      and .schemaVersion == 1 and .stateRoot == $stateRoot
      and .legacyHome == $legacyHome and .managedHome == $managedHome
      and .managedState == $managedState and .charioxUid == $uid
      and .charioxGid == $gid and .required == false and .rootIdentity == null and .entries == []' \
    "$migration_journal" >/dev/null \
    || fail "image installer left a nonempty migration journal"
  rm -f -- "$migration_journal" "$migration_complete"
fi
provider_toolchain_source=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/toolchain
provider_toolchain_root=/opt/chariox-provider-toolchain
for toolchain_file in package.json package-lock.json; do
  [ -f "$provider_toolchain_source/$toolchain_file" ] \
    && [ ! -L "$provider_toolchain_source/$toolchain_file" ] \
    || fail "signed provider toolchain is missing $toolchain_file"
done
rm -rf "$provider_toolchain_root"
install -d -o root -g root -m 0755 "$provider_toolchain_root"
install -o root -g root -m 0644 \
  "$provider_toolchain_source/package.json" \
  "$provider_toolchain_source/package-lock.json" \
  "$provider_toolchain_root/"
npm_cache=/tmp/chariox-provider-npm-cache
rm -rf "$npm_cache"
(cd "$provider_toolchain_root" && npm_config_cache="$npm_cache" npm ci --omit=dev)
rm -rf "$npm_cache" /root/.npm
chmod -R u=rwX,go=rX "$provider_toolchain_root"
ln -sfn "$provider_toolchain_root/node_modules/.bin/codex" /usr/local/bin/codex
ln -sfn "$provider_toolchain_root/node_modules/.bin/opencode" /usr/local/bin/opencode
ln -sfn "$provider_toolchain_root/node_modules/.bin/claude" /usr/local/bin/claude
ln -sfn "$provider_toolchain_root/node_modules/.bin/pnpm" /usr/local/bin/pnpm
cleanup_provider_probe_home() {
  [ -z "$provider_probe_home" ] || rm -rf "$provider_probe_home"
}
provider_probe_exit_cleanup() {
  provider_probe_exit_status=$?
  trap - 0 HUP INT TERM
  cleanup_provider_probe_home || :
  exit "$provider_probe_exit_status"
}
provider_probe_signal_cleanup() {
  provider_probe_signal=$1
  trap - 0 HUP INT TERM
  cleanup_provider_probe_home || :
  trap - "$provider_probe_signal"
  kill -s "$provider_probe_signal" "$$"
  exit 1
}
provider_probe_home=$(mktemp -d /tmp/chariox-provider-probe.XXXXXX)
trap provider_probe_exit_cleanup 0
trap 'provider_probe_signal_cleanup HUP' HUP
trap 'provider_probe_signal_cleanup INT' INT
trap 'provider_probe_signal_cleanup TERM' TERM
chown chariox:chariox "$provider_probe_home"
chmod 0700 "$provider_probe_home"
provider_tool_as_chariox() {
  runuser -u chariox -- env -i \
    HOME="$provider_probe_home" \
    PATH=/usr/local/bin:/usr/bin:/bin \
    sh -c 'cd "$HOME" && exec "$@"' sh "$@"
}
[ "$(provider_tool_as_chariox codex --version)" = "codex-cli $codex_version" ] \
  || fail "installed Codex version does not match"
[ "$(provider_tool_as_chariox opencode --version)" = "$opencode_version" ] \
  || fail "installed OpenCode version does not match"
[ "$(provider_tool_as_chariox claude --version)" = "$claude_version (Claude Code)" ] \
  || fail "installed Claude Code version does not match"
[ "$(provider_tool_as_chariox pnpm --version)" = "11.22.0" ] \
  || fail "installed pnpm version does not match"
if [ "$managed_provider_topology" = shared_host ]; then
  sh "$script_root/verify-provider-runtime-bind.sh"
fi
rm -rf "$provider_probe_home"
trap - 0 HUP INT TERM
[ -x /usr/share/docker.io/contrib/dockerd-rootless.sh ] \
  || fail "Docker package has no rootless daemon launcher"
if id -nG chariox | tr ' ' '\n' | grep -qx docker; then
  fail "managed kernel user must not control rootful Docker"
fi
configure_subid_range() {
  subid_file=$1
  usermod_option=$2
  expected=chariox-docker:231072:65536
  if grep -q '^chariox-docker:' "$subid_file"; then
    [ "$(grep '^chariox-docker:' "$subid_file")" = "$expected" ] \
      || fail "chariox-docker has an incompatible subordinate ID range in $subid_file"
    return
  fi
  awk -F: '
    BEGIN { requested_start = 231072; requested_end = 296607; overlap = 0 }
    NF == 3 {
      existing_start = $2 + 0
      existing_end = existing_start + $3 - 1
      if (existing_start <= requested_end && existing_end >= requested_start) overlap = 1
    }
    END { exit overlap }
  ' "$subid_file" || fail "requested subordinate ID range overlaps $subid_file"
  usermod "$usermod_option" 231072-296607 chariox-docker
}
configure_subid_range /etc/subuid --add-subuids
configure_subid_range /etc/subgid --add-subgids
"$script_root/verify-slice-publication-access.sh"
[ "$(stat -c %d /var/lib/chariox-slice-share/.broker-private/output)" = \
  "$(stat -c %d /var/lib/chariox-slice-share)" ] \
  || fail "broker output staging is not on the managed share filesystem"
python3 "$script_root/../local-linux/provision-docker-admission-locks.py"
rootless_docker_config=/var/lib/chariox-docker/home/.config/docker/daemon.json
remove_seeded_rootless_quota_config=0
if [ ! -e "$rootless_docker_config" ] && [ ! -L "$rootless_docker_config" ]; then
  remove_seeded_rootless_quota_config=1
fi
# Both topologies own disposable builder storage; register cleanup before any claim.
trap cleanup_path1_data_volume_bypass EXIT
trap 'exit 1' HUP INT TERM
bypass_path1_data_volume_dropins
assert_path1_builder_storage_pristine
if [ ! -e /var/lib/chariox-docker/data ] && [ ! -L /var/lib/chariox-docker/data ]; then
  install -d -o chariox-docker -g chariox-docker -m 0700 /var/lib/chariox-docker/data
fi
claim_empty_probe_root /var/lib/chariox-docker/data
systemctl start chariox-rootless-docker.service
rootless_docker_ready=0
for _attempt in $(seq 1 30); do
  if runuser -u chariox-docker -- env DOCKER_HOST=unix:///run/chariox-docker/docker.sock docker info >/dev/null 2>&1; then
    rootless_docker_ready=1
    break
  fi
  sleep 1
done
[ "$rootless_docker_ready" -eq 1 ] || fail "rootless Docker daemon is not ready"
"$script_root/verify-rootless-handle-lifecycle.sh"
slice_base_image=node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c
runuser -u chariox-docker -- env \
  DOCKER_HOST=unix:///run/chariox-docker/docker.sock \
  docker pull "$slice_base_image" >/dev/null
# Exercise BuildKit's client-side registry-auth resolution through the real
# broker entrypoint. A Docker pull alone only tests daemon-side resolution.
broker_namespace=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh
{
  printf '%s\n' '# syntax=docker/dockerfile:1@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32'
  printf 'FROM %s\n' "$slice_base_image"
} |
runuser -u chariox-docker -- env \
  DOCKER_BUILDKIT=1 \
  TMPDIR=/tmp \
  DOCKER_HOST=unix:///run/chariox-docker/docker.sock \
  "$broker_namespace" docker build --pull --tag chariox-broker-network-check:local - >/dev/null
runuser -u chariox-docker -- env \
  DOCKER_HOST=unix:///run/chariox-docker/docker.sock \
  docker image inspect "$slice_base_image" >/dev/null
runuser -u chariox-docker -- env \
  DOCKER_HOST=unix:///run/chariox-docker/docker.sock \
  docker image rm chariox-broker-network-check:local >/dev/null
runuser -u chariox-docker -- env \
  DOCKER_HOST=unix:///run/chariox-docker/docker.sock \
  docker image rm "$slice_base_image" >/dev/null
if runuser -u chariox -- env DOCKER_HOST=unix:///run/chariox-docker/docker.sock docker info >/dev/null 2>&1; then
  fail "managed kernel user can access the rootless Docker daemon"
fi
systemctl stop chariox-rootless-docker.service
systemctl stop chariox-slice-disk-quota-allocator.service
if systemctl is-active --quiet chariox-rootless-docker.service; then
  fail "rootless Docker remained active while freezing the image"
fi
if systemctl is-active --quiet chariox-slice-disk-quota-allocator.service; then
  fail "slice disk quota allocator remained active while freezing the image"
fi
assert_path1_builder_storage_pristine
restore_path1_data_volume_dropins
clear_owned_probe_root /var/lib/chariox-docker/data
if [ -e /var/lib/chariox-docker/data ] || [ -L /var/lib/chariox-docker/data ]; then
  [ -d /var/lib/chariox-docker/data ] && [ ! -L /var/lib/chariox-docker/data ] \
    || fail "Docker data-root is not a real directory"
  if find /var/lib/chariox-docker/data -mindepth 1 -print -quit | grep -q .; then
    fail "Docker data-root contains volume data; refusing to erase it while preparing the image"
  fi
fi
if [ "$managed_provider_topology" = path1 ]; then
  # Path 1 admission mounts the data Volume here and then hands it to chariox-docker.
  install -d -o root -g root -m 0700 /var/lib/chariox-docker/data
else
  # The shared host's quota allocator names the data root in ReadWritePaths, so
  # keep it present and owned by rootless Docker, which uses it unmounted.
  install -d -o chariox-docker -g chariox-docker -m 0700 /var/lib/chariox-docker/data
fi
rm -rf /var/lib/chariox-docker/home/.docker
if [ "$remove_seeded_rootless_quota_config" -eq 1 ] \
  && [ -f "$rootless_docker_config" ] \
  && cmp -s "$rootless_docker_config" - <<'EOF'
{"features":{"containerd-snapshotter":false},"storage-driver":"overlay2"}
EOF
then
  rm -f -- "$rootless_docker_config"
  rmdir /var/lib/chariox-docker/home/.config/docker /var/lib/chariox-docker/home/.config 2>/dev/null || true
fi
install -d -o chariox-docker -g chariox-docker -m 0700 /var/lib/chariox-docker/home
systemctl is-enabled --quiet chariox-rootless-docker.service \
  || fail "rootless Docker service was not enabled"
if systemctl is-enabled --quiet chariox-slice-broker.service; then
  fail "slice Docker broker must be published only by managed bootstrap prestart"
fi
systemctl is-enabled --quiet "$managed_bootstrap_service" \
  || fail "managed bootstrap service was not enabled"
if systemctl is-enabled --quiet "$other_managed_bootstrap_service"; then
  fail "a non-selected managed bootstrap service was also enabled"
fi
if systemctl is-enabled --quiet chariox-disposable-worker-bootstrap.service; then
  fail "disposable worker bootstrap service must not be enabled in a managed-home image"
fi
for bootstrap_service in "$managed_bootstrap_service" "$other_managed_bootstrap_service" chariox-disposable-worker-bootstrap.service; do
  if systemctl is-active --quiet "$bootstrap_service"; then
    fail "bootstrap service $bootstrap_service started while the image was being built"
  fi
done

for state_directory in \
  /var/lib/chariox-slice-disk-quota \
  /var/lib/chariox-data-volume \
  /run/chariox-data-volume-observation; do
  if [ -e "$state_directory" ] || [ -L "$state_directory" ]; then
    [ -d "$state_directory" ] && [ ! -L "$state_directory" ] \
      || fail "managed image runtime state path is not a real directory: $state_directory"
    state_entry=$(find "$state_directory" -mindepth 1 -print -quit) \
      || fail "could not inspect managed image runtime state path: $state_directory"
    [ -z "$state_entry" ] \
      || fail "managed image contains persisted quota, data-volume binding, or observation state: $state_directory"
  fi
done
if [ -e /etc/chariox/bootstrap ] || [ -L /etc/chariox/bootstrap ]; then
  [ -d /etc/chariox/bootstrap ] && [ ! -L /etc/chariox/bootstrap ] \
    || fail "protected bootstrap path is not a real directory"
  if find /etc/chariox/bootstrap -mindepth 1 -print -quit | grep -q .; then
    fail "protected bootstrap input must not be captured in the image"
  fi
fi

if find /var/lib/chariox -mindepth 1 -print -quit | grep -q .; then
  fail "managed runtime state entered the image"
fi
if [ -L /home/chariox ] || [ ! -d /home/chariox ]; then
  fail "managed service-account home is missing or linked"
fi
if [ "$(stat -c %a /home/chariox)" != 700 ] || [ "$(stat -c %U /home/chariox)" != chariox ]; then
  fail "managed service-account home permissions are unsafe"
fi
if find /home/chariox -mindepth 1 ! -path /home/chariox/.chariox -print -quit | grep -q .; then
  fail "managed user data entered the image"
fi
if [ -L /home/chariox/.chariox ] || [ ! -d /home/chariox/.chariox ]; then
  fail "managed kernel state directory is missing or linked"
fi
if [ "$(stat -c %a /home/chariox/.chariox)" != 700 ] || [ "$(stat -c %U /home/chariox/.chariox)" != chariox ]; then
  fail "managed kernel state directory permissions are unsafe"
fi
if find /home/chariox/.chariox -mindepth 1 -print -quit | grep -q .; then
  fail "managed kernel state entered the image"
fi
# install-image.sh creates the empty protected slice layout root; anything in it is state.
private_layout_root=/var/lib/chariox-docker/private-layout
if find /var/lib/chariox-docker -mindepth 1 ! -path /var/lib/chariox-docker/data ! -path /var/lib/chariox-docker/home \
    ! -path "$private_layout_root" -print -quit | grep -q . \
  || find /var/lib/chariox-docker/home /var/lib/chariox-docker/data -mindepth 1 -print -quit | grep -q .; then
  fail "rootless Docker state entered the image"
fi
if [ -L "$private_layout_root" ] || [ ! -d "$private_layout_root" ] \
  || [ "$(stat -c '%U:%a' "$private_layout_root")" != chariox-docker:711 ]; then
  fail "protected slice layout root is missing or unsafe"
fi
if find /var/lib/chariox-slice-share -mindepth 1 \
  ! -path /var/lib/chariox-slice-share/.broker-private \
  ! -path /var/lib/chariox-slice-share/.broker-private/output \
  ! -path /var/lib/chariox-slice-share/.broker-private/artifacts \
  ! -path /var/lib/chariox-slice-share/.broker-private/artifacts/states \
  ! -path /var/lib/chariox-slice-share/.broker-private/artifacts/backups \
  ! -path /var/lib/chariox-slice-share/.broker-private/control \
  ! -path /var/lib/chariox-slice-share/slices \
  ! -path /var/lib/chariox-slice-share/slices/development \
  -print -quit | grep -q .; then
  fail "managed slice state entered the image"
fi

apt-get clean
managed_sshd_config=/etc/ssh/sshd_config.d/00-chariox-managed.conf
install -d -o root -g root -m 0755 /etc/ssh/sshd_config.d
managed_sshd_tmp=$(mktemp)
{
  printf '%s\n' 'PasswordAuthentication no'
  printf '%s\n' 'KbdInteractiveAuthentication no'
  printf '%s\n' 'PermitRootLogin prohibit-password'
  # Service accounts may own a login shell (Path 1 chariox) but never inbound SSH.
  printf '%s\n' 'DenyUsers chariox chariox-docker'
} >"$managed_sshd_tmp"
install -o root -g root -m 0644 "$managed_sshd_tmp" "$managed_sshd_config"
rm -f "$managed_sshd_tmp"
passwd --lock root
chage -d "$(date -u +%Y-%m-%d)" -M 99999 -I -1 -E -1 root
sshd_effective=$(sshd -T)
printf '%s\n' "$sshd_effective" | grep -Fxq 'passwordauthentication no' \
  || fail "managed image must disable SSH password authentication"
printf '%s\n' "$sshd_effective" | grep -Fxq 'kbdinteractiveauthentication no' \
  || fail "managed image must disable interactive SSH authentication"
printf '%s\n' "$sshd_effective" | grep -Fxq 'permitrootlogin prohibit-password' \
  || fail "managed image must restrict root SSH to public keys"
# sshd -T prints DenyUsers one per line or space-separated, by OpenSSH version.
printf '%s\n' "$sshd_effective" \
  | awk '$1 == "denyusers" { for (i = 2; i <= NF; i++) denied[$i] = 1 } END { exit !(denied["chariox"] && denied["chariox-docker"]) }' \
  || fail "managed image must deny SSH to its service accounts"
if [ "$managed_provider_topology" = path1 ]; then
  runuser -u chariox -- sudo -n true || fail "Path-1 chariox must have passwordless sudo"
elif runuser -u chariox -- sudo -n true 2>/dev/null; then
  fail "shared-host chariox must not have sudo"
fi
rm -rf /var/lib/apt/lists/* /tmp/chariox-managed-release /root/.cache /root/.npm /root/.ssh
find /var/log -type f -exec sh -c ': > "$1"' _ {} \;
cloud-init clean --logs --machine-id --seed
rm -f /etc/ssh/ssh_host_* "$MARKER_PATH"
sync
echo "managed Hetzner image preparation passed"
