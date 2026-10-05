#!/usr/bin/env bash
# MP-02/MP-03/MP-08/MP-10/MP-11: provider-free save/backup replay on an enrolled isolated host.
set -euo pipefail
[[ ${CHARIOX_STORAGE_QUALIFICATION_ISOLATED_HOST:-} == 1 ]] || { echo 'MP-10: select a disposable isolated DEV host explicitly' >&2; exit 2; }
[[ $(id -u) != 0 ]] || { echo 'MP-02/MP-10: run as the enrolled ordinary user' >&2; exit 2; }
: "${M20_KERNEL_BINARY:?absolute, verified public kernel required}"
: "${M20_SLICE_IMAGE:?immutable enrolled worker image required}"
: "${M20_RUNTIME_ROOT:?absolute disposable runtime root required}"
: "${M20_ARTIFACT_DIR:?absolute evidence directory outside the repository required}"
source_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
node_binary=$(command -v node)
python3 - "$source_root" "$M20_RUNTIME_ROOT" "$M20_ARTIFACT_DIR" "$M20_KERNEL_BINARY" "$node_binary" <<'PY'
from pathlib import Path
import sys
source = Path(sys.argv[1]).resolve()
for raw in sys.argv[2:]:
    p = Path(raw)
    if not p.is_absolute(): raise SystemExit('MP-10: absolute paths required')
for raw in sys.argv[2:4]:
    p = Path(raw).resolve()
    if p == source or source in p.parents or p in source.parents:
        raise SystemExit('MP-10: state/evidence must not overlap source')
if Path(sys.argv[2]).exists(): raise SystemExit('MP-10: use a fresh runtime root per replay')
runtime = Path(sys.argv[2]).resolve()
evidence = Path(sys.argv[3]).resolve()
if evidence == runtime or runtime in evidence.parents:
    raise SystemExit('MP-10: evidence must be outside the disposable runtime root')
PY
export M20_USE_PREBUILT=1 M20_LOCAL_DEV_ENROLLMENT=1
fixture_environment=()
if [[ -n ${M20_STORAGE_FIXTURE_HELPER:-} ]]; then
  fixture_environment+=("M20_STORAGE_FIXTURE_HELPER=$M20_STORAGE_FIXTURE_HELPER")
fi
# A clean environment prevents inherited provider homes or credentials from
# entering this provider-free fixture. The kernel starts its enrolled broker.
exec env -i \
  PATH="$(dirname -- "$node_binary"):/usr/bin:/bin" HOME="$HOME" USER="$(id -un)" LOGNAME="$(id -un)" \
  CODEX_HOME="$M20_RUNTIME_ROOT/provider/codex" \
  CLAUDE_CONFIG_DIR="$M20_RUNTIME_ROOT/provider/claude" \
  OPENCODE_CONFIG_DIR="$M20_RUNTIME_ROOT/provider/opencode" \
  XDG_DATA_HOME="$M20_RUNTIME_ROOT/xdg-data" \
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0="$source_root" \
  M20_USE_PREBUILT=1 M20_LOCAL_DEV_ENROLLMENT=1 \
  M20_KERNEL_BINARY="$M20_KERNEL_BINARY" M20_SLICE_IMAGE="$M20_SLICE_IMAGE" \
  M20_RUNTIME_ROOT="$M20_RUNTIME_ROOT" M20_ARTIFACT_DIR="$M20_ARTIFACT_DIR" \
  "${fixture_environment[@]}" \
  CHARIOX_SLICE_APPARMOR_PROFILE="${CHARIOX_SLICE_APPARMOR_PROFILE:-unconfined}" \
  CHARIOX_ROOM_DRILL_MEMORY_MB="${CHARIOX_ROOM_DRILL_MEMORY_MB:-2048}" \
  CHARIOX_SLICE_DOCKER_PROVISIONER="$source_root/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh" \
  "$node_binary" "$source_root/apps/cli/scripts/live-docker-slice-browser-state-drill.mjs" "$@"
