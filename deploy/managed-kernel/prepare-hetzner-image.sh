#!/bin/sh
set -eu

MARKER_VALUE=managed-remote-kernels-image-builder-v1
MARKER_PATH=/.chariox-managed-image-builder

fail() {
  echo "prepare-hetzner-image.sh: $*" >&2
  exit 1
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
apt-get install -y --no-install-recommends \
  acl \
  bash \
  bubblewrap \
  busybox-static \
  build-essential \
  ca-certificates \
  cloud-init \
  curl \
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
  pkg-config \
  protobuf-compiler \
  ripgrep \
  rsync \
  rootlesskit \
  slirp4netns \
  socat \
  uidmap \
  unzip \
  util-linux \
  zstd

systemctl disable --now docker.service docker.socket >/dev/null 2>&1 || true
systemctl mask docker.service docker.socket >/dev/null
if [ -S /var/run/docker.sock ]; then
  fail "rootful Docker socket remained active"
fi

node_major=$(node -p 'Number(process.versions.node.split(".")[0])')
[ "$node_major" -eq 22 ] || fail "Ubuntu image did not provide the required Node.js 22 runtime"

"$script_root/install-image.sh" "$release_rootfs" "$release_digest" "$trusted_public_key"
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
ln -sfn "$provider_toolchain_root/node_modules/.bin/codex" /usr/local/bin/codex
ln -sfn "$provider_toolchain_root/node_modules/.bin/opencode" /usr/local/bin/opencode
ln -sfn "$provider_toolchain_root/node_modules/.bin/claude" /usr/local/bin/claude
ln -sfn "$provider_toolchain_root/node_modules/.bin/pnpm" /usr/local/bin/pnpm
[ "$(codex --version)" = "codex-cli $codex_version" ] \
  || fail "installed Codex version does not match"
[ "$(opencode --version)" = "$opencode_version" ] \
  || fail "installed OpenCode version does not match"
[ "$(claude --version)" = "$claude_version (Claude Code)" ] \
  || fail "installed Claude Code version does not match"
[ "$(pnpm --version)" = "11.22.0" ] \
  || fail "installed pnpm version does not match"
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
if runuser -u chariox -- env DOCKER_HOST=unix:///run/chariox-docker/docker.sock docker info >/dev/null 2>&1; then
  fail "managed kernel user can access the rootless Docker daemon"
fi
systemctl stop chariox-rootless-docker.service
rm -rf /var/lib/chariox-docker/data /var/lib/chariox-docker/home/.docker
install -d -o chariox-docker -g chariox-docker -m 0700 /var/lib/chariox-docker/home
systemctl is-enabled --quiet chariox-rootless-docker.service \
  || fail "rootless Docker service was not enabled"
if systemctl is-enabled --quiet chariox-slice-broker.service; then
  fail "slice Docker broker must be published only by managed bootstrap prestart"
fi
systemctl is-enabled --quiet chariox-managed-bootstrap.service \
  || fail "managed bootstrap service was not enabled"
if systemctl is-active --quiet chariox-managed-bootstrap.service; then
  fail "managed bootstrap service started while the image was being built"
fi

if find /var/lib/chariox -mindepth 1 ! -path /var/lib/chariox/home -print -quit | grep -q .; then
  fail "managed runtime state entered the image"
fi
if find /var/lib/chariox-docker -mindepth 1 ! -path /var/lib/chariox-docker/home -print -quit | grep -q . \
  || find /var/lib/chariox-docker/home -mindepth 1 -print -quit | grep -q .; then
  fail "rootless Docker state entered the image"
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
rm -rf /var/lib/apt/lists/* /tmp/chariox-managed-release /root/.npm /root/.ssh
find /var/log -type f -exec sh -c ': > "$1"' _ {} \;
cloud-init clean --logs --machine-id --seed
rm -f /etc/ssh/ssh_host_* "$MARKER_PATH"
sync
echo "managed Hetzner image preparation passed"
