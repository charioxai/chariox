# Release activation owns these two access settings until the final commit.
begin_access_policy_transaction() {
  previous_login_shell=$(getent passwd chariox | cut -d: -f7)
  [ -n "$previous_login_shell" ] || return 1
  sudoers_was_present=0
  if [ -e "$chariox_sudoers" ] || [ -L "$chariox_sudoers" ]; then
    if [ -L "$chariox_sudoers" ] || [ ! -f "$chariox_sudoers" ]; then
      echo "managed sudoers policy is not a regular file" >&2
      return 1
    fi
    cp -p -- "$chariox_sudoers" "$staging_root/access-policy-sudoers" || return 1
    sudoers_was_present=1
  fi
  printf '%s\n' "$previous_login_shell" > "$staging_root/access-policy-login-shell" || return 1
  access_policy_pending=1
}
activate_access_policy() {
  if [ "$managed_provider_topology" = path1 ]; then
    install -d -o root -g root -m 0750 "$install_root/etc/sudoers.d" || return 1
    chariox_sudoers_tmp=$(mktemp "$install_root/etc/sudoers.d/.chariox.XXXXXX") || return 1
    printf '%s\n' 'chariox ALL=(ALL) NOPASSWD: ALL' >"$chariox_sudoers_tmp" || return 1
    chmod 0440 "$chariox_sudoers_tmp" || return 1
    if ! visudo -cqf "$chariox_sudoers_tmp"; then
      rm -f -- "$chariox_sudoers_tmp"
      return 1
    fi
    mv -f -- "$chariox_sudoers_tmp" "$chariox_sudoers" || return 1
  else
    rm -f -- "$chariox_sudoers" || return 1
  fi
  [ "$previous_login_shell" = "$chariox_login_shell" ] || usermod --shell "$chariox_login_shell" chariox
}
restore_access_policy() {
  restore_failed=0
  if [ "$sudoers_was_present" = 1 ]; then
    # Copy only the recorded policy into a new inert file, then publish atomically.
    restore_tmp=$(mktemp "$install_root/etc/sudoers.d/.chariox.XXXXXX") || return 1
    cp -p -- "$staging_root/access-policy-sudoers" "$restore_tmp" && mv -f -- "$restore_tmp" "$chariox_sudoers" || restore_failed=1
    rm -f -- "$restore_tmp"
  else
    rm -f -- "$chariox_sudoers" || restore_failed=1
  fi
  current_login_shell=$(getent passwd chariox | cut -d: -f7)
  if [ "$current_login_shell" != "$previous_login_shell" ]; then
    usermod --shell "$previous_login_shell" chariox || restore_failed=1
  fi
  [ "$restore_failed" = 0 ]
}
