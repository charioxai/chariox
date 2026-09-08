#!/usr/bin/env bash
# Disposable hosted runtime execution only; never compiles or downloads Node.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted ]]
[[ "$(uname -sm)" == 'Linux x86_64' && "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ && "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]]
fixture_repo="$(realpath -e -- "$GITHUB_WORKSPACE")"
fixture_scratch="$(realpath -e -- "$EMBEDDED_SCRATCH")"
fixture_temp="$(realpath -e -- "$RUNNER_TEMP")"
[[ "$(dirname "$fixture_scratch")" == "$fixture_temp" && "$(basename "$fixture_scratch")" =~ ^chariox-embedded\.[A-Za-z0-9]+$ ]]
fixture_unit="chariox-embedded-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
fixture_profile="$fixture_scratch/chariox-embedded.apparmor"
fixture_node="$(command -v node)"
fixture_uid="$(id -u)"
fixture_gid="$(id -g)"
mkdir -m 700 "$fixture_scratch/bin"
cleanup_host() {
  sudo systemctl stop "$fixture_unit.service" >/dev/null 2>&1 || true
  if [[ -e "$fixture_profile" ]]; then sudo apparmor_parser -R "$fixture_profile" >/dev/null 2>&1 || true; fi
}
trap cleanup_host EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
cd "$fixture_repo"
# Same installer-style bundled bubblewrap pin as the independent libc fixture.
mapfile -t fixture_pin < <(node -e 'const p=require("./apps/app-worker/sandbox.lock.json").bubblewrap; for(const v of [p.url,p.sha256,p.size,p.version]) console.log(v)')
[[ "${fixture_pin[0]}" == https://github.com/containers/bubblewrap/releases/download/v0.12.0/bubblewrap-0.12.0.tar.xz ]]
[[ "${fixture_pin[1]}" =~ ^[a-f0-9]{64}$ && "${fixture_pin[2]}" == 126452 && "${fixture_pin[3]}" == 0.12.0 ]]
curl --fail --location --proto '=https' --tlsv1.2 --max-time 30 --max-filesize 131072 \
  --output "$fixture_scratch/bubblewrap.tar.xz" "${fixture_pin[0]}"
[[ "$(stat -c %s "$fixture_scratch/bubblewrap.tar.xz")" == "${fixture_pin[2]}" ]]
printf '%s  %s\n' "${fixture_pin[1]}" "$fixture_scratch/bubblewrap.tar.xz" | sha256sum --check --status
mkdir "$fixture_scratch/bubblewrap-source"
tar -xJf "$fixture_scratch/bubblewrap.tar.xz" --strip-components=1 -C "$fixture_scratch/bubblewrap-source"
meson setup "$fixture_scratch/bubblewrap-build" "$fixture_scratch/bubblewrap-source" \
  --buildtype=release --wrap-mode=nodownload -Dselinux=disabled -Dman=disabled -Dtests=false \
  -Dbash_completion=disabled -Dzsh_completion=disabled -Dassume_kernel=5.9.0
meson compile -C "$fixture_scratch/bubblewrap-build" -j 1
cp "$fixture_scratch/bubblewrap-build/bwrap" "$fixture_scratch/bin/chariox-bwrap"
chmod 755 "$fixture_scratch/bin/chariox-bwrap"
[[ "$("$fixture_scratch/bin/chariox-bwrap" --version)" == 'bubblewrap 0.12.0' ]]
fixture_source="$fixture_repo/apps/app-worker/src"
/usr/bin/gcc -std=c11 -Wall -Wextra -Werror -O1 -I "$fixture_source" \
  "$fixture_source/launcher.c" "$fixture_source/launch_record.c" "$fixture_source/launch_process.c" \
  "$fixture_source/sandbox_linux.c" -ldl -o "$fixture_scratch/bin/chariox-app-worker"
# No artifact library has executed. The production loader will dlopen only
# after its namespace/cgroup/seccomp readiness is independently inspected.
node scripts/package-app-runtime.mjs verify --directory "$fixture_scratch/bundle"
if [[ -r /sys/module/apparmor/parameters/enabled && "$(cat /sys/module/apparmor/parameters/enabled)" == Y ]]; then
  [[ "$fixture_scratch" =~ ^[a-zA-Z0-9_./-]+$ ]]
  printf 'abi <abi/4.0>,\ninclude <tunables/global>\nprofile %s "%s/bin/chariox-bwrap" flags=(unconfined) {\n  userns,\n}\n' \
    "$fixture_unit" "$fixture_scratch" > "$fixture_profile"
  sudo apparmor_parser -r "$fixture_profile"
fi
sudo systemd-run --unit="$fixture_unit" --wait --collect --pipe \
  --property=MemoryMax=2G --property=MemorySwapMax=0 --property=CPUQuota=100% \
  --property=TasksMax=128 --property=RuntimeMaxSec=180 --property=KillMode=control-group \
  /usr/bin/unshare --mount --propagation private /bin/bash \
  "$fixture_repo/scripts/app-worker-embedded-fixture.sh" "$fixture_repo" "$fixture_scratch" \
  "$fixture_uid" "$fixture_gid" "$fixture_node" "$fixture_unit"
