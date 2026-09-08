#!/usr/bin/env bash
# Reuses the pinned native-fixture build; no Node source/library compilation.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted && "${GITHUB_REPOSITORY:-}" == charioxai/chariox ]]
[[ "$(uname -sm)" == 'Linux x86_64' && "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ && "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]]
domain_repo="$(realpath -e -- "$GITHUB_WORKSPACE")"
domain_temp="$(realpath -e -- "$RUNNER_TEMP")"
domain_scratch="$(realpath -e -- "$CHARIOX_DOMAIN_FIXTURE_SCRATCH")"
[[ "$(dirname -- "$domain_scratch")" == "$domain_temp" && "$(basename -- "$domain_scratch")" == chariox-sandbox.* ]]
[[ "$(stat -c %u "$domain_scratch")" == "$(id -u)" && "$(stat -c %a "$domain_scratch")" == 700 ]]
domain_unit="chariox-domain-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
domain_profile="$domain_scratch/domain.apparmor"
cleanup() {
  sudo systemctl stop "$domain_unit.service" "$domain_unit-build.service" >/dev/null 2>&1 || true
  if [[ -f "$domain_profile" ]]; then sudo apparmor_parser -R "$domain_profile" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
cd "$domain_repo"
for domain_source in linux_domain_entry.c ../tests/domain_immediate_fork.c; do
  domain_output=chariox-app-domain-entry
  [[ "$domain_source" != ../tests/domain_immediate_fork.c ]] || domain_output=domain-immediate-fork
  /usr/bin/gcc -std=c11 -Wall -Wextra -Werror -O1 "apps/app-worker/src/$domain_source" -o "$domain_scratch/bin/$domain_output"
done
mkdir -m 700 "$domain_scratch/rust-build"
# Compilation has its own finite cgroup; the execution below uses a smaller one.
sudo systemd-run --unit="$domain_unit-build" --wait --collect --pipe \
  --uid="$(id -u)" --gid="$(id -g)" --working-directory="$domain_repo" \
  --property=MemoryMax=3G --property=MemorySwapMax=0 --property=CPUQuota=200% \
  --property=TasksMax=256 --property=RuntimeMaxSec=600 --property=KillMode=control-group \
  --setenv="PATH=$PATH" --setenv="CARGO_HOME=$HOME/.cargo" --setenv="RUSTUP_HOME=$HOME/.rustup" \
  --setenv="CARGO_TARGET_DIR=$domain_scratch/rust-build" --setenv=CARGO_BUILD_JOBS=1 \
  --setenv=CARGO_INCREMENTAL=0 --setenv=CARGO_PROFILE_TEST_DEBUG=0 \
  "$(command -v cargo)" test --locked -p chariox-app-runtime --lib --no-run --message-format=json \
  > "$domain_scratch/evidence/domain-build.jsonl" 2> "$domain_scratch/evidence/domain-build.log"
domain_binary="$(python3 - "$domain_scratch/evidence/domain-build.jsonl" "$domain_scratch/rust-build" <<'PY'
import json,pathlib,sys
records=pathlib.Path(sys.argv[1]); assert records.stat().st_size <= 4*1024*1024
root=pathlib.Path(sys.argv[2]).resolve(); found=[]
for line in records.read_text().splitlines():
 try: value=json.loads(line)
 except ValueError: continue
 if value.get('reason')=='compiler-artifact' and value.get('target',{}).get('name')=='chariox_app_runtime' and value.get('profile',{}).get('test') and value.get('executable'):
  path=pathlib.Path(value['executable']); assert path.resolve()==path and path.is_relative_to(root)
  found.append(str(path))
assert len(found)==1
print(found[0])
PY
)"
if [[ -r /sys/module/apparmor/parameters/enabled && "$(cat /sys/module/apparmor/parameters/enabled)" == Y ]]; then
  [[ "$domain_scratch" =~ ^[a-zA-Z0-9_./-]+$ ]]
  printf 'abi <abi/4.0>,\ninclude <tunables/global>\nprofile %s "%s/bin/chariox-bwrap" flags=(unconfined) {\n  userns,\n}\n' "$domain_unit" "$domain_scratch" > "$domain_profile"
  sudo apparmor_parser -r "$domain_profile"
fi
sudo systemd-run --unit="$domain_unit" --wait --collect --pipe \
  --property=MemoryMax=1G --property=MemorySwapMax=0 --property=CPUQuota=100% \
  --property=TasksMax=128 --property=RuntimeMaxSec=120 --property=KillMode=control-group --property=Delegate=yes \
  /usr/bin/unshare --mount --propagation private /bin/bash \
  "$domain_repo/scripts/app-domain-linux-fixture.sh" "$domain_scratch" "$(id -u)" "$(id -g)" \
  "$domain_binary" "$domain_unit" "$domain_temp" \
  2>&1 | tee "$domain_scratch/evidence/domain-execution.log"
