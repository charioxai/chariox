#!/usr/bin/env bash
# Hosted only: build the actual helper/tests once, then execute fixed root setup.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted && "${GITHUB_REPOSITORY:-}" == charioxai/chariox ]]
[[ "$(uname -sm)" == 'Linux x86_64' && "$GITHUB_RUN_ID" =~ ^[0-9]+$ && "$GITHUB_RUN_ATTEMPT" =~ ^[0-9]+$ ]]
storage_repo="$(realpath -e "$GITHUB_WORKSPACE")"
storage_temp="$(realpath -e "$RUNNER_TEMP")"
storage_scratch="$(mktemp -d "$storage_temp/chariox-storage.XXXXXXXX")"
chmod 700 "$storage_scratch"
mkdir -m 700 "$storage_scratch/evidence" "$storage_scratch/build"
printf 'scratch=%s\nevidence=%s\n' "$storage_scratch" "$storage_scratch/evidence" >> "$GITHUB_OUTPUT"
storage_build="chariox-storage-build-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
cleanup() { sudo systemctl stop "$storage_build.service" >/dev/null 2>&1 || true; }
trap cleanup EXIT
sudo systemd-run --unit="$storage_build" --wait --collect --pipe \
  --uid="$(id -u)" --gid="$(id -g)" --working-directory="$storage_repo" \
  --property=MemoryMax=3G --property=MemorySwapMax=0 --property=CPUQuota=200% \
  --property=TasksMax=256 --property=RuntimeMaxSec=600 --property=KillMode=control-group \
  --setenv="PATH=$PATH" --setenv="CARGO_HOME=$HOME/.cargo" --setenv="RUSTUP_HOME=$HOME/.rustup" \
  --setenv="CARGO_TARGET_DIR=$storage_scratch/build" --setenv=CARGO_BUILD_JOBS=1 \
  --setenv=CARGO_INCREMENTAL=0 --setenv=CARGO_PROFILE_TEST_DEBUG=0 \
  --setenv=CARGO_PROFILE_DEV_DEBUG=0 /bin/bash -c \
  'cargo test --locked -p chariox-app-runtime --lib --no-run --message-format=json && cargo build --locked -p chariox-app-runtime --bin chariox-app-storage --message-format=json' \
  > "$storage_scratch/evidence/build.jsonl" 2> "$storage_scratch/evidence/build.log"
python3 - "$storage_scratch" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]); log=root/'evidence/build.jsonl'; assert log.stat().st_size<8*1024*1024
found={}
for line in log.read_text().splitlines():
 try: item=json.loads(line)
 except ValueError: continue
 if item.get('reason')!='compiler-artifact' or not item.get('executable'): continue
 name=item.get('target',{}).get('name'); path=pathlib.Path(item['executable'])
 assert path.resolve()==path and path.is_relative_to(root/'build')
 if name=='chariox_app_runtime' and item.get('profile',{}).get('test'): found['tests']=path
 if name=='chariox-app-storage' and not item.get('profile',{}).get('test'): found['helper']=path
assert set(found)=={'tests','helper'}
(root/'binaries').write_text(str(found['tests'])+'\n'+str(found['helper'])+'\n')
PY
mapfile -t storage_binaries < "$storage_scratch/binaries"
"${storage_binaries[0]}" worker_process::storage_linux:: --skip hosted_ --test-threads=1 \
  2>&1 | tee "$storage_scratch/evidence/offline.log"
sudo /bin/bash "$storage_repo/scripts/app-storage-linux-fixture.sh" \
  "$storage_repo" "$storage_scratch" "${storage_binaries[0]}" "${storage_binaries[1]}" \
  2>&1 | tee "$storage_scratch/evidence/hosted.log"
