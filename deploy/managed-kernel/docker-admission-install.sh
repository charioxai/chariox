# Called only after the complete release, including its source tree, is verified.
# Old releases without the provisioner remain valid for rollback.
install_docker_admission_artifacts() {
  admission_context=$1/usr/lib/chariox/slice-build-context
  admission_prefix=$2
  admission_script=$admission_context/deploy/local-linux/provision-docker-admission-locks.py
  admission_unit=$admission_context/deploy/managed-kernel/chariox-docker-admission-locks.service
  if [ ! -e "$admission_script" ] && [ ! -e "$admission_unit" ]; then
    return 0
  fi
  [ -f "$admission_script" ] && [ ! -L "$admission_script" ] \
    && [ -f "$admission_unit" ] && [ ! -L "$admission_unit" ] || return 1
  install -d -o root -g root -m 0755 "$admission_prefix/usr/libexec" "$admission_prefix/etc/systemd/system" || return 1
  install -o root -g root -m 0755 "$admission_script" "$admission_prefix/usr/libexec/chariox-docker-admission-locks" || return 1
  install -o root -g root -m 0644 "$admission_unit" "$admission_prefix/etc/systemd/system/chariox-docker-admission-locks.service" || return 1
  python3 "$admission_prefix/usr/libexec/chariox-docker-admission-locks" --root "${admission_prefix:-/}" || return 1
  if [ -z "$admission_prefix" ]; then
    systemctl daemon-reload || return 1
    systemctl enable chariox-docker-admission-locks.service
  else
    echo "staged Docker admission boot unit; host systemd unchanged"
  fi
}
