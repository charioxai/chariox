#!/usr/bin/env bash
# Disposable hosted native OS fixture only. No Docker or Node/libnode build.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != true || "${RUNNER_ENVIRONMENT:-}" != github-hosted || "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo 'This fixture requires a disposable GitHub-hosted Linux x64 runner.' >&2
  exit 1
fi
[[ "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ && "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]] || exit 1
fixture_repo="$(realpath -e -- "$GITHUB_WORKSPACE")"
fixture_temp="$(realpath -e -- "$RUNNER_TEMP")"
fixture_scratch="$(mktemp -d "$fixture_temp/chariox-sandbox.XXXXXXXX")"
fixture_unit="chariox-native-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
fixture_profile="$fixture_scratch/chariox-native.apparmor"
fixture_node="$(command -v node)"
fixture_uid="$(id -u)"
fixture_gid="$(id -g)"
printf 'scratch=%s\nevidence=%s/evidence\n' "$fixture_scratch" "$fixture_scratch" >> "$GITHUB_OUTPUT"
mkdir -m 700 "$fixture_scratch/evidence" "$fixture_scratch/bin"
cleanup_host() {
  sudo systemctl stop "$fixture_unit.service" >/dev/null 2>&1 || true
  if [[ -e "$fixture_profile" ]]; then
    sudo apparmor_parser -R "$fixture_profile" >/dev/null 2>&1 || true
  fi
}
trap cleanup_host EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
cd "$fixture_repo"
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
fixture_tests="$fixture_repo/apps/app-worker/tests"
fixture_common=( -std=c11 -Wall -Wextra -Werror -O1 -I "$fixture_source" )
/usr/bin/gcc "${fixture_common[@]}" "$fixture_source/launcher.c" "$fixture_source/launch_record.c" \
  "$fixture_source/launch_process.c" "$fixture_source/sandbox_linux.c" -ldl -o "$fixture_scratch/bin/chariox-app-worker"
/usr/bin/gcc "${fixture_common[@]}" "$fixture_source/launcher.c" "$fixture_source/launch_record.c" \
  "$fixture_source/launch_process.c" "$fixture_tests/weakened_policy.c" -ldl -o "$fixture_scratch/bin/weakened-test-worker"
/usr/bin/gcc "${fixture_common[@]}" -shared -fPIC -pthread "$fixture_tests/native_probe.c" -o "$fixture_scratch/bin/libchariox-app-runtime.so"
# The probe needs libc only. Inspect ELF metadata; do not execute the library.
for fixture_binary in chariox-app-worker weakened-test-worker libchariox-app-runtime.so; do
  readelf -d "$fixture_scratch/bin/$fixture_binary" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p' > "$fixture_scratch/$fixture_binary.needed"
  [[ "$(cat "$fixture_scratch/$fixture_binary.needed")" == libc.so.6 ]]
done
# Ubuntu's userns restriction is handled by one owned AppArmor profile for our
# exact bundled executable, never by disabling AppArmor or a global sysctl.
if [[ -r /sys/module/apparmor/parameters/enabled && "$(cat /sys/module/apparmor/parameters/enabled)" == Y ]]; then
  [[ "$fixture_scratch" =~ ^[a-zA-Z0-9_./-]+$ ]]
  printf 'abi <abi/4.0>,\ninclude <tunables/global>\nprofile %s "%s/bin/chariox-bwrap" flags=(unconfined) {\n  userns,\n}\n' \
    "$fixture_unit" "$fixture_scratch" > "$fixture_profile"
  sudo apparmor_parser -r "$fixture_profile"
fi
# Root provisioning and every probe descendant enter this cgroup before any
# native fixture runs. All mounts live in a private mount namespace.
sudo systemd-run --unit="$fixture_unit" --wait --collect --pipe \
  --property=MemoryMax=1G --property=MemorySwapMax=0 --property=CPUQuota=100% \
  --property=TasksMax=64 --property=RuntimeMaxSec=120 --property=KillMode=control-group \
  /usr/bin/unshare --mount --propagation private /bin/bash \
  "$fixture_repo/scripts/app-worker-linux-fixture.sh" "$fixture_repo" "$fixture_scratch" \
  "$fixture_uid" "$fixture_gid" "$fixture_node" "$fixture_unit"
