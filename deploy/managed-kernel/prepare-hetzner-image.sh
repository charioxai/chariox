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

if [ ! -f /etc/os-release ] || [ -L /etc/os-release ]; then
  fail "the image builder has no regular os-release file"
fi
# shellcheck disable=SC1091
. /etc/os-release
[ "${ID:-}" = "ubuntu" ] || fail "the managed staging image must use Ubuntu"
[ "${VERSION_ID:-}" = "26.04" ] || fail "the managed staging image must use Ubuntu 26.04"

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  bash \
  bubblewrap \
  build-essential \
  ca-certificates \
  cloud-init \
  curl \
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
  socat \
  unzip \
  zstd

node_major=$(node -p 'Number(process.versions.node.split(".")[0])')
[ "$node_major" -eq 22 ] || fail "Ubuntu image did not provide the required Node.js 22 runtime"

npm install -g \
  "@openai/codex@$codex_version" \
  "opencode-ai@$opencode_version" \
  "@anthropic-ai/claude-code@$claude_version"
[ "$(codex --version)" = "codex-cli $codex_version" ] \
  || fail "installed Codex version does not match"
[ "$(opencode --version)" = "$opencode_version" ] \
  || fail "installed OpenCode version does not match"
[ "$(claude --version)" = "$claude_version (Claude Code)" ] \
  || fail "installed Claude Code version does not match"

"$script_root/install-image.sh" "$release_rootfs" "$release_digest" "$trusted_public_key"
systemctl is-enabled --quiet chariox-managed-bootstrap.service \
  || fail "managed bootstrap service was not enabled"
if systemctl is-active --quiet chariox-managed-bootstrap.service; then
  fail "managed bootstrap service started while the image was being built"
fi

if find /var/lib/chariox -mindepth 1 ! -path /var/lib/chariox/home -print -quit | grep -q .; then
  fail "managed runtime state entered the image"
fi

apt-get clean
rm -rf /var/lib/apt/lists/* /tmp/chariox-managed-release /root/.ssh
find /var/log -type f -exec sh -c ': > "$1"' _ {} \;
cloud-init clean --logs --machine-id --seed
rm -f /etc/ssh/ssh_host_*
sync
echo "managed Hetzner image preparation passed"
