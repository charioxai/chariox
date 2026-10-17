# MP-07/MP-11: only fixed public phases, never command output or arguments.
# Ordinary progress requires shell builtins only, before Node/Python provisioning.
record_image_preparation_phase() {
  case "$1" in
    image_prepare_start|image_prepare_packages|image_prepare_pin|image_prepare_providers|image_prepare_provider_probe|image_prepare_rootless|image_prepare_pull|image_prepare_build|image_prepare_freeze|image_prepare_complete|image_prepare_failed|image_install_start|image_install_verify|image_install_pin|image_install_publish|image_install_activate|image_install_complete) ;;
    *) echo 'chariox-image-preparation: invalid phase' >&2; return 1 ;;
  esac
  printf 'chariox-image-preparation: %s\n' "$1"
  if [ -n "${CHARIOX_IMAGE_PREPARATION_DIAGNOSTICS_DIR:-}" ]; then
    if ! python3 "${script_root:-$(dirname -- "$0")}/runtime-diagnostics.py" event \
      --directory "$CHARIOX_IMAGE_PREPARATION_DIAGNOSTICS_DIR" --event "$1" >/dev/null 2>&1; then
      echo 'chariox-image-preparation: diagnostic publication failed' >&2
    fi
  fi
}
