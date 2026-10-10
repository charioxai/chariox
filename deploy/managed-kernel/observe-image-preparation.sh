#!/bin/sh
# MP-07/MP-10/MP-11: start public observation before the installer, not after it.
# This disposable-builder wrapper never enrolls an account or reads general logs.
set -eu
[ "$(id -u)" -eq 0 ] && [ "$#" -eq 3 ] || {
  echo 'observe-image-preparation.sh: root and three preparation arguments required' >&2
  exit 1
}
[ -f /.chariox-managed-image-builder ] && [ ! -L /.chariox-managed-image-builder ] \
  && [ "$(sed -n '1p' /.chariox-managed-image-builder)" = managed-remote-kernels-image-builder-v1 ] || {
  echo 'observe-image-preparation.sh: disposable image-builder marker required' >&2
  exit 1
}
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$script_root/image-preparation-progress.sh"
observer_unit=
diagnostic_root=
finish_preparation_observation() {
  preparation_status=$?
  trap - EXIT HUP INT TERM
  if [ "$preparation_status" -eq 0 ]; then
    record_image_preparation_phase image_prepare_complete
  else
    record_image_preparation_phase image_prepare_failed
  fi
  if [ -n "$observer_unit" ]; then
    if ! timeout --foreground --kill-after=5s 20s python3 "$script_root/runtime-diagnostics.py" ship \
      --once --directory "$diagnostic_root" --observer-url "$CHARIOX_IMAGE_PREPARATION_OBSERVER_URL"; then
      echo 'chariox-image-preparation: final observer acknowledgement failed' >&2
      [ "$preparation_status" -ne 0 ] || preparation_status=1
    fi
    if ! systemctl stop "$observer_unit"; then
      echo 'chariox-image-preparation: observer stop failed; public records retained' >&2
      exit 1
    fi
  fi
  # Public records only. Remove this invocation's exact mktemp directory, even
  # after prepare deletes the builder marker; never an inherited runtime store.
  [ -z "$diagnostic_root" ] || rm -rf -- "$diagnostic_root"
  exit "$preparation_status"
}
trap finish_preparation_observation EXIT
trap 'exit 1' HUP INT TERM
unset CHARIOX_IMAGE_PREPARATION_DIAGNOSTICS_DIR
if [ -n "${CHARIOX_IMAGE_PREPARATION_OBSERVER_URL:-}" ]; then
  # The campaign launcher explicitly provisions Python before downloading tools.
  # No Node runtime is needed to install observation or record public progress.
  python3 - "$CHARIOX_IMAGE_PREPARATION_OBSERVER_URL" <<'PY'
import re,sys
if not re.fullmatch(r'https://[a-zA-Z0-9.:-]+/path1-diagnostics/[a-z0-9-]{8,80}',sys.argv[1]):
    raise SystemExit('chariox-image-preparation: credential-free HTTPS observer required')
PY
  diagnostic_root=$(mktemp -d "${TMPDIR:-/tmp}/chariox-image-observation.XXXXXX")
  chmod 0700 "$diagnostic_root"
  export CHARIOX_IMAGE_PREPARATION_DIAGNOSTICS_DIR=$diagnostic_root
  candidate_observer_unit=chariox-image-observation-$$.service
  systemd-run --quiet --collect --unit="$candidate_observer_unit" \
    --property=MemoryMax=64M --property=CPUQuota=10% --property=TasksMax=16 \
    /usr/bin/python3 "$script_root/runtime-diagnostics.py" ship \
    --directory "$diagnostic_root" --observer-url "$CHARIOX_IMAGE_PREPARATION_OBSERVER_URL"
  observer_unit=$candidate_observer_unit
fi
record_image_preparation_phase image_prepare_start
sh "$script_root/prepare-hetzner-image.sh" "$@"
